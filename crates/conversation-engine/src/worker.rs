//! Bounded JSON protocol. Independent single-threaded TTS/STT executors retain one model each.
use crate::{
    config::Config,
    model::{LoadOptions, TtsModel, VoiceState},
    whisper::Recognizer,
};
use anyhow::{Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::io::Read;
use std::{
    collections::HashMap,
    io::{BufRead, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
};

type Backend = xn::Unquantized<f32, xn::CpuDevice>;
type Jobs = Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>;
type Task = (Value, Arc<AtomicBool>);
static NEXT_MODEL: AtomicU64 = AtomicU64::new(1);
pub const LANGUAGES: [&str; 7] = ["en", "fr", "de", "pt", "it", "es", "nl"];

#[derive(Clone)]
pub(crate) struct Output(Arc<Mutex<std::io::Stdout>>);
impl Output {
    pub(crate) fn emit(&self, id: &str, kind: &str, mut value: Value) -> Result<()> {
        value["id"] = json!(id);
        value["kind"] = json!(kind);
        let mut out = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Speech output unavailable"))?;
        serde_json::to_writer(&mut *out, &value)?;
        out.write_all(b"\n")?;
        out.flush()?;
        Ok(())
    }
}
struct Selected {
    language: String,
    model: TtsModel<Backend>,
    voice: VoiceState<Backend>,
    identity: u64,
}
impl Selected {
    fn load(root: &Path, language: &str) -> Result<Self> {
        if !LANGUAGES.contains(&language) {
            bail!("Unsupported conversation language");
        }
        let path = root.join("models").join(language);
        let config = Config::parse(&std::fs::read_to_string(path.join("config.yaml"))?)?;
        let model = TtsModel::load(config, &LoadOptions::default())?;
        let voice = model.import_state(&path.join("voice.safetensors"))?;
        Ok(Self {
            language: language.into(),
            model,
            voice,
            identity: NEXT_MODEL.fetch_add(1, Ordering::Relaxed),
        })
    }
}
fn field<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v[key]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing speech {key}"))
}
fn finish(output: &Output, jobs: &Jobs, id: &str, cancel: &AtomicBool, result: Result<()>) {
    if let Err(error) = result {
        let _ = output.emit(
            id,
            "error",
            json!({"detail":error.to_string().chars().take(1500).collect::<String>()}),
        );
    }
    let _ = output.emit(
        id,
        if cancel.load(Ordering::Acquire) {
            "cancelled"
        } else {
            "done"
        },
        json!({}),
    );
    if let Ok(mut jobs) = jobs.lock() {
        jobs.remove(id);
    }
}

pub fn run(root: PathBuf) -> Result<()> {
    if !root.is_absolute() || !root.is_dir() {
        bail!("Speech requires an installed absolute asset directory");
    }
    let whisper_threads = std::thread::available_parallelism()
        .map(|n| (n.get() / 4).clamp(2, 8))
        .unwrap_or(2);
    // Keep PocketTTS's custom xn threadpool at 2 threads (caller + 1 worker) while giving
    // CPU Whisper a dedicated bounded Rayon pool without oversubscribing low-latency synthesis.
    let _ = xn::threadpool::start_pool_with(
        xn::threadpool::PoolConfig {
            workers: 1,
            spin_budget: None,
        },
        |task| {
            let _ = std::thread::Builder::new()
                .name("kestrel-pocket-tts".into())
                .spawn(task);
        },
    );
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(whisper_threads)
        .build_global();
    let output = Output(Arc::new(Mutex::new(std::io::stdout())));
    let jobs: Jobs = Arc::new(Mutex::new(HashMap::new()));
    let (tts_tx, tts_rx) = mpsc::sync_channel::<Task>(2);
    let (stt_tx, stt_rx) = mpsc::sync_channel::<Task>(2);
    let stt_identity = Arc::new(AtomicU64::new(0));
    let device = Arc::new(Mutex::new(None::<crate::device::Playback>));
    let tts_thread = {
        let root = root.clone();
        let output = output.clone();
        let jobs = jobs.clone();
        let stt_identity = stt_identity.clone();
        let device = device.clone();
        std::thread::spawn(move || {
            let mut selected: Option<Selected> = None;
            for (request, cancel) in tts_rx {
                let id = request["id"].as_str().unwrap_or_default();
                let result = (|| -> Result<()> {
                    if cancel.load(Ordering::Acquire) {
                        return Ok(());
                    }
                    match field(&request, "op")? {
                        "status" => {
                            let models = selected
                                .as_ref()
                                .map(|s| json!({s.language.clone():s.identity}))
                                .unwrap_or_else(|| json!({}));
                            output.emit(
                                id,
                                "status",
                                json!({
                                    "pid": std::process::id(),
                                    "models": models,
                                    "stt": stt_identity.load(Ordering::Acquire),
                                    "cuda": null,
                                    "devices": selected.as_ref()
                                        .map(|s| json!({s.language.clone(): "cpu"}))
                                        .unwrap_or_else(|| json!({}))
                                }),
                            )?;
                        }
                        "warm_tts" | "speak" => {
                            let language = field(&request, "language")?;
                            if request["op"] == "speak"
                                && selected.as_ref().is_some_and(|s| s.language != language)
                            {
                                bail!(
                                    "Conversation language changed. Prepare the selected language before speaking again."
                                );
                            }
                            if selected.as_ref().is_none_or(|s| s.language != language) {
                                selected = None;
                                selected = Some(Selected::load(&root, language)?);
                            }
                            if request["op"] == "speak" {
                                let text = field(&request, "text")?;
                                if text.trim().is_empty() || text.len() > 1200 {
                                    bail!("A spoken clause must contain 1–1200 UTF-8 bytes");
                                }
                                if request["voicePath"].as_str().is_some() {
                                    bail!(
                                        "Conversation uses the supplied language voice. Recorded voices remain available in manual TTS."
                                    );
                                }
                                let selected = selected.as_ref().unwrap();
                                let playback = if request["nativePlayback"].as_bool() == Some(true)
                                {
                                    Some(device.lock().unwrap().clone().ok_or_else(|| {
                                        anyhow::anyhow!("Microphone session is no longer active")
                                    })?)
                                } else {
                                    None
                                };
                                if let Some(p) = &playback {
                                    if request["audioSessionId"].as_str()
                                        != Some(p.session_id.as_str())
                                    {
                                        bail!(
                                            "The audio session changed. Old speech was discarded."
                                        );
                                    }
                                    p.begin(cancel.clone())?;
                                }
                                let mut samples = 0;
                                let synthesis = selected.model.generate_audio_stream(
                                    &selected.voice, text, 50, None, &cancel, &mut |pcm| {
                                        if cancel.load(Ordering::Acquire) { return Ok(()); }
                                        samples += pcm.len();
                                        if samples > 24_000 * 60 {
                                            bail!("Speech exceeded the 60-second clause limit");
                                        }
                                        if let Some(p) = &playback {
                                            p.push(pcm, &cancel)?;
                                        } else {
                                            for part in pcm.chunks(2400) {
                                                if cancel.load(Ordering::Acquire) { break; }
                                                let bytes: Vec<u8> = part.iter().flat_map(|s| {
                                                    ((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()
                                                }).collect();
                                                output.emit(id, "audio", json!({
                                                    "sampleRate": 24000, "pcm": STANDARD.encode(bytes)
                                                }))?;
                                            }
                                        }
                                        Ok(())
                                    },
                                );
                                if synthesis.is_err() {
                                    cancel.store(true, Ordering::Release);
                                }
                                if let Some(p) = playback {
                                    p.drain(&cancel)?;
                                }
                                synthesis?;
                            }
                        }
                        _ => bail!("Unknown speech operation"),
                    }
                    Ok(())
                })();
                finish(&output, &jobs, id, &cancel, result);
            }
        })
    };
    let stt_thread = {
        let root = root.clone();
        let output = output.clone();
        let jobs = jobs.clone();
        let stt_identity = stt_identity.clone();
        std::thread::spawn(move || {
            let mut recognizer = None;
            for (request, cancel) in stt_rx {
                let id = request["id"].as_str().unwrap_or_default();
                let live = request["live"].as_bool() == Some(true);
                let result = (|| -> Result<()> {
                    if cancel.load(Ordering::Acquire) {
                        return Ok(());
                    }
                    if recognizer.is_none() {
                        recognizer = Some(Recognizer::load(&root)?);
                        stt_identity.store(
                            NEXT_MODEL.fetch_add(1, Ordering::Relaxed),
                            Ordering::Release,
                        );
                    }
                    if request["op"] == "transcribe" {
                        let language = field(&request, "language")?;
                        if !LANGUAGES.contains(&language) {
                            bail!("Select a supported conversation language before speaking");
                        }
                        let bytes = STANDARD.decode(field(&request, "pcm")?)?;
                        if !(3200..=960000).contains(&bytes.len()) || bytes.len() % 2 != 0 {
                            bail!("Invalid microphone PCM length");
                        }
                        let pcm: Vec<f32> = bytes
                            .chunks_exact(2)
                            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
                            .collect();
                        let text = recognizer
                            .as_mut()
                            .unwrap()
                            .transcribe(&pcm, language, &cancel)?;
                        if !cancel.load(Ordering::Acquire) {
                            output.emit(
                                id,
                                "transcript",
                                json!({"text":text,"language":language,"words":[]}),
                            )?;
                        }
                    }
                    Ok(())
                })();
                if live {
                    if let Err(error) = result {
                        let _ =
                            output.emit(id, "capture_error", json!({"detail":error.to_string()}));
                    }
                } else {
                    finish(&output, &jobs, id, &cancel, result);
                }
            }
        })
    };
    let mut audio_thread = None;
    let mut audio_cancel: Option<Arc<AtomicBool>> = None;
    let mut input = std::io::stdin().lock();
    loop {
        let mut line = Vec::new();
        let count = (&mut input).take(1_400_001).read_until(b'\n', &mut line)?;
        if count == 0 {
            break;
        }
        if count > 1_400_000 || !line.ends_with(b"\n") {
            bail!("Oversized speech request");
        }
        let request: Value = serde_json::from_slice(&line)?;
        let id = field(&request, "id")?.to_string();
        let op = field(&request, "op")?;
        if id.is_empty() || id.len() > 200 {
            bail!("Invalid speech request identity");
        }
        if op == "cancel" {
            if let Some(flag) = jobs.lock().unwrap().get(&id) {
                flag.store(true, Ordering::Release);
            }
            continue;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut jobs = jobs.lock().unwrap();
            if jobs.contains_key(&id) || jobs.len() >= 8 {
                output.emit(
                    &id,
                    "error",
                    json!({"detail":"Speech queue is full; retry after the current utterance"}),
                )?;
                continue;
            }
            jobs.insert(id.clone(), cancel.clone());
        }
        if op == "listen" {
            if audio_thread
                .as_ref()
                .is_some_and(|h: &std::thread::JoinHandle<()>| !h.is_finished())
                && !audio_cancel
                    .as_ref()
                    .is_some_and(|flag| flag.load(Ordering::Acquire))
            {
                finish(
                    &output,
                    &jobs,
                    &id,
                    &cancel,
                    Err(anyhow::anyhow!(
                        "A microphone session is still closing. Retry shortly."
                    )),
                );
                continue;
            }
            if let Some(h) = audio_thread.take() {
                let _ = h.join();
            }
            let output = output.clone();
            let jobs = jobs.clone();
            let tx = stt_tx.clone();
            let device = device.clone();
            audio_cancel = Some(cancel.clone());
            audio_thread = Some(std::thread::spawn(move || {
                let result = crate::device::listen(&request, &cancel, &device, |kind, data| {
                    if kind == "utterance" {
                        let utterance = json!({"id":id,"op":"transcribe","live":true,"language":request["language"],"pcm":data["pcm"]});
                        tx.try_send((utterance, cancel.clone())).map_err(|_| {
                            anyhow::anyhow!("Please pause while the previous speech is transcribed")
                        })?;
                    } else {
                        output.emit(&id, kind, data)?;
                    }
                    Ok(())
                });
                // Device failure must also invalidate recognition already queued for this session.
                cancel.store(true, Ordering::Release);
                *device.lock().unwrap() = None;
                finish(&output, &jobs, &id, &cancel, result);
            }));
            continue;
        }
        let tx = if matches!(op, "warm_stt" | "transcribe") {
            &stt_tx
        } else {
            &tts_tx
        };
        if tx.try_send((request, cancel.clone())).is_err() {
            finish(
                &output,
                &jobs,
                &id,
                &cancel,
                Err(anyhow::anyhow!("Speech queue is full")),
            );
        }
    }
    for flag in jobs.lock().unwrap().values() {
        flag.store(true, Ordering::Release);
    }
    if let Some(h) = audio_thread {
        let _ = h.join();
    }
    drop(tts_tx);
    drop(stt_tx);
    tts_thread
        .join()
        .map_err(|_| anyhow::anyhow!("TTS worker panicked"))?;
    stt_thread
        .join()
        .map_err(|_| anyhow::anyhow!("STT worker panicked"))?;
    Ok(())
}
