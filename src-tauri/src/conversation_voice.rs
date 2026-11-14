//! Resident CPU speech. Never touches RuntimeManager, ComfyUI, or the language-model lease.
use base64::{engine::general_purpose::STANDARD, Engine as _};
use kestrel_app_core::{
    ConversationAudio, ConversationSpeakRequest, ConversationTranscribeRequest,
    ConversationTranscript, ConversationVoiceSnapshot,
};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::{mpsc, Mutex as AsyncMutex, Semaphore},
};
use tokio_util::sync::CancellationToken;

pub const REVISION: &str = "pocket-rust-9e8413e-whisper-medium-en-candle-v1";
pub const LANGUAGES: [&str; 7] = ["en", "fr", "de", "pt", "it", "es", "nl"];
type Pending = Arc<Mutex<HashMap<String, mpsc::Sender<Value>>>>;

fn worker_executable() -> Result<PathBuf, String> {
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    #[cfg(test)]
    {
        if let Some(path) = std::env::var_os("KESTREL_VOICE_WORKER_EXE") {
            let path = PathBuf::from(path);
            if !path.is_absolute() || !path.is_file() {
                return Err("Invalid explicit test worker path".into());
            }
            return Ok(path);
        }
        // Cargo's test harness cannot dispatch the application worker entry point.
        let path = executable
            .parent()
            .and_then(Path::parent)
            .ok_or("Missing test executable directory")?
            .join(if cfg!(windows) {
                "kestrel-local.exe"
            } else {
                "kestrel-local"
            });
        if !path.is_file() {
            return Err("Build the Kestrel binary before running live voice tests.".into());
        }
        Ok(path)
    }
    #[cfg(not(test))]
    Ok(executable)
}

struct Worker {
    child: Child,
    #[cfg(windows)]
    _job: crate::strata::ProcessJob,
    input: Arc<AsyncMutex<ChildStdin>>,
    pending: Pending,
    root: PathBuf,
    alive: Arc<AtomicBool>,
}

pub struct ConversationVoice {
    worker: AsyncMutex<Option<Worker>>,
    jobs: Mutex<HashMap<String, CancellationToken>>,
    cancelled: Mutex<VecDeque<String>>,
    tts: Semaphore,
    stt: Semaphore,
}

pub fn root(install: &str) -> PathBuf {
    Path::new(install).join("ConversationVoice")
}

pub fn installed(install: &str) -> bool {
    let root = root(install);
    std::fs::File::open(root.join("ready.txt"))
        .and_then(|file| {
            let mut value = String::new();
            std::io::Read::read_to_string(&mut std::io::Read::take(file, 256), &mut value)
                .map(|_| value)
        })
        .is_ok_and(|s| s.trim() == REVISION)
        && LANGUAGES
            .iter()
            .all(|lang| root.join(format!("models/{lang}/config.yaml")).is_file())
        && root
            .join("models/whisper-medium-en-native/model.safetensors")
            .is_file()
        && crate::setup::conversation::assets().iter().all(|asset| {
            std::fs::metadata(root.join(&asset.relative))
                .is_ok_and(|m| m.is_file() && m.len() == asset.bytes)
        })
}

impl ConversationVoice {
    pub fn new() -> Self {
        Self {
            worker: AsyncMutex::new(None),
            jobs: Mutex::new(HashMap::new()),
            cancelled: Mutex::new(VecDeque::new()),
            tts: Semaphore::new(1),
            stt: Semaphore::new(1),
        }
    }

    pub async fn snapshot(&self, install: &str) -> ConversationVoiceSnapshot {
        let mut worker = self.worker.lock().await;
        let process_id = worker
            .as_mut()
            .filter(|w| w.root == root(install))
            .and_then(|w| match w.child.try_wait() {
                Ok(None) if w.alive.load(Ordering::Acquire) => w.child.id(),
                _ => None,
            });
        let installed = installed(install);
        ConversationVoiceSnapshot {
            installed,
            running: process_id.is_some(),
            process_id,
            languages: LANGUAGES.iter().map(|s| (*s).into()).collect(),
            detail: if installed {
                "Resident CPU speech is available; the selected LLM stays loaded."
            } else {
                "Prepare voice conversation from Setup before using automatic speech."
            }
            .into(),
        }
    }

