# Resident voice conversation

Implemented for review, 14 November 2026. This records the producer's requested behavior,
the Rust migration and acceptance evidence. The [Sonder H3 plan](SONDER_H3_IMPLEMENTATION_PLAN.md)
remains separate future work.

## Behavior and installation

Prepare **Voice conversation** in the existing Setup page. The executable includes the native
speech engine; Setup downloads only hash-verified models, tokenizers and supplied voices under
`<install root>/ConversationVoice`. The 24 pinned assets total 4,632,504,441 bytes. Existing
Pocket assets are reused. An earlier Python, `whisper-base-native`, or `whisper-small-native` installation needs the
`whisper-medium-en-native` files and updated readiness revision; unused older environments and models are left on disk.
No account, Python, PyTorch, separate speech server or gated cloning download is required.
Windows must permit desktop applications to use the microphone.

Unchanged retired model selections, including old MiniMax Music 3 selections, are preserved
without entering a new install request. They cannot block conversation Setup.

Conversation is enabled by default in the visible Chat and Computer Tasks workspace. Speak,
pause, and hear the selected LLM's public reply as complete clauses arrive. Further utterances
use the same native chat or producer-direction commands as typing, including clarification
answers and a spoken "continue". Opening an interrupted saved task never resumes it by itself.
The selected LLM and its thinking settings remain unchanged.

The existing book layout, microphone, Stop, transcript, Listen and export controls remain.
The microphone toggles conversation mute; Shift-click retains manual large-Whisper dictation.
The existing microphone settings contain the one Conversation language selector. The seven
choices are English, French, German, Portuguese, Italian, Spanish and Dutch. The selected
language chooses PocketTTS weights and supplied voice, keeping one Pocket model resident;
language changes cancel old speech and replace that model. One English-only `whisper-medium.en`
recognizer remains resident across all turns to maximize accuracy on non-native English speech
while remaining fast enough for live conversation.

Conversation endpointing defaults to 600 ms silence after at least 200 ms speech, with 200 ms
pre-roll and a 30-second utterance cap. Its saved settings are independent of manual dictation.
Speech onset cancels current playback and queued clauses. Manual Listen temporarily pauses
conversation capture so its WebView audio cannot become a user turn; capture resumes when
manual playback releases ownership. Existing manual voices, narration checks, highlighted
Listen playback, timing sidecars and exports retain their existing route.

## Rust architecture

`crates/conversation-engine` embeds the official Pocket Rust inference adapter with local JSON
tokenizers and supplied voice conditioning. See its [provenance and licenses](../crates/conversation-engine/README.md).
Candle runs English-only **Whisper medium.en (769M)** via streaming safetensors loading
(`SafetensorsReader`) with a hybrid precision layout: `F32` encoder and cross-attention `K/V`,
`F16` token embedding lookup, `Q8_0` (`QMatMul`) decoder projections/logits, self and cross KV
caching, and a centered reflect-padded `MelExtractor` with dynamic even framing (`1500–3000`
frames). PocketTTS pins `xn`'s threadpool to `workers: 1`, while Candle Whisper uses a bounded
Rayon pool (`(available_parallelism / 4).clamp(2, 8)` — 8 threads on a 32-logical-CPU host). CPAL owns native microphone and speaker
streams; Sonora performs AEC3 echo cancellation, noise suppression and high-pass filtering.
Actual output callback samples, including underruns, feed the echo reference. Streaming
resampling preserves phase between device packets. Bounded native queues cover device frames,
utterances, latent synthesis and playback. No conversation AudioWorklet, Web Audio capture or
PCM playback queue remains in the WebView, removing the reported worklet loading failure.

The application launches its own executable with `--conversation-voice-worker`, before Tauri
initialization, inside the existing owned Windows job object. Bounded JSON pipes carry requests,
status and final text. Separate TTS/STT executors retain one model each. There is no HTTP
listener or runtime downloader. Only explicit Setup can fetch the pinned assets. CPU speech
does not acquire the LLM inference lease, unload Strata or invoke ComfyUI memory cleanup.
RuntimeManager continues to own the sole LLM runtime and inference gate. Muting and navigation
close audio devices while retaining the CPU models; application shutdown terminates its worker.

