//! Greedy, cancellable English-only Whisper transcription using hybrid F32/Q8_0 inference.
//! No word times are fabricated: this decoder returns text, not forced alignment.
use anyhow::{Result, bail};
use candle::{
    DType, Device, IndexOp, Module, Tensor,
    quantized::{GgmlDType, QMatMul, QTensor},
};
use candle_nn::{
    Conv1d, Conv1dConfig, LayerNorm, Linear,
    ops::{softmax, softmax_last_dim},
};
use candle_transformers::models::whisper as m;
use half::{bf16, f16};
use rayon::prelude::*;
use serde::Deserialize;
use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};
use tokenizers::Tokenizer;

const N_FFT: usize = m::N_FFT;
const N_FREQ: usize = N_FFT / 2 + 1;
const HOP_LENGTH: usize = m::HOP_LENGTH;
const N_MELS: usize = 80;
const MAX_HEADER_BYTES: usize = 16 * 1024 * 1024;
const SILENCE_LOG10: f32 = -10.0;

pub struct Recognizer {
    model: HybridWhisper,
    tokenizer: Tokenizer,
    mel: MelExtractor,
    suppress: Tensor,
}

impl Recognizer {
    /// Load English-only `whisper-medium.en` weights directly via streaming reads so peak load
    /// memory stays bounded and file locks are released before inference begins.
    pub fn load(root: &Path) -> Result<Self> {
        let root = root.join("models/whisper-medium-en-native");
        let config = load_config(&root)?;
        validate_config(&config)?;
        let tokenizer = Tokenizer::from_file(root.join("tokenizer.json"))
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let mel = MelExtractor::new(mel_filters(config.num_mel_bins)?);
        let mut suppress = vec![0.0_f32; config.vocab_size];
        for &token in &config.suppress_tokens {
            if let Some(slot) = suppress.get_mut(token as usize) {
                *slot = f32::NEG_INFINITY;
            }
        }
        let mut reader = SafetensorsReader::open(&root.join("model.safetensors"))?;
        let model = HybridWhisper::load(&mut reader, &config)?;
        drop(reader);
        Ok(Self {
            model,
            tokenizer,
            mel,
            suppress: Tensor::new(suppress.as_slice(), &Device::Cpu)?,
        })
    }

    fn token(&self, text: &str) -> Result<u32> {
        self.tokenizer
            .token_to_id(text)
            .ok_or_else(|| anyhow::anyhow!("Whisper token missing: {text}"))
    }

    pub fn transcribe(
        &mut self,
        pcm: &[f32],
        _language: &str,
        cancel: &AtomicBool,
    ) -> Result<String> {
        self.model.reset_kv_cache();
        let result = self.transcribe_inner(pcm, cancel);
        self.model.reset_kv_cache();
        result
    }

    fn transcribe_inner(&mut self, pcm: &[f32], cancel: &AtomicBool) -> Result<String> {
        if !(1600..=480_000).contains(&pcm.len()) {
            bail!("Speech must contain 0.1–30 seconds of mono 16 kHz PCM");
        }
        if cancel.load(Ordering::Acquire) {
            return Ok(String::new());
        }
        // Reject digital silence before the expensive encoder; native AEC/VAD handles real mic audio.
        if pcm.iter().map(|s| s * s).sum::<f32>() / (pcm.len() as f32) < 1e-8 {
            return Ok(String::new());
        }
        let (mel_data, frames) = self.mel.extract(pcm);
        let mel = Tensor::from_vec(mel_data, (1, N_MELS, frames), &Device::Cpu)?;
        let features = self.model.encoder.forward(&mel)?;
        let eot = self.token(m::EOT_TOKEN)?;
        let no_speech = m::NO_SPEECH_TOKENS
            .iter()
            .find_map(|s| self.tokenizer.token_to_id(s))
            .ok_or_else(|| anyhow::anyhow!("Missing silence token"))?;
        let mut tokens = vec![
            self.token(m::SOT_TOKEN)?,
            self.token(m::NO_TIMESTAMPS_TOKEN)?,
        ];
        let mut probability = 0.0_f64;
        let mut count = 0;
        for step in 0..224 {
            if cancel.load(Ordering::Acquire) {
                return Ok(String::new());
            }
            let (slice, pos_offset, flush_cache) = if step == 0 {
                (&tokens[..], 0, true)
            } else {
                (&tokens[tokens.len() - 1..], tokens.len() - 1, false)
            };
            let input = Tensor::new(slice, &Device::Cpu)?.unsqueeze(0)?;
            let decoded =
                self.model
                    .decoder
                    .forward(&input, &features, pos_offset, flush_cache)?;
            if step == 0 {
                let first = decoded.i((..1, ..1))?.contiguous()?;
                let logits = self.model.decoder.final_linear(&first)?.i((0, 0))?;
                if softmax(&logits, 0)?
                    .i(no_speech as usize)?
                    .to_scalar::<f32>()?
                    > 0.6
                {
                    return Ok(String::new());
                }
            }
            let (_, length, _) = decoded.dims3()?;
            let tail = if length == 1 {
                decoded
            } else {
                decoded.i((..1, length - 1..))?.contiguous()?
            };
            let logits = self
                .model
                .decoder
                .final_linear(&tail)?
                .i((0, 0))?
                .broadcast_add(&self.suppress)?;
            let values = logits.to_vec1::<f32>()?;
            let next = values
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.total_cmp(b))
                .map(|(i, _)| i as u32)
                .unwrap();
            if next == eot {
                break;
            }
            probability += (softmax(&logits, 0)?.i(next as usize)?.to_scalar::<f32>()? as f64).ln();
            count += 1;
            tokens.push(next);
        }
        if count == 0 || probability / (count as f64) < -1.0 {
            return Ok(String::new());
        }
        Ok(self
            .tokenizer
            .decode(&tokens[2..], true)
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .trim()
            .into())
    }
}

