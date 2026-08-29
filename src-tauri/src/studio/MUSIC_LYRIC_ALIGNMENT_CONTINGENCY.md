# Music lyric isolation and alignment contingency

Status: **do not replace the current path without a repeatable acceptance failure.** Brief producer
checks in August 2026 exposed a cold-start weakness in the first several seconds while the existing
path otherwise remained useful. Kestrel now hardens that opening without adding another runtime or
model download. This observation is not a benchmark and does not justify a model comparison or a
new maintenance surface.

This note records the preferred fallback so a future maintainer does not have to restart the model
and licensing investigation if dense arrangements, new languages, or a changed generator cause a
repeatable regression.

## Current cold-start hardening

Music lyric sync uses the `music-repeat` mode of Kestrel's owned Whisper adapter. It converts the
immutable take to mono 16 kHz once and transcribes `take + one second of silence + take`. The first
copy gives Whisper decoded lyric context before it encounters the opening again; the silence makes
that second opening an explicit song boundary. This is text-context conditioning, not persistent
learning of the first copy's audio features.

The initial prompt is not the complete lyric sheet. Kestrel removes bracketed section headings and
keeps at most the first four lyric lines and 512 UTF-8 bytes. The adapter carries that bounded
opening excerpt while preserving room for Whisper's recent decoded text. Ordinary dictation and
spoken-audio alignment continue to use the single-copy path.

The adapter returns the source duration, seam, and second-copy interval as typed metadata. Rust
rejects a missing, mismatched, non-finite, or out-of-range boundary and independently extracts both
copies into original-take time. It scores their ordered token coverage against the authoritative
generated lyrics, including a separately weighted opening window. Copy two is selected only when
its score is materially higher; otherwise the stable first copy wins. This guard is necessary
because recent tail lyrics can occasionally make a context-conditioned second opening jump ahead
to a chorus. Boundary-crossing cues are rebuilt from only in-range words. The durable lyric-sync
receipt records the strategy, copy count, selected copy, both candidate scores, and seam. Scoring is
bounded to 1,024 tokens per side. Generated music is limited to five minutes; the adapter allows a
30-second container margin and fails explicitly above that bound rather than making an unbounded
duplicate allocation.

This pass costs roughly twice the Whisper time of a single transcription. If it does not recover a
repeatable dense opening, do not stack more heuristic ASR passes. Use the activation gate below to
evaluate isolated-vocal forced alignment against the authoritative generated lyrics.

## Activation gate

Consider the fallback only when a checked local acceptance set demonstrates one or more of these:

- accompaniment is repeatedly transcribed as invented lyric text;
- word boundaries are materially wrong after the existing bounded Whisper pass;
- the same failure reproduces across at least three completed immutable takes; or
- a supported model/runtime update makes the fallback simpler than the current ComfyUI adapter.

Do not change the pipeline because one expressive vocal needs a manual cue adjustment. Do not add a
remote fallback, background download, model leaderboard, or second unmanaged service. Setup must
make every weight download explicit and allowlisted; production must remain fully offline.

## Preferred pipeline

```text
immutable stereo master
  -> verify the take SHA-256 again
  -> BS-RoFormer ep368 Q8: vocals.wav + instrumental.wav
  -> align Kestrel's authoritative generated lyrics to vocals.wav
  -> validate monotonic bounded word times
  -> preserve the stem, exact model identity, hashes, timings, and receipt
```

The first implementation candidate is **BS-RoFormer ep368 Q8 through audio.cpp**. audio.cpp exposes
a native, argument-array CLI for 44.1 kHz mono or stereo separation and writes explicit vocal and
instrumental stems. Its published GGUF package is small enough for the supported production GPU,
has a tested Q8 path, and lists the original model license as Apache-2.0. Pin an audited audio.cpp
revision, the exact model URL, byte length, SHA-256, license text, and expected output names before
adding it to Setup.

- Runtime and invocation: <https://github.com/0xShug0/audio.cpp/blob/main/docs/audio_tools.md#bs-roformer>
- Published GGUF package and license table: <https://huggingface.co/audio-cpp/audio.cpp-gguf>
- Native runtime source: <https://github.com/0xShug0/audio.cpp>

**HTDemucs** is the compatibility fallback, not the first choice. Its official implementation and
weights have a mature two-stem vocal route and an MIT license, but the RoFormer route is the better
quality-oriented starting point for lyric recovery.

- Official Demucs source and model descriptions: <https://github.com/facebookresearch/demucs>

## Text authority and alignment

For Kestrel-generated songs, the take's generated lyrics remain authoritative. A recognizer may
propose timings, but its decoded words must never silently replace the producer's lyric text. This
is the most important rule in this contingency: isolation solves signal quality; forced alignment
solves timing; neither receives authorship.

For English, Chinese, Cantonese, French, German, Italian, Japanese, Korean, Portuguese, Russian, and
Spanish, the preferred aligner candidate is **Qwen3-ForcedAligner-0.6B**. It accepts known text and
audio, returns word or character timestamps, supports inputs up to five minutes, and is Apache-2.0.
Its official evaluation reports lower timestamp shift than WhisperX. The model card currently lists
speech rather than singing as the aligner's declared audio type, so Kestrel must run an isolated-
vocal acceptance set before adoption rather than inferring song quality from speech scores.

- Official model card, supported languages, and local download instructions:
  <https://huggingface.co/Qwen/Qwen3-ForcedAligner-0.6B>
- Official implementation example:
  <https://github.com/QwenLM/Qwen3-ASR/blob/main/examples/example_qwen3_forced_aligner.py>

For unsupported forced-alignment languages, including Finnish, retain the current local Whisper
timestamps and monotonically map recognized tokens onto the known lyric sequence. Keep unmatched
words as explicit low-confidence intervals derived between adjacent anchors; never invent a word or
claim an exact boundary. The producer's existing cue editor remains the final correction path.

When no authoritative lyrics exist, **Qwen3-ASR-1.7B** is the future transcription candidate because
its official model card includes singing voice and songs with background music. That is a separate
unknown-lyrics feature and must not be smuggled into the known-lyrics alignment path.

## Kestrel integration boundary

If the activation gate is met:

1. Add a Kestrel-owned bounded separator adapter beside `music.rs`; never execute model-produced
   command text.
2. Use fixed argument arrays, absolute validated paths, a private per-job directory, bounded output
   sizes, cancellation, timeout, and recovery-safe cleanup.
3. Acquire the existing application work gate and unload competing GPU models before separation or
   alignment. Do not weaken `RuntimeManager`'s single-inference ownership.
4. Store derived stems under the immutable take's lyric-sync session. They are evidence for timing,
   not replacements for the preserved master.
5. Record input/output SHA-256 values, runner revision, model hash, parameters, duration, language,
   confidence, and `network: "disabled"` in the session receipt.
6. Reject non-monotonic, overlapping, non-finite, out-of-duration, excessive, or text-mismatched word
   timings before publication.
7. Keep the current path available until a fixed offline acceptance corpus proves the replacement
   at least as reliable on supported Kestrel hardware.

This proposal is intentionally dormant. It exists to make a future evidence-driven repair small,
reviewable, and consistent with Kestrel's offline and durable-data contracts.