Rust owns settings, installation, device processing, model lifetime and IPC contracts. The view
assembles public clauses and displays native lifecycle/final-transcript events. Capture session
identities reject late text after navigation, mute or language changes. Only final recognition
can submit a turn. Reasoning and tool JSON are excluded from speech. Chat remains tool-free;
Computer Task tools still require complete validated native responses and the existing access
policy. Provisional streamed speech is never a mutation receipt or evidence of success.

## Whisper and alignment

The resident recognizer is English-only **Whisper medium.en (769M)** (`openai/whisper-medium.en`
@ `2e98eb6279edf5095af0c8dedb36bdec0acd172b`), loaded directly from `model.safetensors` into a
hybrid precision runtime: **`F32`** `AudioEncoder` and decoder cross-attention `K`/`V` projections
(processed once per utterance via AVX2 `gemm`), **`F16`** `embed_tokens` lookup, and **`Q8_0`**
(`QMatMul`) for `logits_proj`, decoder self-attention, cross-attention `Q`/`Out`, and decoder MLPs,
with both self-attention and cross-attention KV caching and a centered reflect-padded `MelExtractor`
(`1500`–`3000` even frames). This replaces the earlier `whisper-base`, `whisper-small`, and
faster-whisper INT8 workers. The current Candle decoder returns text with an empty word-timing list;
it does not implement timestamp alignment or pretend that estimated timings are exact. There is no
extra recognition pass over generated Pocket clauses on the live path.