    async fn connection(
        &self,
        install: &str,
    ) -> Result<(Arc<AsyncMutex<ChildStdin>>, Pending, Arc<AtomicBool>), String> {
        let mut slot = self.worker.lock().await;
        let root = root(install);
        if let Some(worker) = slot.as_mut() {
            if worker.root != root {
                return Err(
                    "The speech installation changed. Restart Kestrel to use the new location."
                        .into(),
                );
            }
            match worker.child.try_wait().map_err(|e| e.to_string())? {
                None if worker.alive.load(Ordering::Acquire) => {
                    return Ok((
                        worker.input.clone(),
                        worker.pending.clone(),
                        worker.alive.clone(),
                    ))
                }
                None => {
                    let _ = worker.child.kill().await;
                    *slot = None;
                }
                Some(_) => {
                    *slot = None;
                }
            }
        }
        if !installed(install) {
            return Err("Voice conversation is not installed. Prepare it from Setup; existing manual speech remains available.".into());
        }
        crate::setup::conversation::write_local_configs(&root).map_err(|e| e.to_string())?;
        let whisper_threads = std::thread::available_parallelism()
            .map(|n| (n.get() / 4).clamp(2, 8))
            .unwrap_or(2);
        let mut command = Command::new(worker_executable()?);
        command
            .arg("--conversation-voice-worker")
            .arg(&root)
            .current_dir(&root)
            .env("RAYON_NUM_THREADS", whisper_threads.to_string())
            .env("TOKENIZERS_PARALLELISM", "false")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        let (mut child, job) = crate::strata::ProcessJob::spawn(&mut command)
            .map_err(|e| format!("Cannot own CPU speech process: {e}"))?;
        #[cfg(not(windows))]
        let mut child = command.spawn().map_err(|e| e.to_string())?;
        let input = Arc::new(AsyncMutex::new(
            child
                .stdin
                .take()
                .ok_or("Speech input pipe is unavailable")?,
        ));
        let output = child
            .stdout
            .take()
            .ok_or("Speech output pipe is unavailable")?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or("Speech diagnostics pipe is unavailable")?;
        let diagnostic = Arc::new(Mutex::new(String::new()));
        let errors = diagnostic.clone();
        tokio::spawn(async move {
            let mut bytes = [0; 2048];
            while let Ok(count @ 1..) = stderr.read(&mut bytes).await {
                if let Ok(mut tail) = errors.lock() {
                    tail.push_str(&String::from_utf8_lossy(&bytes[..count]));
                    if tail.len() > 4096 {
                        let start = tail
                            .char_indices()
                            .map(|(at, _)| at)
                            .find(|at| *at >= tail.len() - 4096)
                            .unwrap_or(0);
                        tail.drain(..start);
                    }
                }
            }
        });
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let replies = pending.clone();
        let alive = Arc::new(AtomicBool::new(true));
        let reader_alive = alive.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(output);
            loop {
                let mut line = Vec::new();
                let result = (&mut reader)
                    .take(65_537)
                    .read_until(b'\n', &mut line)
                    .await;
                if !matches!(result, Ok(1..)) || line.len() > 65_536 || !line.ends_with(b"\n") {
                    break;
                }
                let Ok(value) = serde_json::from_slice::<Value>(&line) else {
                    break;
                };
                let id = value["id"].as_str().unwrap_or_default();
                let sender = replies.lock().ok().and_then(|map| map.get(id).cloned());
                if let Some(sender) = sender {
                    let _ = sender.send(value).await;
                }
            }
            reader_alive.store(false, Ordering::Release);
            let detail = diagnostic
                .lock()
                .map(|tail| tail.clone())
                .unwrap_or_default();
            let senders = replies
                .lock()
                .map(|mut map| map.drain().map(|(_, tx)| tx).collect::<Vec<_>>())
                .unwrap_or_default();
            for sender in senders {
                let _ = sender.send(json!({"kind":"error","detail":format!("The CPU speech worker stopped. Retry the utterance or repair Voice conversation in Setup; the language model was not restarted. {detail}")})).await;
            }
        });
        *slot = Some(Worker {
            child,
            #[cfg(windows)]
            _job: job,
            input: input.clone(),
            pending: pending.clone(),
            root,
            alive: alive.clone(),
        });
        Ok((input, pending, alive))
    }

    async fn request(
        &self,
        install: &str,
        id: &str,
        mut value: Value,
        mut receive: impl FnMut(Value) -> Result<(), String>,
    ) -> Result<(), String> {
        validate_id(id)?;
        let listening = value["op"] == "listen";
        let cancel = CancellationToken::new();
        {
            // Same lock order as cancel(): cancellation cannot fall between checking the
            // tombstone and registering the live token.
            let cancelled = self
                .cancelled
                .lock()
                .map_err(|_| "Speech cancellation registry unavailable")?;
            if cancelled.iter().any(|cancelled| cancelled == id) {
                return Err("Conversation speech cancelled before it started.".into());
            }
            let mut jobs = self
                .jobs
                .lock()
                .map_err(|_| "Speech job registry unavailable")?;
            if jobs.contains_key(id) {
                return Err("A speech request with this ID already exists.".into());
            }
            jobs.insert(id.into(), cancel.clone());
        }
        let result = async {
            if cancel.is_cancelled() { return Err("Conversation speech cancelled.".into()); }
            let (input, pending, alive) = self.connection(install).await?;
            let (tx, mut rx) = mpsc::channel(8);
            pending.lock().map_err(|_| "Speech reply registry unavailable")?.insert(id.into(), tx);
            value["id"] = json!(id);
            let result = async {
                // The reader marks itself dead before draining the registry. Either this
                // check sees the failure or that drain delivers it to our registered receiver.
                if !alive.load(Ordering::Acquire) { return Err("CPU speech disconnected before this utterance; retry it.".into()); }
                if cancel.is_cancelled() { return Err("Conversation speech cancelled.".into()); }
                send(&input, &value).await?;
                loop {
                    let value = tokio::select! {
                        _ = cancel.cancelled() => return Err("Conversation speech cancelled.".into()),
                        value = async {
                            if listening { Ok(rx.recv().await) }
                            else { tokio::time::timeout(Duration::from_secs(120), rx.recv()).await }
                        } => value.map_err(|_| "CPU speech stopped responding; retry this utterance.")?
                            .ok_or("CPU speech disconnected; retry this utterance.")?,
                    };
                    match value["kind"].as_str() {
                        Some("done") => return Ok(()),
                        Some("cancelled") => return Err("Conversation speech cancelled.".into()),
                        Some("error") => return Err(value["detail"].as_str().unwrap_or("CPU speech failed").to_string()),
                        _ => receive(value)?,
                    }
                }
            }.await;
            // Cancellation stops computation cooperatively, retaining model and voice state.
            if result.is_err() {
                let _ = send(&input, &json!({"id":id,"op":"cancel"})).await;
                // A replacement capture must wait until the old native devices are released.
                let _ = tokio::time::timeout(Duration::from_secs(5), async {
                    while let Some(reply) = rx.recv().await {
                        if matches!(reply["kind"].as_str(), Some("done" | "cancelled")) { break; }
                    }
                }).await;
            }
            if let Ok(mut map) = pending.lock() { map.remove(id); }
            result
        }.await;
        if let Ok(mut jobs) = self.jobs.lock() {
            jobs.remove(id);
        }
        result
    }

    pub async fn prepare(
        &self,
        install: &str,
        language: &str,
    ) -> Result<ConversationVoiceSnapshot, String> {
        validate_language(language, false)?;
        let _tts = self.tts.acquire().await.map_err(|e| e.to_string())?;
        let _stt = self.stt.acquire().await.map_err(|e| e.to_string())?;
        self.request(
            install,
            &uuid::Uuid::new_v4().to_string(),
            json!({"op":"warm_stt"}),
            |_| Ok(()),
        )
        .await?;
        self.request(
            install,
            &uuid::Uuid::new_v4().to_string(),
            json!({"op":"warm_tts", "language":language}),
            |_| Ok(()),
        )
        .await?;
        Ok(self.snapshot(install).await)
    }

    pub async fn speak(
        &self,
        install: &str,
        request: ConversationSpeakRequest,
        voice: crate::voice_library::VoiceConditioning,
        app: &tauri::AppHandle,
    ) -> Result<(), String> {
        let _permit = self.tts.try_acquire().map_err(|_| {
            "Speech is still finishing the previous clause; retry after cancellation settles."
        })?;
        validate_language(&request.language, false)?;
        validate_id(&request.conversation_id)?;
        validate_id(&request.response_id)?;
        if request.text.trim().is_empty() || request.text.len() > 1200 {
            return Err("A spoken clause must contain 1–1200 UTF-8 bytes.".into());
        }
        let text = crate::speech_text::conversation_text(&request.text, &request.language);
        let native = request.audio_session_id.is_some();
        if let Some(session) = &request.audio_session_id {
            validate_id(session)?;
        }
        let value = json!({"op":"speak","text":text,"language":request.language,
            "nativePlayback":native,"audioSessionId":request.audio_session_id,
            "voicePath":voice.reference_path,"voiceFingerprint":voice.fingerprint()});
        let mut sequence = 0;
        self.request(install, &request.job_id, value, |value| {
            if value["kind"] != "audio" {
                return Err("Unexpected speech worker reply.".into());
            }
            let rate = value["sampleRate"]
                .as_u64()
                .filter(|r| *r == 24000)
                .ok_or("Invalid speech sample rate")? as u32;
            let pcm = value["pcm"].as_str().ok_or("Missing speech samples")?;
            if sequence >= 600 {
                return Err(
                    "Speech exceeded the 60-second clause limit; the written answer is retained."
                        .into(),
                );
            }
            if pcm.len() > 6400 || STANDARD.decode(pcm).map_err(|e| e.to_string())?.len() % 2 != 0 {
                return Err("Invalid speech PCM frame".into());
            }
            let event = ConversationAudio {
                job_id: request.job_id.clone(),
                conversation_id: request.conversation_id.clone(),
                response_id: request.response_id.clone(),
                sequence,
                sample_rate: rate,
                pcm_base64: pcm.into(),
            };
            crate::ipc_events::emit::<kestrel_app_core::events::ConversationAudio>(app, &event)
                .map_err(|e| e.to_string())?;
            sequence += 1;
            Ok(())
        })
        .await
    }

    pub async fn capture(
        &self,
        install: &str,
        session_id: &str,
        language: &str,
        vad: &kestrel_app_core::VadSettings,
        app: &tauri::AppHandle,
    ) -> Result<(), String> {
        validate_language(language, false)?;
        self.request(
            install,
            session_id,
            json!({"op":"listen", "language":language,
            "threshold":vad.speech_threshold_db, "silenceMs":vad.silence_timeout_sec*1000.0,
            "minimumMs":vad.min_speech_duration_ms}),
            |value| {
                let kind = value["kind"]
                    .as_str()
                    .ok_or("Missing native capture event")?;
                if !["listening", "speech_start", "transcript", "capture_error"].contains(&kind) {
                    return Err("Unexpected native capture event".into());
                }
                let text = value["text"].as_str().unwrap_or_default();
                let detail = value["detail"].as_str().unwrap_or_default();
                if text.len() > 16_000 || detail.len() > 4096 {
                    return Err("Native capture event exceeded its limit".into());
                }
                crate::ipc_events::emit::<kestrel_app_core::events::ConversationCapture>(
                    app,
                    &kestrel_app_core::ConversationCaptureEvent {
                        session_id: session_id.into(),
                        kind: kind.into(),
                        text: text.into(),
                        detail: detail.into(),
                    },
                )
                .map_err(|e| e.to_string())
            },
        )
        .await
    }

    pub async fn transcribe(
        &self,
        install: &str,
        request: ConversationTranscribeRequest,
    ) -> Result<ConversationTranscript, String> {
        let _permit = self
            .stt
            .try_acquire()
            .map_err(|_| "The previous utterance is still being transcribed.")?;
        validate_language(&request.language, true)?;
        if request.pcm_base64.len() > 1_280_000 {
            return Err("An utterance must be at most 30 seconds.".into());
        }
        let bytes = STANDARD
            .decode(&request.pcm_base64)
            .map_err(|_| "Invalid microphone PCM encoding")?;
        if !(3200..=960_000).contains(&bytes.len()) || bytes.len() % 2 != 0 {
            return Err("Microphone audio must be 0.1–30 seconds of mono 16 kHz PCM.".into());
        }
        let mut result = None;
        self.request(
            install,
            &request.job_id,
            json!({"op":"transcribe","pcm":request.pcm_base64,"language":request.language}),
            |v| {
                if v["kind"] != "transcript" || result.is_some() {
                    return Err("Unexpected transcription reply".into());
                }
                let text = v["text"]
                    .as_str()
                    .ok_or("Missing transcription text")?
                    .to_string();
                if text.len() > 16_000 {
                    return Err("Transcription exceeded the utterance limit".into());
                }
                let words: Vec<kestrel_app_core::SpeechTiming> =
                    serde_json::from_value(v["words"].clone()).map_err(|e| e.to_string())?;
                validate_timings(&words, bytes.len() as f64 / 32000.0)?;
                let language = v["language"]
                    .as_str()
                    .ok_or("Missing transcription language")?;
                if language.len() > 16
                    || !language
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b == b'-')
                {
                    return Err("Invalid transcription language".into());
                }
                result = Some(ConversationTranscript {
                    job_id: request.job_id.clone(),
                    text,
                    language: language.into(),
                    words,
                });
                Ok(())
            },
        )
        .await?;
        result.ok_or("Whisper returned no transcription".into())
    }

    pub fn cancel(&self, id: &str) -> Result<(), String> {
        validate_id(id)?;
        let mut cancelled = self
            .cancelled
            .lock()
            .map_err(|_| "Speech cancellation registry unavailable")?;
        cancelled.push_back(id.into());
        if cancelled.len() > 64 {
            cancelled.pop_front();
        }
        let jobs = self
            .jobs
            .lock()
            .map_err(|_| "Speech job registry unavailable")?;
        if let Some(cancel) = jobs.get(id) {
            cancel.cancel();
        }
        Ok(())
    }

    pub async fn stop(&self) {
        if let Ok(jobs) = self.jobs.lock() {
            for cancel in jobs.values() {
                cancel.cancel();
            }
        }
        if let Some(mut worker) = self.worker.lock().await.take() {
            let _ = worker.child.kill().await;
        }
    }
}

