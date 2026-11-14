//! Port of `TTSModel` from `pocket_tts/models/tts_model.py`, on the streaming primitives of
//! xn-ptts: loading a config's weights, conditioning on a voice, and generating audio chunk by
//! chunk while a second thread decodes it.

// Adapted from Kyutai Pocket TTS 9e8413e; see THIRD_PARTY.md and LICENSE-Pocket-TTS.

use crate::config::{Config, FlowType};
use crate::defaults;
use crate::rng::NoiseRng;
use crate::text::{TextOptions, prepare_text_prompt, split_into_best_sentences};
use crate::tokenizer::{SharedTokenizer, Tokenizer};
use crate::weights::{SafetensorsFile, WeightFiles};
use anyhow::{Result, bail};
use ptts::flow_lm::{FlowLMState, StepInput};
use ptts::transformer::{LayerAttentionState, StreamingMHAState, StreamingTransformerState};
use ptts::tts_model::{TTSModel, TTSState};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::TryRecvError;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use xn::{BackendQ, Tensor, WithDTypeF};

/// The backends this binary runs on: the CPU, with f32 or int8 weights.
pub trait CpuBackend: BackendQ<B = xn::CpuDevice> {}
impl<Q: BackendQ<B = xn::CpuDevice>> CpuBackend for Q {}

const TOKENS_PER_SECOND_ESTIMATE: f64 = 3.0;
const GEN_SECONDS_PADDING: f64 = 2.0;
/// EOS is ignored on the first frames: before speech starts, the EOS logit of some voices can
/// cross the threshold, and a short text then ends before the word is spoken.
const MIN_FRAMES_BEFORE_EOS: usize = 6;

/// What `load_model` takes besides the config.
#[derive(Debug, Clone)]
pub struct LoadOptions {
    pub temperature: Option<f32>,
    pub sampler_decode_steps: usize,
    pub noise_clamp: Option<f32>,
    /// The config's `default_eos_threshold` when `None`.
    pub eos_threshold: Option<f32>,
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self {
            temperature: None,
            sampler_decode_steps: defaults::DEFAULT_SAMPLER_DECODE_STEPS,
            noise_clamp: None,
            eos_threshold: None,
        }
    }
}

/// The flow LM state after a voice prompt, which every generation starts from.
pub struct VoiceState<Q: CpuBackend> {
    transformer_state: StreamingTransformerState<Q::T, Q::B>,
}

impl<Q: CpuBackend> VoiceState<Q> {
    /// Positions the prompt fills.
    pub fn len(&self) -> usize {
        match self.transformer_state.layer_states.first() {
            Some(LayerAttentionState::FlowLm(state)) => state.current_end,
            _ => 0,
        }
    }
}

pub struct TtsModel<Q: CpuBackend> {
    pub config: Config,
    model: TTSModel<Q>,
    tokenizer: Arc<Tokenizer>,
    text_options: TextOptions,
    rng: Mutex<NoiseRng>,
    num_layers: usize,
    num_kv_heads: usize,
    head_dim: usize,
}

