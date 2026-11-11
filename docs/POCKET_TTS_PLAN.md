# Resident voice conversation

Implemented for review, 10 November 2026. This records the requested scope, the final runtime
choice, installation and acceptance evidence. The [Sonder H3 plan](SONDER_H3_IMPLEMENTATION_PLAN.md)
remains separate future work.

## Behavior and installation

Prepare **Voice conversation** in the existing Setup page, then allow microphone access when
Windows/WebView asks. Setup installs an isolated CPU runtime under
`<install root>/ConversationVoice`; it does not replace the existing speech installation.
The immutable model/voice/tokenizer manifest contains 25 assets, totaling 1,722,439,550 bytes.
Python, dependency wheels and installation caches require additional disk space.
No account or gated voice-cloning download is needed for the supplied voices.

Setup preserves unchanged saved selections from retired model manifests, including MiniMax
Music 3 selections saved before the move to YuE2. Only current manifest assets enter an install
request. Old music selections therefore cannot block installing conversation voice, and no
saved model paths or source files need to be deleted to proceed.

Conversation is enabled by default in the visible Chat and Computer Tasks workspace. Speak,
pause, and hear the selected LLM's reply as its visible text arrives. A subsequent utterance
uses the same native chat or producer-direction command as typing. This includes clarification
answers and a spoken "continue". Opening a saved interrupted task never resumes it by itself.
The selected model ID and its thinking settings are unchanged.

The existing microphone, Stop, VAD settings, transcript, Listen and export controls remain in
their existing layout. The microphone toggles conversation mute; mute is saved natively.
Shift-click the microphone for the existing manual large-Whisper dictation. Existing Listen,
voice-library, narration checks and exports continue using the existing manual speech route.
The conversation pause setting defaults to 600 ms, with a 200 ms minimum speech duration;
its native preferences are independent of the user's manual dictation settings.

English, French, German, Portuguese, Italian, Spanish and Dutch are supported. A single
**Conversation language** selector in the existing microphone settings chooses both the Pocket
model and Whisper's language hint (English by default). The preference is stored natively.
Exactly one Pocket model is retained: switching language cancels old playback, drops the old
model and its conditioning states, and prepares the selection in the same CPU worker. Repeated
turns in that language reuse the model. The one multilingual base Whisper model stays loaded
across language changes; there is no separate Whisper model per language. All seven Pocket
models remain installed on disk. Old queued speech cannot reload a previous language after a
switch. The normal conversation route always uses the supplied voice. Custom
voice selection remains available through the existing manual TTS controls.

## Architecture and runtime decision