async fn send(input: &AsyncMutex<ChildStdin>, value: &Value) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    let mut input = input.lock().await;
    tokio::time::timeout(Duration::from_secs(5), input.write_all(&bytes))
        .await
        .map_err(|_| "CPU speech input stopped responding")?
        .map_err(|e| e.to_string())
}

fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
    {
        return Err("Invalid conversation speech identifier".into());
    }
    Ok(())
}

fn validate_timings(words: &[kestrel_app_core::SpeechTiming], duration: f64) -> Result<(), String> {
    let mut previous = 0.0;
    if words.len() > 1024 {
        return Err("Too many transcription word timings".into());
    }
    for word in words {
        if word.value.len() > 512
            || !word.start.is_finite()
            || !word.end.is_finite()
            || word.start < previous
            || word.end < word.start
            || word.end > duration + 0.1
        {
            return Err("Invalid transcription word timing".into());
        }
        previous = word.start;
    }
    Ok(())
}

fn validate_language(language: &str, auto: bool) -> Result<(), String> {
    if LANGUAGES.contains(&language) || (auto && language == "auto") {
        Ok(())
    } else {
        Err("Conversation speech supports English, French, German, Portuguese, Italian, Spanish and Dutch.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn worker_models(service: &ConversationVoice, install: &str) -> Value {
        let mut status = Value::Null;
        service
            .request(
                install,
                &uuid::Uuid::new_v4().to_string(),
                json!({"op":"status"}),
                |v| {
                    status = v;
                    Ok(())
                },
            )
            .await
            .unwrap();
        status
    }

    #[test]
    fn rejects_unbounded_or_invalid_transcription_timings() {
        let word = kestrel_app_core::SpeechTiming {
            value: "hello".into(),
            start: 0.0,
            end: 0.4,
        };
        assert!(validate_timings(std::slice::from_ref(&word), 1.0).is_ok());
        assert!(validate_timings(
            &[kestrel_app_core::SpeechTiming {
                end: f64::NAN,
                ..word.clone()
            }],
            1.0
        )
        .is_err());
        assert!(validate_timings(std::slice::from_ref(&word), 0.1).is_err());
        assert!(validate_timings(&vec![word; 1025], 1.0).is_err());
    }

    #[tokio::test]
    async fn cancelled_before_dispatch_never_launches_a_worker() {
        let service = ConversationVoice::new();
        service.cancel("late-request").unwrap();
        let result = service
            .request("unused", "late-request", json!({"op":"warm_stt"}), |_| {
                Ok(())
            })
            .await;
        assert!(result.unwrap_err().contains("cancelled before"));
        assert!(service.worker.lock().await.is_none());
    }

    #[tokio::test]
    #[ignore = "requires the installed CPU speech runtime at KESTREL_VOICE_TEST_ROOT; runs offline synthesis and transcription"]
    async fn live_resident_cpu_speech() {
        let install =
            std::env::var("KESTREL_VOICE_TEST_ROOT").expect("explicit installed test root");
        let service = ConversationVoice::new();
        let snapshot = service.prepare(&install, "en").await.unwrap();
        let pid = snapshot.process_id.expect("owned resident process");
        let initial = worker_models(&service, &install).await;
        let mut en_pcm_16k = Vec::new();
        for (language, text) in [
            (
                "en",
                "Hello, this is Kestrel. We can keep talking without changing your language model.",
            ),
            ("fr", "Bonjour, nous pouvons continuer notre conversation."),
            ("de", "Hallo, wir können unser Gespräch fortsetzen."),
            ("pt", "Olá, podemos continuar a nossa conversa."),
            ("it", "Ciao, possiamo continuare la nostra conversazione."),
            ("es", "Hola, podemos continuar nuestra conversación."),
            ("nl", "Hallo, we kunnen ons gesprek voortzetten."),
        ] {
            service.prepare(&install, language).await.unwrap();
            let selected = worker_models(&service, &install).await;
            assert_eq!(selected["models"].as_object().unwrap().len(), 1);
            assert!(selected["models"][language].is_number());
            assert_eq!(selected["stt"], initial["stt"]);
            service.prepare(&install, language).await.unwrap();
            assert_eq!(
                worker_models(&service, &install).await["models"],
                selected["models"]
            );
            if language != "en" {
                let stale = service
                    .request(
                        &install,
                        &uuid::Uuid::new_v4().to_string(),
                        json!({"op":"speak", "language":"en", "text":"Stale response."}),
                        |_| Ok(()),
                    )
                    .await;
                assert!(stale.unwrap_err().contains("language changed"));
                assert_eq!(
                    worker_models(&service, &install).await["models"],
                    selected["models"]
                );
            }
            let started = std::time::Instant::now();
            let mut first = None;
            let mut pcm = Vec::new();
            service
                .request(
                    &install,
                    &uuid::Uuid::new_v4().to_string(),
                    json!({"op":"speak","text":text,"language":language}),
                    |value| {
                        assert_eq!(value["kind"], "audio");
                        first.get_or_insert_with(|| started.elapsed());
                        pcm.extend(STANDARD.decode(value["pcm"].as_str().unwrap()).unwrap());
                        Ok(())
                    },
                )
                .await
                .unwrap();
            assert!(pcm.len() > 24000, "real generated audio");
            assert_eq!(service.snapshot(&install).await.process_id, Some(pid));
            eprintln!(
                "{language}: first audio {:?}, {} samples, complete {:?}, resident pid {pid}",
                first.unwrap(),
                pcm.len() / 2,
                started.elapsed()
            );
            if language == "en" || language == "nl" {
                // Test the actual English STT route on audio produced by the real English TTS
                // model, both initially and after cycling through every non-English TTS voice;
                // this is a pipeline and residency check, not an acoustic echo-cancellation claim.
                if language == "en" {
                    let samples: Vec<f32> = pcm
                        .chunks_exact(2)
                        .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32)
                        .collect();
                    for i in 0..samples.len() * 2 / 3 {
                        let at = i as f64 * 1.5;
                        let lo = at.floor() as usize;
                        let hi = (lo + 1).min(samples.len() - 1);
                        let value = samples[lo] + (samples[hi] - samples[lo]) * at.fract() as f32;
                        en_pcm_16k.extend_from_slice(&(value as i16).to_le_bytes());
                    }
                }
                let transcript = service
                    .transcribe(
                        &install,
                        ConversationTranscribeRequest {
                            job_id: uuid::Uuid::new_v4().to_string(),
                            pcm_base64: STANDARD.encode(&en_pcm_16k),
                            language: "en".into(),
                        },
                    )
                    .await
                    .unwrap();
                assert!(
                    transcript.text.to_ascii_lowercase().contains("kestrel"),
                    "unexpected English transcription: {}",
                    transcript.text
                );
                eprintln!("{language} stt check heard: {}", transcript.text);
            }
            assert_eq!(
                worker_models(&service, &install).await["models"],
                selected["models"]
            );
        }
        service.prepare(&install, "en").await.unwrap();
        let long_id = uuid::Uuid::new_v4().to_string();
        let mut chunks = 0;
        let result = service.request(&install, &long_id, json!({"op":"speak", "language":"en", "text":"This is a long answer that should be interrupted immediately when the producer starts speaking again. We must stop old audio and keep the model in memory for the next turn."}), |_| {
            chunks += 1;
            service.cancel(&long_id)?;
            Ok(())
        }).await;
        assert!(result.is_err());
        assert!(chunks > 0);
        assert_eq!(service.snapshot(&install).await.process_id, Some(pid));
        service
            .request(
                &install,
                &uuid::Uuid::new_v4().to_string(),
                json!({"op":"speak", "language":"en", "text":"I am still here."}),
                |_| Ok(()),
            )
            .await
            .unwrap();
        assert_eq!(service.snapshot(&install).await.process_id, Some(pid));
        service.stop().await;
        assert!(!service.snapshot(&install).await.running);
    }

    #[tokio::test]
    #[ignore = "opens the default Windows microphone and speaker, speaks a short local test and verifies cancellation/reopen; requires explicit speech installation"]
    async fn live_native_audio_cancels_and_reopens_without_reloading_models() {
        let install = std::env::var("KESTREL_VOICE_TEST_ROOT").expect("explicit speech installation");
        let voice = ConversationVoice::new();
        voice.prepare(&install, "en").await.unwrap();
        let identity = worker_models(&voice, &install).await;
        for _ in 0..2 {
            let capture_id = uuid::Uuid::new_v4().to_string();
            let (tx, rx) = tokio::sync::oneshot::channel();
            let mut listening = Some(tx);
            let capture = voice.request(&install, &capture_id,
                json!({"op":"listen","language":"en","threshold":-42.0,"silenceMs":600.0,"minimumMs":200.0}), |event| {
                    if event["kind"] == "listening" { if let Some(tx)=listening.take() { let _=tx.send(()); } }
                    assert_ne!(event["kind"], "capture_error"); Ok(())
                });
            let speaker = async {
                tokio::time::timeout(Duration::from_secs(15), rx).await.unwrap().unwrap();
                let result=voice.request(&install, &uuid::Uuid::new_v4().to_string(),
                    json!({"op":"speak","language":"en","text":"Native voice is ready.","nativePlayback":true,"audioSessionId":capture_id}), |_| Ok(())).await;
                // A real user interrupting the acoustic test is also valid; no automatic turn is submitted here.
                if let Err(error)=result { assert!(error.contains("cancelled"), "{error}"); }
                voice.cancel(&capture_id).unwrap();
            };
            let (capture,())=tokio::join!(capture,speaker);
            assert!(capture.unwrap_err().contains("cancelled"));
            assert_eq!(worker_models(&voice,&install).await["models"],identity["models"]);
        }
        voice.stop().await;
    }

    #[tokio::test]
    #[ignore = "requires KESTREL_VOICE_TEST_ROOT; kills only this test's owned speech worker to verify recovery"]
    async fn live_cpu_worker_crash_recovers_on_a_new_request_only() {
        let install =
            std::env::var("KESTREL_VOICE_TEST_ROOT").expect("explicit speech installation");
        let service = ConversationVoice::new();
        let old = service
            .prepare(&install, "en")
            .await
            .unwrap()
            .process_id
            .unwrap();
        let job = uuid::Uuid::new_v4().to_string();
        let speech = service.request(&install, &job, json!({"op":"speak", "language":"en", "text":"This answer is interrupted by a controlled worker crash. It must never replay automatically."}), |_| Ok(()));
        let crash = async {
            while !service.jobs.lock().unwrap().contains_key(&job) {
                tokio::task::yield_now().await;
            }
            service
                .worker
                .lock()
                .await
                .as_mut()
                .unwrap()
                .child
                .kill()
                .await
                .unwrap();
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(15), async {
            tokio::join!(speech, crash)
        })
        .await
        .unwrap();
        assert!(result.is_err());
        assert!(!service.snapshot(&install).await.running);
        assert!(service.jobs.lock().unwrap().is_empty());
        // Merely observing a dead worker must not restart it or replay the failed text.
        assert!(!service.snapshot(&install).await.running);
        let next = service
            .prepare(&install, "en")
            .await
            .unwrap()
            .process_id
            .unwrap();
        assert_ne!(old, next);
        service
            .request(
                &install,
                &uuid::Uuid::new_v4().to_string(),
                json!({"op":"speak", "language":"en", "text":"This is a new request."}),
                |_| Ok(()),
            )
            .await
            .unwrap();
        service.stop().await;
    }

    #[tokio::test]
    #[ignore = "loads the explicitly selected installed Strata LLM and resident speech; requires KESTREL_VOICE_TEST_ROOT and KESTREL_LIVE_LIBRARY"]
    async fn live_voice_coexists_with_selected_strata() {
        use crate::{
            config::ControlSettingsStore,
            runtime::{authorized, RuntimeManager},
        };
        let install =
            std::env::var("KESTREL_VOICE_TEST_ROOT").expect("explicit speech installation");
        let library = PathBuf::from(
            std::env::var("KESTREL_LIVE_LIBRARY").expect("explicit settings library"),
        );
        let settings = ControlSettingsStore::new(&library).load().unwrap();
        let selected = settings
            .selected_model_id
            .as_ref()
            .expect("user-selected model");
        let configured = std::env::var("KESTREL_LIVE_STRATA_ROOT").unwrap_or_default();
        let models = crate::strata::discover(&crate::strata::known_installs(&[&configured]));
        let model = models
            .iter()
            .find(|m| &m.id == selected)
            .expect("the selected model must be the installed Strata model");
        let effective = settings.for_model(selected);
        let runtime = Arc::new(RuntimeManager::new());
        let voice = ConversationVoice::new();
        let voice_pid = voice.prepare(&install, "en").await.unwrap().process_id;
        let mut model_identity = Value::Null;
        voice
            .request(
                &install,
                &uuid::Uuid::new_v4().to_string(),
                json!({"op":"status"}),
                |v| {
                    model_identity = v;
                    Ok(())
                },
            )
            .await
            .unwrap();
        assert!(model_identity["cuda"].is_null());
        assert_eq!(model_identity["models"].as_object().unwrap().len(), 1);
        assert!(model_identity["models"]["en"].is_number());
        let mut model_pid = None;
        for round in 1..=2 {
            let lease = runtime
                .lease_model(selected, &models, &settings, None)
                .await
                .unwrap();
            let before = runtime.snapshot().await;
            if let Some(pid) = model_pid {
                assert_eq!(before.pid, Some(pid));
            }
            model_pid = before.pid;
            assert_eq!(before.model_id.as_ref(), Some(selected));
            let (tx, rx) = tokio::sync::oneshot::channel::<String>();
            let mut first_clause = Some(tx);
            let mut visible = String::new();
            let start = std::time::Instant::now();
            let model_reply = async {
                let client = reqwest::Client::builder().no_proxy().build().unwrap();
                let level = effective.thinking_level;
                let body = json!({"model":lease.connection.model_id,
                    "messages":[{"role":"user","content":"Explain a peaceful walk beside a lake in twelve short spoken sentences. Begin directly with the first sentence, without a heading or list. This is a local speech streaming acceptance check."}],
                    "stream":true,"temperature":0.2,"max_tokens":effective.max_output_tokens,
                    "reasoning_effort":level.as_str(),"reasoning_budget_tokens":level.budget_tokens(effective.max_output_tokens.saturating_sub(512)),
                    "chat_template_kwargs":{"enable_thinking":!level.is_off(),"reasoning_effort":level.as_template_effort()}});
                let response = authorized(
                    client.post(format!("{}/chat/completions", lease.connection.endpoint)),
                    &lease.connection,
                )
                .json(&body)
                .send()
                .await
                .unwrap();
                assert!(response.status().is_success());
                let result = crate::agent::response::read(response, |delta| {
                    visible.push_str(delta);
                    if visible.len() > 25 {
                        // Match the UI's sentence/newline boundaries, including replies that
                        // put each spoken sentence on its own line.
                        if let Some((end, _)) = visible.char_indices().find(|(at, ch)| {
                            *ch == '\n'
                                || (".!?;".contains(*ch)
                                    && visible[at + ch.len_utf8()..]
                                        .starts_with(char::is_whitespace))
                        }) {
                            if let Some(tx) = first_clause.take() {
                                let _ = tx.send(visible[..=end].into());
                            }
                        }
                    }
                })
                .await
                .unwrap();
                let completed = start.elapsed();
                if first_clause.is_some() {
                    eprintln!("No streamable clause in visible reply: {visible}");
                }
                drop(first_clause.take());
                assert!(
                    result["choices"][0]["message"]["content"]
                        .as_str()
                        .unwrap()
                        .len()
                        > 80
                );
                completed
            };
            let speech = async {
                let text = rx.await.expect("a clause before the reply completes");
                let submitted = start.elapsed();
                let mut first = None;
                voice
                    .request(
                        &install,
                        &uuid::Uuid::new_v4().to_string(),
                        json!({"op":"speak","language":"en","text":text}),
                        |v| {
                            assert_eq!(v["kind"], "audio");
                            first.get_or_insert_with(|| start.elapsed());
                            Ok(())
                        },
                    )
                    .await
                    .unwrap();
                (submitted, first.expect("real streaming PCM"))
            };
            let (completed, (submitted, audio)) = tokio::join!(model_reply, speech);
            assert!(
                audio < completed,
                "speech must begin before the selected LLM finishes"
            );
            assert_eq!(runtime.snapshot().await.pid, model_pid);
            assert_eq!(voice.snapshot(&install).await.process_id, voice_pid);
            voice
                .request(
                    &install,
                    &uuid::Uuid::new_v4().to_string(),
                    json!({"op":"status"}),
                    |v| {
                        assert_eq!(v["models"], model_identity["models"]);
                        assert_eq!(v["stt"], model_identity["stt"]);
                        assert_eq!(v["devices"].as_object().unwrap().len(), 1);
                        assert_eq!(v["devices"]["en"], "cpu");
                        Ok(())
                    },
                )
                .await
                .unwrap();
            eprintln!("COEXISTENCE round={round} model={} LLM_PID={model_pid:?} CPU_PID={voice_pid:?} first_clause={submitted:?} first_audio={audio:?} reply_complete={completed:?}", model.name);
            drop(lease);
        }
        voice.stop().await;
        runtime.stop_managed().await.unwrap();
    }
}