impl<Q: CpuBackend> TtsModel<Q> {
    /// Port of `TTSModel.load_model`, for the config already read.
    pub fn load(config: Config, options: &LoadOptions) -> Result<Self> {
        let mut weight_files = WeightFiles::default();
        if let Some(mimi_path) = config.mimi.weights_path.as_deref() {
            if config.flow_lm.weights_path.is_none() && config.weights_path.is_none() {
                bail!(
                    "If you specify mimi.weights_path you should specify flow_lm.weights_path or weights_path"
                )
            }
            log::info!("Loading Mimi weights from {mimi_path}");
            weight_files.mimi = Some(crate::download::download_if_necessary(mimi_path)?);
        }
        if let Some(weights_path) = config.weights_path.as_deref() {
            log::info!("Loading TTSModel weights from {weights_path}");
            let path = match crate::download::download_if_necessary(weights_path) {
                Ok(path) => path,
                Err(e) => {
                    let Some(fallback) = config.weights_path_without_voice_cloning.as_deref()
                    else {
                        return Err(e);
                    };
                    log::debug!("falling back to the weights without voice cloning: {e:#}");
                    crate::download::download_if_necessary(fallback)?
                }
            };
            weight_files.bundle = Some(path);
        } else if let Some(flow_lm_path) = config.flow_lm.weights_path.as_deref() {
            if weight_files.mimi.is_none() {
                bail!("flow_lm.weights_path needs mimi.weights_path, for the Mimi weights")
            }
            log::info!("Loading FlowLM weights from {flow_lm_path}");
            weight_files.flow_lm = Some(crate::download::download_if_necessary(flow_lm_path)?);
        } else {
            bail!("No weights_path specified for the TTSModel, the model would be uninitialized")
        }

        let lookup_table = &config.flow_lm.lookup_table;
        let tokenizer = Arc::new(Tokenizer::load(
            lookup_table.n_bins,
            &lookup_table.tokenizer_path,
            &lookup_table.tokenizer,
        )?);

        // The drifting and pointwise heads map the noise in one step, whatever the number of
        // steps asked.
        let decode_steps = match config.flow_type() {
            FlowType::Drifting | FlowType::Pointwise => 1,
            FlowType::Lsd | FlowType::FlowMatching => options.sampler_decode_steps,
        };
        if decode_steps == 0 {
            bail!("the number of sampler decode steps must be positive")
        }
        let eos_threshold = options
            .eos_threshold
            .unwrap_or(config.default_eos_threshold);
        let ptts_config = config.to_ptts(decode_steps, eos_threshold)?;
        let weights = crate::weights::load(&weight_files)?;
        let model = TTSModel::<Q>::load(
            &weights.vb,
            Box::new(SharedTokenizer(tokenizer.clone())),
            &ptts_config,
            &HashMap::new(),
        )?;
        drop(weights);
        let temperature = options.temperature.unwrap_or(config.default_temperature);
        let transformer = &config.flow_lm.transformer;
        Ok(Self {
            num_layers: transformer.num_layers,
            num_kv_heads: transformer.num_kv_heads(),
            head_dim: transformer.head_dim(),
            text_options: TextOptions::from_config(&config),
            rng: Mutex::new(NoiseRng::new(temperature, options.noise_clamp)?),
            config,
            model,
            tokenizer,
        })
    }

    pub fn sample_rate(&self) -> usize {
        self.config.mimi.sample_rate
    }

    /// Reads a state written by `export-voice`, from this binary or the Python package.
    pub fn import_state(&self, path: &Path) -> Result<VoiceState<Q>> {
        let file = SafetensorsFile::open(path)?;
        let dim_per_head = self.head_dim;
        let mut layer_states = Vec::with_capacity(self.num_layers);
        for i in 0..self.num_layers {
            let module = format!("transformer.layers.{i}.self_attn");
            let cache_name = format!("{module}/cache");
            if !file.tensors.contains_key(&cache_name) {
                bail!(
                    "{} is not a voice state of this model: it has no {cache_name}",
                    path.display()
                )
            }
            let (cache, shape) = file.read_f32(&cache_name)?;
            let [2, b, capacity, h, d] = shape[..] else {
                bail!("{cache_name} has shape {shape:?}, expected [2, batch, time, heads, dim]")
            };
            if b != 1 || h != self.num_kv_heads || d != dim_per_head {
                bail!(
                    "{cache_name} has shape {shape:?}, this model needs [2, 1, _, {}, {dim_per_head}]",
                    self.num_kv_heads
                )
            }
            // States written before the offset was stored hold the position in the length of
            // `current_end`.
            let offset_name = format!("{module}/offset");
            let current_end_name = format!("{module}/current_end");
            let used = if file.tensors.contains_key(&offset_name) {
                file.read_i64(&offset_name)?.first().copied().unwrap_or(0) as usize
            } else if let Some(info) = file.tensors.get(&current_end_name) {
                info.shape.first().copied().unwrap_or(0)
            } else {
                capacity
            };
            if used > capacity {
                bail!("{offset_name} is {used}, past the {capacity} cached positions")
            }
            let per_position = h * d;
            let plane = capacity * per_position;
            let take = |start: usize| -> Result<Tensor<Q::T, Q::B>> {
                let data = cache[start..start + used * per_position]
                    .iter()
                    .map(|&v| Q::T::from_f32(v))
                    .collect();
                Ok(Tensor::from_vec(
                    data,
                    (1, used, h, d),
                    self.model.device(),
                )?)
            };
            layer_states.push(LayerAttentionState::FlowLm(StreamingMHAState {
                k_cache: take(0)?,
                v_cache: take(plane)?,
                current_end: used,
            }));
        }
        Ok(VoiceState {
            transformer_state: StreamingTransformerState { layer_states },
        })
    }