Candidate models were evaluated against a real `24.30 s` non-native (Finnish-accented) English
microphone recording (`"Tallenne (16).m4a"`, expected: *"Testing a real-life test for microphone
input for different whisper models on this computer in non-native English accent."*) and live
Strata (`Qwen3.8-Flash-Next GSQ-RCO IQ3_S`) memory coexistence:
- `whisper-small`, `whisper-small.en`, `distil-small.en`, `distil-medium.en`, `distil-large-v3`,
  `distil-large-v3.5`, and `whisper-large-v3-turbo` (F32 & Q4_K) all misheard `"whisper models"` or
  `"on this computer"` as `"FISPR"`, `"phase per"`, `"face pair"`, `"visible"`, `"Fisper"`, or `"his computer"`.
- CPU F32 `whisper-large-v3-turbo` (`3.75 GiB` steady / `4.43 GiB` peak private) squeezed free
  physical RAM down to `0.01 GiB` alongside Strata (`minFreeVirt = 0.27 GiB`), and Q4_K
  `whisper-large-v3-turbo` took `18.32 s` warm CPU time on `"Tallenne (16).m4a"`.
- **`openai/whisper-medium.en`** (`Q8_0` decoder + `F32` encoder, `1.85–1.86 GiB` steady worker
  private / `2.25 GiB` peak load private; `2.25–2.56 GiB` with PocketTTS resident) was the only
  tested model to transcribe `"Tallenne (16).m4a"` verbatim while coexisting with Strata. Existing
  manual large-Whisper checks and highlighting remain available when invoked.

## Failure and recovery

- Silence and low-confidence recognition create no turn. Recognition errors remain possible;
  accepted text is visible in the durable chat/task transcript.
- Cancellation preserves models. Native speech onset clears the speaker queue immediately.
  Mute/navigation invalidate pending recognition and prevent additional automatic turns.
- Device failure, overflow, worker EOF and protocol/time limits produce concise actionable
  errors. Retry can restart the owned worker without restarting the selected LLM. A failed
  utterance or interrupted Computer Task is never automatically replayed.
- A speech failure never silently selects another LLM or invokes the heavy manual speech path.
- Manual microphone/device access remains subject to Windows permissions. Permission failures
  do not cause a retry loop.

## Acceptance evidence

The 24 pinned `whisper-medium.en` + PocketTTS assets in `assets.json` total `4,632,504,441` bytes
under the native readiness revision `pocket-rust-9e8413e-whisper-medium-en-candle-v1`. With
`+avx2,+fma,+f16c` enabled in `.cargo/config.toml` and release `worker.exe` (bounded 8-thread Rayon
pool / `xn` `workers: 1`), direct recognizer verification on the `24.30 s` Finnish-accented English
microphone sample (`"Tallenne (16).m4a"`) measured:
- Cold `warm_stt` load: `2.00 s` (`1.85 GiB` steady private/working set, `2.25 GiB` peak during
  streaming load & `Q8_0` quantization).
- Full `24.30 s` clip: `7.20–7.28 s` warm CPU transcription producing exact verbatim text
  (*"Testing a real-life test for microphone input for different whisper models on this computer in
  non-native English accent."*).
- Short conversational slices (zero repetition): `0.0–6.2 s` in `3.46 s` (*"Testing a real life test
  for"*), `1.5–6.2 s` (`4.7 s` active speech) in `3.20 s` (*"testing a real life test for"*), and
  `0.0–8.8 s` in `2.87 s` (*"Testing a real-life test for microphone input."*), holding `1.86 GiB`
  steady private/working set.

`live_resident_cpu_speech` passed in `16.27 s` across all seven PocketTTS voices with one worker
(first PCM `78.3–119.5 ms` after clause receipt, peaking at `2.56 GiB` working set / `2.48 GiB`
private with PocketTTS + `whisper-medium.en` resident), transcribing the generated English clause
verbatim before and after cycling through all non-English voice models.

The standalone **Qwen3.8-Flash-Next GSQ-RCO IQ3_S (Strata)** coexistence test
(`live_voice_coexists_with_selected_strata`) passed in `60.92 s` with fixed LLM and speech
worker process identities (`PID 21304` / `PID 24252`, sampled `peakWorkerPriv = 2.25 GiB`,
`peakStrataPriv = 61.86 GiB`, `minFreePhys = 0.03 GiB`, `minFreeVirt = 0.53 GiB` during Strata's
startup/prefill before pagefile settling). Round one produced a clause at `7.71 s`, first PCM at
`8.14 s`, and completion at `8.81 s`; round two produced a clause at `3.84 s`, first PCM at
`4.03 s`, and completion at `4.87 s`. The selected thinking settings remain active; phone-call
responsiveness still depends on when the LLM emits public text.

Controlled worker-crash recovery (`1.60 s`) and native microphone/speaker cancel/reopen (`4.01 s`)
both passed with retained model identities. Synthetic delayed speaker audio was suppressed by more
than 10 dB while a second near voice survived double-talk; physical speaker/headset echo, room
noise and perceived endpoint latency still require device-specific listening review, as synthetic
tests cannot prove those conditions. No telemetry or background benchmarking is installed.

Automated coverage includes native endpoint/re-arm and short-noise handling, resampling phase,
30-second bounds, offline asset paths, clause splitting, reasoning exclusion, interruption,
manual Listen coordination, warm-up utterances, stale sessions, task clarification/continue,
native preferences, truncated tool streams, cancellation, crash recovery and process ownership.

Required repository checks are `git diff --check`, all-target app tests and Clippy with
`-D warnings`, `npm run check`, `npm test -- --run`, and `npm run build`. Also run the new
engine's tests/Clippy. Explicit live acceptance commands:

```powershell
$env:KESTREL_VOICE_TEST_ROOT = '<install directory containing ConversationVoice>'
# Optional during development: same engine entry point in a small optimized executable.
# cargo build -p kestrel-conversation-engine --release --example worker
# $env:KESTREL_VOICE_WORKER_EXE = '<absolute path to release/examples/worker.exe>'
cargo test --manifest-path src-tauri/Cargo.toml live_resident_cpu_speech -- --ignored --nocapture
cargo test --manifest-path src-tauri/Cargo.toml live_cpu_worker_crash -- --ignored --nocapture
# Opens the default microphone and audibly plays two brief test clauses:
cargo test --manifest-path src-tauri/Cargo.toml live_native_audio -- --ignored --nocapture
$env:KESTREL_LIVE_LIBRARY = '<existing Kestrel library>'
$env:KESTREL_LIVE_STRATA_ROOT = '<existing Strata installation>'
cargo test --manifest-path src-tauri/Cargo.toml live_voice_coexists -- --ignored --nocapture
```