struct HybridWhisper {
    encoder: AudioEncoder,
    decoder: TextDecoder,
}

impl HybridWhisper {
    fn load(reader: &mut SafetensorsReader, config: &m::Config) -> Result<Self> {
        let encoder = AudioEncoder::load(reader, config)?;
        let decoder = TextDecoder::load(reader, config)?;
        Ok(Self { encoder, decoder })
    }

    fn reset_kv_cache(&mut self) {
        self.decoder.reset_kv_cache();
    }
}

struct AudioEncoder {
    conv1: Conv1d,
    conv2: Conv1d,
    positional_embedding: Tensor,
    blocks: Vec<ResidualBlock>,
    ln_post: LayerNorm,
}

impl AudioEncoder {
    fn load(reader: &mut SafetensorsReader, cfg: &m::Config) -> Result<Self> {
        let n_state = cfg.d_model;
        let n_head = cfg.encoder_attention_heads;
        let n_ctx = cfg.max_source_positions;
        let cfg1 = Conv1dConfig {
            padding: 1,
            stride: 1,
            ..Default::default()
        };
        let cfg2 = Conv1dConfig {
            padding: 1,
            stride: 2,
            ..Default::default()
        };
        let conv1 = reader.conv1d("model.encoder.conv1", n_state, cfg.num_mel_bins, 3, cfg1)?;
        let conv2 = reader.conv1d("model.encoder.conv2", n_state, n_state, 3, cfg2)?;
        let positional_embedding =
            reader.tensor("model.encoder.embed_positions.weight", &[n_ctx, n_state])?;
        let blocks = (0..cfg.encoder_layers)
            .map(|i| {
                ResidualBlock::load_encoder(
                    reader,
                    &format!("model.encoder.layers.{i}"),
                    n_state,
                    n_head,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let ln_post = reader.layer_norm("model.encoder.layer_norm", n_state)?;
        Ok(Self {
            conv1,
            conv2,
            positional_embedding,
            blocks,
            ln_post,
        })
    }

    fn forward(&mut self, x: &Tensor) -> Result<Tensor> {
        let x = self.conv1.forward(x)?.gelu()?;
        let x = self.conv2.forward(&x)?.gelu()?.transpose(1, 2)?;
        let (_, seq_len, _) = x.dims3()?;
        let pos = self.positional_embedding.narrow(0, 0, seq_len)?;
        let mut x = x.broadcast_add(&pos)?;
        for block in &mut self.blocks {
            x = block.forward(&x, None, None, false)?;
        }
        Ok(self.ln_post.forward(&x)?)
    }
}

struct TextDecoder {
    embed_tokens_f16: Tensor,
    logits_proj: QMatMul,
    positional_embedding: Tensor,
    blocks: Vec<ResidualBlock>,
    ln: LayerNorm,
    mask: Tensor,
    d_model: usize,
}

impl TextDecoder {
    fn load(reader: &mut SafetensorsReader, cfg: &m::Config) -> Result<Self> {
        let n_state = cfg.d_model;
        let n_head = cfg.decoder_attention_heads;
        let n_ctx = cfg.max_target_positions;
        let positional_embedding =
            reader.tensor("model.decoder.embed_positions.weight", &[n_ctx, n_state])?;
        let blocks = (0..cfg.decoder_layers)
            .map(|i| {
                ResidualBlock::load_decoder(
                    reader,
                    &format!("model.decoder.layers.{i}"),
                    n_state,
                    n_head,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let ln = reader.layer_norm("model.decoder.layer_norm", n_state)?;
        // Load token embeddings last so the temporary F32 matrix for Q8_0 quantization stays
        // isolated from layer weight construction.
        let embed_shape = [cfg.vocab_size, n_state];
        let embed_tokens_f16 = reader.tensor_f16("model.decoder.embed_tokens.weight", &embed_shape)?;
        let logits_proj = reader.qmatmul("model.decoder.embed_tokens.weight", &embed_shape)?;
        let mask: Vec<f32> = (0..n_ctx)
            .flat_map(|i| (0..n_ctx).map(move |j| if j > i { f32::NEG_INFINITY } else { 0.0 }))
            .collect();
        let mask = Tensor::from_vec(mask, (n_ctx, n_ctx), &Device::Cpu)?;
        Ok(Self {
            embed_tokens_f16,
            logits_proj,
            positional_embedding,
            blocks,
            ln,
            mask,
            d_model: n_state,
        })
    }

    fn forward(
        &mut self,
        tokens: &Tensor,
        xa: &Tensor,
        pos_offset: usize,
        flush_cache: bool,
    ) -> Result<Tensor> {
        let seq_len = tokens.dim(1)?;
        let flat = tokens.flatten_all()?;
        let emb = self
            .embed_tokens_f16
            .index_select(&flat, 0)?
            .to_dtype(DType::F32)?
            .reshape((1, seq_len, self.d_model))?;
        let pos = self.positional_embedding.narrow(0, pos_offset, seq_len)?;
        let mut x = emb.broadcast_add(&pos)?;
        let mask = if seq_len > 1 {
            Some(self.mask.i((0..seq_len, 0..seq_len))?)
        } else {
            None
        };
        for block in &mut self.blocks {
            x = block.forward(&x, Some(xa), mask.as_ref(), flush_cache)?;
        }
        Ok(self.ln.forward(&x)?)
    }

    fn final_linear(&self, x: &Tensor) -> Result<Tensor> {
        if x.is_contiguous() {
            Ok(self.logits_proj.forward(x)?)
        } else {
            Ok(self.logits_proj.forward(&x.contiguous()?)?)
        }
    }

    fn reset_kv_cache(&mut self) {
        for block in &mut self.blocks {
            block.reset_kv_cache();
        }
    }
}

struct ResidualBlock {
    attn: Attention,
    attn_ln: LayerNorm,
    cross_attn: Option<(Attention, LayerNorm)>,
    fc1: Projection,
    fc2: Projection,
    mlp_ln: LayerNorm,
}

impl ResidualBlock {
    fn load_encoder(
        reader: &mut SafetensorsReader,
        prefix: &str,
        n_state: usize,
        n_head: usize,
    ) -> Result<Self> {
        let attn = Attention::load(
            reader,
            &format!("{prefix}.self_attn"),
            n_state,
            n_head,
            AttentionMode::Encoder,
            false,
            false,
        )?;
        let attn_ln = reader.layer_norm(&format!("{prefix}.self_attn_layer_norm"), n_state)?;
        let fc1 = Projection::Dense(reader.linear(&format!("{prefix}.fc1"), n_state * 4, n_state, true)?);
        let fc2 = Projection::Dense(reader.linear(&format!("{prefix}.fc2"), n_state, n_state * 4, true)?);
        let mlp_ln = reader.layer_norm(&format!("{prefix}.final_layer_norm"), n_state)?;
        Ok(Self {
            attn,
            attn_ln,
            cross_attn: None,
            fc1,
            fc2,
            mlp_ln,
        })
    }

    fn load_decoder(
        reader: &mut SafetensorsReader,
        prefix: &str,
        n_state: usize,
        n_head: usize,
    ) -> Result<Self> {
        let attn = Attention::load(
            reader,
            &format!("{prefix}.self_attn"),
            n_state,
            n_head,
            AttentionMode::DecoderSelf { kv_cache: None },
            true,
            true,
        )?;
        let attn_ln = reader.layer_norm(&format!("{prefix}.self_attn_layer_norm"), n_state)?;
        // Cross-attention K/V projections process all encoder frames once per utterance, so keeping
        // them in F32 lets gemm execute multi-frame matmuls efficiently while Q/Out run in Q8_0.
        let cross = Attention::load(
            reader,
            &format!("{prefix}.encoder_attn"),
            n_state,
            n_head,
            AttentionMode::DecoderCross { kv_cache: None },
            true,
            false,
        )?;
        let cross_ln = reader.layer_norm(&format!("{prefix}.encoder_attn_layer_norm"), n_state)?;
        let fc1 =
            Projection::Quantized(reader.qlinear(&format!("{prefix}.fc1"), n_state * 4, n_state, true)?);
        let fc2 =
            Projection::Quantized(reader.qlinear(&format!("{prefix}.fc2"), n_state, n_state * 4, true)?);
        let mlp_ln = reader.layer_norm(&format!("{prefix}.final_layer_norm"), n_state)?;
        Ok(Self {
            attn,
            attn_ln,
            cross_attn: Some((cross, cross_ln)),
            fc1,
            fc2,
            mlp_ln,
        })
    }

    fn forward(
        &mut self,
        x: &Tensor,
        xa: Option<&Tensor>,
        mask: Option<&Tensor>,
        flush_cache: bool,
    ) -> Result<Tensor> {
        let norm = self.attn_ln.forward(x)?;
        let attn = self.attn.forward(&norm, None, mask, flush_cache)?;
        let mut x = (x + attn)?;
        if let Some((cross, ln)) = &mut self.cross_attn {
            let norm = ln.forward(&x)?;
            let cross_out = cross.forward(&norm, xa, None, flush_cache)?;
            x = (&x + cross_out)?;
        }
        let hidden = self.fc1.forward(&self.mlp_ln.forward(&x)?)?.gelu()?;
        let mlp = self.fc2.forward(&hidden)?;
        Ok((&x + mlp)?)
    }

    fn reset_kv_cache(&mut self) {
        self.attn.reset_kv_cache();
        if let Some((cross, _)) = &mut self.cross_attn {
            cross.reset_kv_cache();
        }
    }
}

enum AttentionMode {
    Encoder,
    DecoderSelf { kv_cache: Option<(Tensor, Tensor)> },
    DecoderCross { kv_cache: Option<(Tensor, Tensor)> },
}

struct Attention {
    q_proj: Projection,
    k_proj: Projection,
    v_proj: Projection,
    out_proj: Projection,
    n_head: usize,
    scale: f64,
    mode: AttentionMode,
}

impl Attention {
    fn load(
        reader: &mut SafetensorsReader,
        prefix: &str,
        n_state: usize,
        n_head: usize,
        mode: AttentionMode,
        quantize_q_out: bool,
        quantize_kv: bool,
    ) -> Result<Self> {
        let load_proj =
            |reader: &mut SafetensorsReader, suffix: &str, bias: bool, quant: bool| -> Result<Projection> {
                let name = format!("{prefix}.{suffix}");
                if quant {
                    Ok(Projection::Quantized(
                        reader.qlinear(&name, n_state, n_state, bias)?,
                    ))
                } else {
                    Ok(Projection::Dense(
                        reader.linear(&name, n_state, n_state, bias)?,
                    ))
                }
            };
        let q_proj = load_proj(reader, "q_proj", true, quantize_q_out)?;
        let k_proj = load_proj(reader, "k_proj", false, quantize_kv)?;
        let v_proj = load_proj(reader, "v_proj", true, quantize_kv)?;
        let out_proj = load_proj(reader, "out_proj", true, quantize_q_out)?;
        Ok(Self {
            q_proj,
            k_proj,
            v_proj,
            out_proj,
            n_head,
            scale: ((n_state / n_head) as f64).powf(-0.25),
            mode,
        })
    }

    fn reshape_head(x: &Tensor, n_head: usize) -> Result<Tensor> {
        let (n_batch, n_ctx, n_state) = x.dims3()?;
        Ok(x.reshape(&[n_batch, n_ctx, n_head, n_state / n_head])?
            .transpose(1, 2)?)
    }

    fn forward(
        &mut self,
        x: &Tensor,
        xa: Option<&Tensor>,
        mask: Option<&Tensor>,
        flush_cache: bool,
    ) -> Result<Tensor> {
        let n_head = self.n_head;
        let scale = self.scale;
        let q = (Self::reshape_head(&self.q_proj.forward(x)?, n_head)? * scale)?;
        let (k_t, v) = match &mut self.mode {
            AttentionMode::Encoder => {
                let k = (Self::reshape_head(&self.k_proj.forward(x)?, n_head)? * scale)?;
                let v = Self::reshape_head(&self.v_proj.forward(x)?, n_head)?.contiguous()?;
                (k.transpose(2, 3)?, v)
            }
            AttentionMode::DecoderSelf { kv_cache } => {
                if flush_cache {
                    *kv_cache = None;
                }
                let k_new = (Self::reshape_head(&self.k_proj.forward(x)?, n_head)? * scale)?;
                let v_new = Self::reshape_head(&self.v_proj.forward(x)?, n_head)?.contiguous()?;
                let (k, v) = if let Some((k_prev, v_prev)) = kv_cache.as_ref() {
                    (
                        Tensor::cat(&[k_prev, &k_new], 2)?,
                        Tensor::cat(&[v_prev, &v_new], 2)?,
                    )
                } else {
                    (k_new.contiguous()?, v_new)
                };
                *kv_cache = Some((k.clone(), v.clone()));
                (k.transpose(2, 3)?, v)
            }
            AttentionMode::DecoderCross { kv_cache } => {
                if flush_cache {
                    *kv_cache = None;
                }
                if let Some((k_t, v)) = kv_cache.as_ref() {
                    (k_t.clone(), v.clone())
                } else {
                    let xa = xa.ok_or_else(|| {
                        anyhow::anyhow!("Missing encoder hidden states for cross-attention")
                    })?;
                    let k_t = (Self::reshape_head(&self.k_proj.forward(xa)?, n_head)?
                        .transpose(2, 3)?
                        .contiguous()?
                        * scale)?;
                    let v = Self::reshape_head(&self.v_proj.forward(xa)?, n_head)?.contiguous()?;
                    *kv_cache = Some((k_t.clone(), v.clone()));
                    (k_t, v)
                }
            }
        };
        let mut qk = q.matmul(&k_t)?;
        if let Some(mask) = mask {
            qk = qk.broadcast_add(mask)?;
        }
        let weights = softmax_last_dim(&qk)?;
        let wv = weights
            .matmul(&v)?
            .transpose(1, 2)?
            .flatten_from(2)?
            .contiguous()?;
        self.out_proj.forward(&wv)
    }

    fn reset_kv_cache(&mut self) {
        match &mut self.mode {
            AttentionMode::DecoderSelf { kv_cache }
            | AttentionMode::DecoderCross { kv_cache } => *kv_cache = None,
            AttentionMode::Encoder => {}
        }
    }
}

struct QLinear {
    weight: QMatMul,
    bias: Option<Tensor>,
}

impl QLinear {
    fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let x = if x.is_contiguous() {
            self.weight.forward(x)?
        } else {
            self.weight.forward(&x.contiguous()?)?
        };
        match &self.bias {
            None => Ok(x),
            Some(bias) => Ok(x.broadcast_add(bias)?),
        }
    }
}

enum Projection {
    Dense(Linear),
    Quantized(QLinear),
}

impl Projection {
    fn forward(&self, x: &Tensor) -> Result<Tensor> {
        match self {
            Self::Dense(linear) => Ok(linear.forward(x)?),
            Self::Quantized(linear) => linear.forward(x),
        }
    }
}

struct TensorEntry {
    dtype: String,
    shape: Vec<usize>,
    start: u64,
    end: u64,
}

#[derive(Deserialize)]
struct RawTensorEntry {
    dtype: String,
    shape: Vec<usize>,
    data_offsets: [u64; 2],
}

struct SafetensorsReader {
    file: File,
    data_offset: u64,
    tensors: HashMap<String, TensorEntry>,
    buf: Vec<u8>,
}

impl SafetensorsReader {
    fn open(path: &Path) -> Result<Self> {
        let mut file = File::open(path)?;
        let file_len = file.metadata()?.len();
        let mut header_len_bytes = [0_u8; 8];
        file.read_exact(&mut header_len_bytes)?;
        let header_len = u64::from_le_bytes(header_len_bytes);
        if header_len > MAX_HEADER_BYTES as u64 || 8 + header_len > file_len {
            bail!("Invalid safetensors header length in {}", path.display());
        }
        let mut header_bytes = vec![0_u8; header_len as usize];
        file.read_exact(&mut header_bytes)?;
        let raw_header: HashMap<String, serde_json::Value> = serde_json::from_slice(&header_bytes)?;
        let data_offset = 8 + header_len;
        let mut tensors = HashMap::with_capacity(raw_header.len());
        for (name, value) in raw_header {
            if name == "__metadata__" {
                continue;
            }
            let entry: RawTensorEntry = serde_json::from_value(value)?;
            let [start, end] = entry.data_offsets;
            if start > end || data_offset.saturating_add(end) > file_len {
                bail!("Invalid tensor bounds for {name} in {}", path.display());
            }
            let elem_bytes = match entry.dtype.as_str() {
                "F32" => 4_u64,
                "F16" | "BF16" => 2_u64,
                other => bail!("Unsupported safetensors dtype {other} for {name}"),
            };
            let elems = entry
                .shape
                .iter()
                .try_fold(1_usize, |acc, &d| acc.checked_mul(d))
                .ok_or_else(|| anyhow::anyhow!("Tensor shape overflow for {name}"))?;
            if (elems as u64).checked_mul(elem_bytes) != Some(end - start) {
                bail!("Tensor byte length mismatch for {name}");
            }
            tensors.insert(
                name,
                TensorEntry {
                    dtype: entry.dtype,
                    shape: entry.shape,
                    start,
                    end,
                },
            );
        }
        Ok(Self {
            file,
            data_offset,
            tensors,
            buf: Vec::new(),
        })
    }

    fn read_raw(&mut self, name: &str, expected_shape: &[usize]) -> Result<String> {
        let entry = self
            .tensors
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Missing tensor {name}"))?;
        if entry.shape != expected_shape {
            bail!(
                "Unexpected shape for {name}: got {:?}, expected {expected_shape:?}",
                entry.shape
            );
        }
        let len = (entry.end - entry.start) as usize;
        let offset = self.data_offset + entry.start;
        let dtype = entry.dtype.clone();
        self.buf.resize(len, 0);
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.read_exact(&mut self.buf)?;
        Ok(dtype)
    }

    fn tensor(&mut self, name: &str, expected_shape: &[usize]) -> Result<Tensor> {
        let dtype = self.read_raw(name, expected_shape)?;
        let values: Vec<f32> = match dtype.as_str() {
            "F32" => self
                .buf
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect(),
            "F16" => self
                .buf
                .chunks_exact(2)
                .map(|c| f16::from_le_bytes([c[0], c[1]]).to_f32())
                .collect(),
            "BF16" => self
                .buf
                .chunks_exact(2)
                .map(|c| bf16::from_le_bytes([c[0], c[1]]).to_f32())
                .collect(),
            other => bail!("Unsupported tensor dtype {other}"),
        };
        Ok(Tensor::from_vec(values, expected_shape, &Device::Cpu)?)
    }

    fn tensor_f16(&mut self, name: &str, expected_shape: &[usize]) -> Result<Tensor> {
        let dtype = self.read_raw(name, expected_shape)?;
        let values: Vec<f16> = match dtype.as_str() {
            "F32" => self
                .buf
                .chunks_exact(4)
                .map(|c| f16::from_f32(f32::from_le_bytes([c[0], c[1], c[2], c[3]])))
                .collect(),
            "F16" => self
                .buf
                .chunks_exact(2)
                .map(|c| f16::from_le_bytes([c[0], c[1]]))
                .collect(),
            "BF16" => self
                .buf
                .chunks_exact(2)
                .map(|c| f16::from_f32(bf16::from_le_bytes([c[0], c[1]]).to_f32()))
                .collect(),
            other => bail!("Unsupported tensor dtype {other}"),
        };
        Ok(Tensor::from_vec(values, expected_shape, &Device::Cpu)?)
    }

    fn qmatmul(&mut self, name: &str, expected_shape: &[usize]) -> Result<QMatMul> {
        let tensor = self.tensor(name, expected_shape)?;
        let qt = QTensor::quantize(&tensor, GgmlDType::Q8_0)?;
        drop(tensor);
        Ok(QMatMul::from_qtensor(qt)?)
    }

    fn linear(
        &mut self,
        prefix: &str,
        out_dim: usize,
        in_dim: usize,
        bias: bool,
    ) -> Result<Linear> {
        let weight = self.tensor(&format!("{prefix}.weight"), &[out_dim, in_dim])?;
        let bias = if bias {
            Some(self.tensor(&format!("{prefix}.bias"), &[out_dim])?)
        } else {
            None
        };
        Ok(Linear::new(weight, bias))
    }

    fn qlinear(
        &mut self,
        prefix: &str,
        out_dim: usize,
        in_dim: usize,
        bias: bool,
    ) -> Result<QLinear> {
        let weight = self.qmatmul(&format!("{prefix}.weight"), &[out_dim, in_dim])?;
        let bias = if bias {
            Some(self.tensor(&format!("{prefix}.bias"), &[out_dim])?)
        } else {
            None
        };
        Ok(QLinear { weight, bias })
    }

    fn layer_norm(&mut self, prefix: &str, size: usize) -> Result<LayerNorm> {
        let weight = self.tensor(&format!("{prefix}.weight"), &[size])?;
        let bias = self.tensor(&format!("{prefix}.bias"), &[size])?;
        Ok(LayerNorm::new(weight, bias, 1e-5))
    }

    fn conv1d(
        &mut self,
        prefix: &str,
        out_ch: usize,
        in_ch: usize,
        kernel: usize,
        cfg: Conv1dConfig,
    ) -> Result<Conv1d> {
        let weight = self.tensor(&format!("{prefix}.weight"), &[out_ch, in_ch, kernel])?;
        let bias = self.tensor(&format!("{prefix}.bias"), &[out_ch])?;
        Ok(Conv1d::new(weight, Some(bias), cfg))
    }
}

struct MelExtractor {
    filters: Vec<f32>,
    cos_basis: Vec<f32>,
    sin_basis: Vec<f32>,
}

impl MelExtractor {
    fn new(filters: Vec<f32>) -> Self {
        let hann: Vec<f64> = (0..N_FFT)
            .map(|j| 0.5 * (1.0 - (2.0 * std::f64::consts::PI * (j as f64) / (N_FFT as f64)).cos()))
            .collect();
        let mut cos_basis = vec![0.0_f32; N_FREQ * N_FFT];
        let mut sin_basis = vec![0.0_f32; N_FREQ * N_FFT];
        let step = 2.0 * std::f64::consts::PI / (N_FFT as f64);
        for k in 0..N_FREQ {
            let row = k * N_FFT;
            let k_f = k as f64;
            for j in 0..N_FFT {
                let angle = step * k_f * (j as f64);
                cos_basis[row + j] = (hann[j] * angle.cos()) as f32;
                sin_basis[row + j] = (-hann[j] * angle.sin()) as f32;
            }
        }
        Self {
            filters,
            cos_basis,
            sin_basis,
        }
    }

    fn extract(&self, pcm: &[f32]) -> (Vec<f32>, usize) {
        let raw_frames = pcm.len() / HOP_LENGTH;
        // Whisper medium.en requires at least 1500 mel frames (750 encoder positions) and ~4s of
        // trailing silence for stable <EOT> termination without repetition on short utterances.
        let total_frames = (((raw_frames + 400).clamp(1500, m::N_FRAMES)) + 1) & !1;
        let mel = self.extract_with_frames(pcm, total_frames);
        (mel, total_frames)
    }

    fn extract_with_frames(&self, pcm: &[f32], total_frames: usize) -> Vec<f32> {
        let target_samples = total_frames * HOP_LENGTH;
        let half_fft = N_FFT / 2;
        let active_frames = ((pcm.len() + half_fft).div_ceil(HOP_LENGTH) + 1).min(total_frames);
        let columns: Vec<[f32; N_MELS]> = (0..active_frames)
            .into_par_iter()
            .map(|frame| {
                let base = (frame * HOP_LENGTH) as isize - (half_fft as isize);
                let mut win = [0.0_f32; N_FFT];
                for (j, slot) in win.iter_mut().enumerate() {
                    let pos = base + (j as isize);
                    let idx = if pos < 0 {
                        (-pos) as usize
                    } else if (pos as usize) >= target_samples {
                        2 * (target_samples - 1) - (pos as usize)
                    } else {
                        pos as usize
                    };
                    *slot = if idx < pcm.len() { pcm[idx] } else { 0.0 };
                }

                let mut power = [0.0_f32; N_FREQ];
                for (k, slot) in power.iter_mut().enumerate() {
                    let row = k * N_FFT;
                    let cb = &self.cos_basis[row..row + N_FFT];
                    let sb = &self.sin_basis[row..row + N_FFT];
                    let (mut re, mut im) = (0.0_f32, 0.0_f32);
                    for ((&x, &c), &s) in win.iter().zip(cb).zip(sb) {
                        re += x * c;
                        im += x * s;
                    }
                    *slot = re * re + im * im;
                }

                let mut bins = [SILENCE_LOG10; N_MELS];
                for (bin, slot) in bins.iter_mut().enumerate() {
                    let filter = &self.filters[bin * N_FREQ..(bin + 1) * N_FREQ];
                    let sum: f32 = power.iter().zip(filter).map(|(&p, &w)| p * w).sum();
                    *slot = sum.max(1e-10).log10();
                }
                bins
            })
            .collect();

        let mut mel = vec![SILENCE_LOG10; N_MELS * total_frames];
        for (frame, col) in columns.iter().enumerate() {
            for (bin, &val) in col.iter().enumerate() {
                mel[bin * total_frames + frame] = val;
            }
        }
        let mmax = mel.iter().copied().fold(f32::NEG_INFINITY, f32::max) - 8.0;
        for value in &mut mel {
            *value = (*value).max(mmax) * 0.25 + 1.0;
        }
        mel
    }
}

fn validate_config(config: &m::Config) -> Result<()> {
    if config.num_mel_bins != N_MELS
        || config.d_model != 1024
        || config.encoder_layers != 24
        || config.decoder_layers != 24
        || config.vocab_size != 51864
        || config.max_source_positions != 1500
        || config.max_target_positions < 226
        || config.encoder_attention_heads == 0
        || config.decoder_attention_heads == 0
        || !config.d_model.is_multiple_of(config.encoder_attention_heads)
        || !config.d_model.is_multiple_of(config.decoder_attention_heads)
    {
        bail!("Expected English Whisper medium.en");
    }
    Ok(())
}

fn load_config(root: &Path) -> Result<m::Config> {
    let mut config: m::Config = serde_json::from_slice(&std::fs::read(root.join("config.json"))?)?;
    if config.suppress_tokens.is_empty() {
        let generation_path = root.join("generation_config.json");
        if generation_path.is_file() {
            let value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(generation_path)?)?;
            if let Some(tokens) = value.get("suppress_tokens") {
                config.suppress_tokens = serde_json::from_value(tokens.clone())?;
            }
        }
    }
    if config.suppress_tokens.is_empty() {
        bail!("Expected non-empty Whisper suppress_tokens");
    }
    Ok(config)
}

fn mel_filters(num_mel_bins: usize) -> Result<Vec<f32>> {
    let bytes: &[u8] = match num_mel_bins {
        N_MELS => include_bytes!("melfilters.bytes"),
        other => bail!("Unsupported Whisper mel bin count: {other}"),
    };
    Ok(bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn reference_centered_mel(pcm: &[f32], filters: &[f32], total_frames: usize) -> Vec<f32> {
        let target_samples = total_frames * HOP_LENGTH;
        let half_fft = N_FFT / 2;
        let mut padded = vec![0.0_f64; target_samples + 2 * half_fft];
        for (i, &sample) in pcm.iter().take(target_samples).enumerate() {
            padded[half_fft + i] = sample as f64;
        }
        for i in 0..half_fft {
            padded[half_fft - 1 - i] = padded[half_fft + 1 + i];
            padded[half_fft + target_samples + i] = padded[half_fft + target_samples - 2 - i];
        }
        let hann: Vec<f64> = (0..N_FFT)
            .map(|j| {
                0.5 * (1.0 - (2.0 * std::f64::consts::PI * (j as f64) / (N_FFT as f64)).cos())
            })
            .collect();
        let mut cos_tab = vec![0.0_f64; N_FREQ * N_FFT];
        let mut sin_tab = vec![0.0_f64; N_FREQ * N_FFT];
        for k in 0..N_FREQ {
            for j in 0..N_FFT {
                let angle =
                    2.0 * std::f64::consts::PI * (k as f64) * (j as f64) / (N_FFT as f64);
                cos_tab[k * N_FFT + j] = hann[j] * angle.cos();
                sin_tab[k * N_FFT + j] = -hann[j] * angle.sin();
            }
        }
        let mut log_mel = vec![0.0_f64; N_MELS * total_frames];
        for frame in 0..total_frames {
            let start = frame * HOP_LENGTH;
            let win = &padded[start..start + N_FFT];
            let mut power = [0.0_f64; N_FREQ];
            for (k, slot) in power.iter_mut().enumerate() {
                let row = k * N_FFT;
                let mut re = 0.0_f64;
                let mut im = 0.0_f64;
                for j in 0..N_FFT {
                    re += win[j] * cos_tab[row + j];
                    im += win[j] * sin_tab[row + j];
                }
                *slot = re * re + im * im;
            }
            for bin in 0..N_MELS {
                let sum: f64 = power
                    .iter()
                    .enumerate()
                    .map(|(k, &p)| p * (filters[bin * N_FREQ + k] as f64))
                    .sum();
                log_mel[bin * total_frames + frame] = sum.max(1e-10).log10();
            }
        }
        let mmax = log_mel.iter().copied().fold(f64::NEG_INFINITY, f64::max) - 8.0;
        log_mel
            .into_iter()
            .map(|v| (v.max(mmax) * 0.25 + 1.0) as f32)
            .collect()
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("kestrel-whisper-test-{name}-{nonce}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn medium_en_config() -> m::Config {
        m::Config {
            num_mel_bins: 80,
            max_source_positions: 1500,
            d_model: 1024,
            encoder_attention_heads: 16,
            encoder_layers: 24,
            vocab_size: 51864,
            max_target_positions: 448,
            decoder_attention_heads: 16,
            decoder_layers: 24,
            suppress_tokens: vec![1, 2, 7],
        }
    }

    #[test]
    fn loads_suppress_tokens_from_generation_config_when_config_is_empty() {
        let dir = temp_dir("gen-suppress");
        std::fs::write(
            dir.join("config.json"),
            r#"{
                "num_mel_bins": 80,
                "max_source_positions": 1500,
                "d_model": 1024,
                "encoder_attention_heads": 16,
                "encoder_layers": 24,
                "vocab_size": 51864,
                "max_target_positions": 448,
                "decoder_attention_heads": 16,
                "decoder_layers": 24,
                "suppress_tokens": []
            }"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("generation_config.json"),
            r#"{ "suppress_tokens": [1, 2, 7] }"#,
        )
        .unwrap();

        let config = load_config(&dir).unwrap();
        assert_eq!(config.suppress_tokens, vec![1, 2, 7]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_empty_suppress_tokens() {
        let dir = temp_dir("empty-suppress");
        std::fs::write(
            dir.join("config.json"),
            r#"{
                "num_mel_bins": 80,
                "max_source_positions": 1500,
                "d_model": 1024,
                "encoder_attention_heads": 16,
                "encoder_layers": 24,
                "vocab_size": 51864,
                "max_target_positions": 448,
                "decoder_attention_heads": 16,
                "decoder_layers": 24,
                "suppress_tokens": []
            }"#,
        )
        .unwrap();

        assert!(load_config(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_mismatched_whisper_configs() {
        let base = medium_en_config();
        assert!(validate_config(&base).is_ok());

        let mut wrong_d_model = base.clone();
        wrong_d_model.d_model = 768;
        assert!(validate_config(&wrong_d_model).is_err());

        let mut wrong_decoder_layers = base.clone();
        wrong_decoder_layers.decoder_layers = 2;
        assert!(validate_config(&wrong_decoder_layers).is_err());

        let mut wrong_vocab = base;
        wrong_vocab.vocab_size = 51865;
        assert!(validate_config(&wrong_vocab).is_err());
    }

    #[test]
    fn mel_extractor_matches_reference_whisper_audio() {
        let filters = mel_filters(N_MELS).unwrap();
        let extractor = MelExtractor::new(filters.clone());
        let pcm: Vec<f32> = (0_u32..16_000 * 3 + 73)
            .map(|i| {
                let t = (i as f32) / 16_000.0;
                // Deterministic formant-like multi-harmonic + fricative noise excitation.
                let mut sample = 0.0_f32;
                for h in 1..=24 {
                    sample += (1.0 / (h as f32))
                        * (2.0 * std::f32::consts::PI * 130.0 * (h as f32) * t).sin();
                }
                let noise = (((i.wrapping_mul(1_664_525).wrapping_add(1_013_904_223)) >> 16)
                    & 0xff) as f32
                    / 255.0
                    - 0.5;
                0.2 * sample + 0.05 * noise
            })
            .collect();

        let ref_tensor = reference_centered_mel(&pcm, &filters, m::N_FRAMES);
        let fast = extractor.extract_with_frames(&pcm, m::N_FRAMES);
        assert_eq!(fast.len(), ref_tensor.len());
        let (max_idx, max_diff) = fast
            .iter()
            .zip(ref_tensor.iter())
            .enumerate()
            .fold((0_usize, 0.0_f32), |(mi, md), (i, (a, b))| {
                let d = (a - b).abs();
                if d > md { (i, d) } else { (mi, md) }
            });
        assert!(
            max_diff < 1e-4,
            "max mel diff {max_diff} >= 1e-4 at idx {max_idx} (bin {}, frame {}): fast={} ref={}",
            max_idx / m::N_FRAMES,
            max_idx % m::N_FRAMES,
            fast[max_idx],
            ref_tensor[max_idx]
        );
        assert_eq!(extractor.extract(&pcm).1, 1500);
        assert_eq!(extractor.extract(&vec![0.0_f32; 16_000 * 24]).1, 2800);
    }

    #[test]
    fn selects_mel_filters_by_bin_count() {
        assert_eq!(mel_filters(80).unwrap().len(), 80 * 201);
        assert!(mel_filters(128).is_err());
        assert!(mel_filters(64).is_err());
    }
}