    fn estimate_max_gen_len(&self, token_count: usize) -> usize {
        let gen_len_sec = token_count as f64 / TOKENS_PER_SECOND_ESTIMATE + GEN_SECONDS_PADDING;
        (gen_len_sec * self.config.mimi.frame_rate).ceil() as usize
    }

    /// Port of `generate_audio_stream`: splits the text into chunks and generates them one after
    /// the other from the voice state, calling `on_chunk` with the audio as it is decoded.
    /// Setting `stop` ends the generation early.
    pub fn generate_audio_stream(
        &self,
        voice: &VoiceState<Q>,
        text_to_generate: &str,
        max_tokens: usize,
        frames_after_eos: Option<usize>,
        stop: &AtomicBool,
        on_chunk: &mut (dyn FnMut(&[f32]) -> Result<()> + Send),
    ) -> Result<()> {
        let frames_after_eos = frames_after_eos.or(self.config.model_recommended_frames_after_eos);
        // This is a very simplistic way of handling long texts, as in the Python package.
        let chunks = split_into_best_sentences(
            self.tokenizer.as_ref(),
            text_to_generate,
            max_tokens,
            &self.text_options,
        )?;
        for chunk in chunks {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let (text, frames_after_eos_guess) = prepare_text_prompt(&chunk, &self.text_options)?;
            let frames = frames_after_eos.unwrap_or(frames_after_eos_guess + 2);
            self.generate_short_text(voice, &text, frames, stop, on_chunk)?;
        }
        Ok(())
    }

    fn generate_short_text(
        &self,
        voice: &VoiceState<Q>,
        text: &str,
        frames_after_eos: usize,
        stop: &AtomicBool,
        on_chunk: &mut (dyn FnMut(&[f32]) -> Result<()> + Send),
    ) -> Result<()> {
        let tokens = self.tokenizer.encode(text)?;
        let max_gen_len = self.estimate_max_gen_len(tokens.len());
        let seq_budget = voice.len() + tokens.len() + max_gen_len;
        let transformer_state = voice.transformer_state.with_seq_budget(seq_budget)?;
        let mut state = TTSState {
            flow_lm_state: FlowLMState {
                transformer_state,
                conditions: None,
            },
        };

        let t_generating = Instant::now();
        let start = Instant::now();
        self.model.prompt_text(&mut state, &tokens)?;
        log::info!("Prompting text took {} ms", start.elapsed().as_millis());

        let sample_rate = self.sample_rate();
        let (latent_tx, latent_rx) = std::sync::mpsc::sync_channel::<Option<Tensor<Q::T, Q::B>>>(2);
        let (generated, decoded) = std::thread::scope(|s| {
            let decoder = s.spawn(move || self.decode_worker(latent_rx, on_chunk));
            let generated = self.autoregressive_generation(
                &mut state,
                max_gen_len,
                frames_after_eos,
                &latent_tx,
                stop,
            );
            // Closes the stream, also on error, so that the decoder finishes.
            let _ = latent_tx.send(None);
            let decoded = decoder
                .join()
                .unwrap_or_else(|_| Err(anyhow::anyhow!("the decoder panicked")));
            (generated, decoded)
        });
        generated?;
        let total_samples = decoded?;

        let audio_ms = total_samples * 1000 / sample_rate;
        let generation_ms = t_generating.elapsed().as_millis().max(1) as usize;
        log::info!(
            "Generated: {audio_ms} ms of audio in {generation_ms} ms so {:.2}x faster than real-time",
            audio_ms as f64 / generation_ms as f64
        );
        Ok(())
    }