The upstream source reviewed is
[Pocket TTS c91f50f](https://github.com/kyutai-labs/pocket-tts/tree/c91f50f38c6b914d49dbdff51b8d322edf843db2).
The implementation uses the official **Python Pocket TTS 4.0.1** streaming API in one private
CPU process together with faster-whisper. It retains the selected model and voice state and
supports cooperative cancellation without a separate HTTP server. The initial design allowed
seven resident models; the producer refined this to one selected model. Keeping the private
worker also preserves the already tested shared STT lifecycle and bounded pipe protocol.

The private environment pins Python 3.12.12, CPU-only PyTorch 2.10.0, faster-whisper 1.2.1 and
PyYAML 6.0.3. All transitive wheels are hash-locked. Model assets use immutable revisions,
SHA-256 and expected sizes. Upstream language configurations and the MIT license are included.
Setup writes local paths into those configurations. Downloads only happen through an explicit
Setup operation; normal inference disables Hugging Face networking and socket connections.

`conversation_voice.rs` owns the child inside a Windows job object, bounded JSON pipes,
cancellation and independently serialized TTS/STT work. There is no HTTP listener. This process
never acquires or alters the language-model inference lease and never calls ComfyUI cleanup.
`RuntimeManager` still owns the sole LLM process and inference gate. CPU speech remains loaded
when the microphone is muted or another page opens; shutdown terminates the owned process tree.

Chat remains tool-free. Computer Task responses now use bounded SSE assembly so public content
can be spoken early. Reasoning and tool JSON are excluded from speech. Proposed tools only reach
the existing native validator after a complete response and confirmed stream termination.
Direction revisions, mutation receipts, recovery and initial Full Access opt-ins remain native.
Streaming prose is provisional presentation, never evidence that a task action succeeded.

The browser owns only microphone/playback handles and disposable audio queues. Web Audio capture
requests echo cancellation, noise suppression and automatic gain control. A short pre-roll and
bounded utterance detector commit one final STT result; partial text never initiates tools.
Speech onset cancels old playback and queued clauses while keeping the microphone live.
Audio frames carry a request identity and sequence; stale/reordered frames are discarded or
reported. The input queue is bounded, and navigating away cancels pending transcription.
The microphone worklet is explicitly emitted as a separate same-origin JavaScript asset.
Vite's default inlining of this small script caused a desktop-only `data:` URL loading failure
under the existing CSP. The production build now checks the emitted file and its bundle
reference; capture failures produce a concise retry message instead of encoded script text.

## Compact Whisper and alignment

The resident recognizer is **multilingual Whisper base, CPU INT8**, not the larger model named
`small` and not English-only Whisper. It returns bounded, validated word timestamps with the
committed transcription. These are recognition estimates, not exact forced alignment.

Large Whisper is not required for word highlighting. The future path to replacing highlighted
Listen playback is to align short synthesized clauses against their known spoken text with
the resident base model, validate word coverage/timing, and keep microphone transcription at
higher priority. Late or unreliable timing must never delay the first audio or be presented as
precise alignment. This is a separate feature decision, not a replacement of manual TTS in
this PR. Base accuracy needs real-language evaluation before promising parity with large.
English-only `base.en` can improve English recognition, but one multilingual base supports all
seven choices without another model swap. See [Whisper model documentation](https://github.com/openai/whisper#available-models-and-languages)
and [faster-whisper word timestamps](https://github.com/SYSTRAN/faster-whisper#word-level-timestamps).

The producer's machine reports 67,737,395,200 bytes of physical RAM (about 63.1 GiB usable)
and a 12,227 MiB RTX 5070. During the final selected-Strata coexistence run, a point-in-time
sample showed 4,603 MiB free VRAM and 716,804 KiB free physical RAM (about 0.68 GiB). This is
tight system-memory headroom, not evidence that large Whisper can remain loaded comfortably.
The standard Whisper implementation estimates
roughly 10 GB VRAM for large; the [faster-whisper large-v2 benchmark](https://github.com/SYSTRAN/faster-whisper#large-v2-model-on-gpu)
reports 2,926 MB with INT8. That different checkpoint/runtime measurement suggests a GPU
variant might fit, but does not prove coexistence with this selected Strata configuration or
the existing manual Whisper installation. Keeping large resident would need a separate
coexistence and latency check. The default remains base INT8 on CPU so speech
does not consume Strata's GPU headroom or require ordinary model unloads.

The scope was refined to keep automatic speech on the fastest path: this PR does not run an
extra transcription pass over each generated Pocket clause or add word highlighting to live
audio. That pass would compete with incoming speech and has no consumer in the unchanged UI.
Large-Whisper narration checking/alignment, saved audio, timing sidecars and exports remain
available through their existing manual controls. Conversation PCM is ephemeral and never
overwrites the existing cached speech or claims export-quality alignment.

The native speech API also retains a local-reference conditioning path. It explicitly rejects
recorded-voice conditioning with the supplied public checkpoint. An advanced installation may
provide authorized matching `models/<language>/voice-cloning.safetensors` before worker startup;
this optional checkpoint is not downloaded, hash-certified or exercised by the standard setup.
It is not needed for the automatic conversation described above.

## Failure and recovery

- Empty/low-confidence recognition creates no turn. A transcription failure remains visible;
  the next utterance can be tried without restarting the selected LLM.
- Audio cancellation retains models and voice states. Stop/mute prevents more automatic turns.
- Worker EOF, protocol failures, timeouts and bounded queues report actionable errors. A dead
  worker starts again only on a new request; failed speech is never replayed automatically.
- A speech failure never silently starts the heavy TTS/Whisper path or changes the selected LLM.
- First microphone permission or a WebView autoplay restriction can require the existing mic
  button once. Denied permission is not retried in a loop.
- Recognition can be wrong, particularly for short or noisy multilingual utterances. Accepted
  words remain visible in the native durable transcript and can be corrected by the next turn.

## Acceptance evidence

An explicit fresh installation completed in a workspace-local acceptance directory. It did not
modify the producer's existing `D:/AI` installation. The installer verified the pinned assets
and CPU-only dependency probe.

The live resident test generated actual audio and transcribed it in all seven languages, then
cancelled generation and successfully spoke again with the same process. Initial measured
first PCM was about 138 ms for warm English and 566–638 ms for first-use other languages.
Those measurements exclude microphone endpointing and are not promises for other hardware.
Recognition was nonempty in every language but contained mistakes in French and Dutch.

The selected-model coexistence test used the producer's saved **Qwen3.8-Flash-Next GSQ-RCO
IQ3_S (Strata)** model and saved thinking level. In its first two-turn run, LLM and speech process
identities stayed fixed, Pocket and STT objects stayed resident, and audio started before each
LLM response completed. On the second turn, the first complete clause arrived at 3.648 s,
audio at 3.803 s and response completion at 4.602 s: about 156 ms speech overhead after the
clause. The first turn needed 813 ms speech overhead. Model thinking time remains visible.
Before the one-language refinement, an extended test passed with all seven Pocket models resident beside Strata. Its second turn
produced a clause at 4.636 s, first audio at 4.771 s and response completion at 5.208 s (135 ms
speech overhead). All seven model identities and the STT identity stayed fixed. The main speech
process was observed at approximately 2.0 GiB working set and 3.0 GiB private memory during
coexistence. These historical measurements are not the new single-model memory claim.
The updated tests require exactly one Pocket model, unchanged STT identity across all seven
user selections, reuse on same-language preparation, and rejection of stale speech in a previous
language. The coexistence test now retains only the chosen English Pocket model beside Strata.
The test recognizes sentence/newline boundaries as the UI does.

The final single-language run passed all three live tests, including controlled crash recovery.
All seven selections generated and transcribed real audio with exactly one Pocket model at a
time, stable STT identity, same-language model reuse and rejection of stale-language speech.
First PCM after each selected model was prepared ranged from 82 to 109 ms in that standalone
test; language preparation time is excluded. French and Dutch still had recognition errors.
With the saved Strata model, turn one produced a clause at 6.006 s, first PCM at 7.151 s and
completion at 7.243 s. Turn two produced a clause at 3.189 s, first PCM at 3.345 s and completion
at 4.062 s (155 ms speech overhead). The LLM, selected Pocket and base Whisper identities were
unchanged across both turns. These timings exclude microphone endpointing and do not imply
phone-call latency before the LLM produces a speakable clause.

Automated coverage includes clause boundaries/final flushing, hidden reasoning/code, bounded
VAD and PCM, consecutive final utterances, speech during warm-up, task clarification deduplication,
interruption/stale audio, navigation during STT, native preference migration/mute, validated
word timings, truncated tool streams, cancellation before dispatch, worker-crash recovery and
process cleanup. Worker-crash recovery and the seven-language pipeline passed again with the
final recovery/word-timing validation. The latest standalone run had 94 ms first English PCM
and 897–936 ms for first-use other languages, demonstrating startup/contention variability.
The live tests are opt-in and use explicit local installation paths:

```powershell
$env:KESTREL_VOICE_TEST_ROOT = '<test install root>'
cargo test --manifest-path src-tauri/Cargo.toml live_resident_cpu_speech -- --ignored --nocapture
cargo test --manifest-path src-tauri/Cargo.toml live_cpu_worker_crash -- --ignored --nocapture
$env:KESTREL_LIVE_LIBRARY = '<existing Kestrel library>'
$env:KESTREL_LIVE_STRATA_ROOT = '<existing Strata installation>'
cargo test --manifest-path src-tauri/Cargo.toml live_voice_coexists -- --ignored --nocapture
```

Repository verification: `git diff --check`, all-target native tests (318 passed, 28 optional
live tests ignored), Clippy with `-D warnings`, `npm run check` (including 307 core binding tests),
all 252 frontend tests and `npm run build`. Chat and Computer Tasks were also visually
checked in the browser preview with the existing book layout and composer controls. The new
seven-option selector was visually checked inside the existing microphone settings dialog.
After the packaged-worklet fix, the optimized desktop executable was rebuilt and opened with
the producer's installed `D:/AI/ConversationVoice` runtime. Chat reached "Listening locally"
in the actual Windows WebView without the earlier worklet error or a model reinstall.

Physical speaker/headset echo rejection, room noise, double-talk and perceived endpoint latency
still need the producer's device review. Browser AEC constraints and synthetic mic tests cannot
prove acoustic performance. Review with speakers and a headset, make a short correction during
playback, change language, answer a task question and say "continue" after a pause. The default
600 ms endpoint is adjustable through the existing VAD settings. No telemetry or background
benchmarking is installed.