    fn autoregressive_generation(
        &self,
        state: &mut TTSState<Q>,
        max_gen_len: usize,
        frames_after_eos: usize,
        latent_tx: &std::sync::mpsc::SyncSender<Option<Tensor<Q::T, Q::B>>>,
        stop: &AtomicBool,
    ) -> Result<()> {
        let mut previous: Option<Tensor<Q::T, Q::B>> = None;
        let mut eos_step = None;
        let mut step_times = Vec::new();
        let mut reached_max_len = true;
        for generation_step in 0..max_gen_len {
            if stop.load(Ordering::Relaxed) {
                reached_max_len = false;
                break;
            }
            let start = Instant::now();
            let input = match &previous {
                None => StepInput::Bos { batch: 1 },
                Some(latent) => StepInput::Latent(latent),
            };
            let (latent, is_eos) = {
                let mut rng = self.rng.lock().unwrap_or_else(|e| e.into_inner());
                self.model.generate_step(state, input, &mut *rng)?
            };
            if is_eos && eos_step.is_none() && generation_step >= MIN_FRAMES_BEFORE_EOS {
                eos_step = Some(generation_step);
            }
            if eos_step.is_some_and(|eos_step| generation_step >= eos_step + frames_after_eos) {
                reached_max_len = false;
                break;
            }
            // A closed channel means the decoder stopped on an error, which it reports.
            if latent_tx.send(Some(latent.clone())).is_err() {
                reached_max_len = false;
                break;
            }
            previous = Some(latent);
            step_times.push(start.elapsed().as_millis());
        }
        if reached_max_len {
            if std::env::var("KPOCKET_TTS_ERROR_WITHOUT_EOS").is_ok_and(|v| v == "1") {
                bail!("Generation reached maximum length without EOS!")
            }
            log::warn!(
                "Maximum generation length reached without EOS, this very often indicates an error."
            );
        }
        if !step_times.is_empty() {
            let mean = step_times.iter().sum::<u128>() / step_times.len() as u128;
            log::info!("Average generation step time: {mean} ms");
        }
        Ok(())
    }

    /// Decodes the latents as they come, as many as are queued in one call, and hands the audio
    /// to `on_chunk`. Returns the number of samples decoded.
    fn decode_worker(
        &self,
        latent_rx: std::sync::mpsc::Receiver<Option<Tensor<Q::T, Q::B>>>,
        on_chunk: &mut (dyn FnMut(&[f32]) -> Result<()> + Send),
    ) -> Result<usize> {
        let mut mimi_state = self.model.init_mimi_state(1)?;
        // A fresh Mimi decoder state puts a small step in its first samples, heard as a click at
        // the start of every chunk: fade the first 5 ms in.
        let fade_len = self.sample_rate() / 200;
        let mut fade_in = true;
        let mut total = 0;
        while let Ok(Some(latent)) = latent_rx.recv() {
            let mut latents = vec![latent];
            let mut finished = false;
            loop {
                match latent_rx.try_recv() {
                    Ok(Some(latent)) => latents.push(latent),
                    Ok(None) | Err(TryRecvError::Disconnected) => {
                        finished = true;
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                }
            }
            let refs: Vec<&Tensor<Q::T, Q::B>> = latents.iter().collect();
            let batch = Tensor::cat(&refs, 1)?;
            let audio = self.model.decode_latent(&batch, &mut mimi_state)?;
            let mut pcm = audio.to_vec()?;
            if fade_in {
                let n = fade_len.min(pcm.len());
                for (i, sample) in pcm[..n].iter_mut().enumerate() {
                    *sample *= if fade_len > 1 {
                        i as f32 / (fade_len - 1) as f32
                    } else {
                        0.0
                    };
                }
                fade_in = false;
            }
            total += pcm.len();
            on_chunk(&pcm)?;
            if finished {
                break;
            }
        }
        Ok(total)
    }
}
