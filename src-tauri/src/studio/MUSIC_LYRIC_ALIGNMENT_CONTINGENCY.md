# Music lyric isolation and alignment contingency

Status: **do not replace the current path without an acceptance failure.** Extended producer
testing in August 2026 found Kestrel's installed Whisper lyric sync correct in approximately 99% of
the tested material. That result is not a general benchmark, but it is strong evidence that another
runtime, model download, and maintenance surface are not presently justified.

This note records the preferred fallback so a future maintainer does not have to restart the model
and licensing investigation if dense arrangements, new languages, or a changed generator cause a
repeatable regression.

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
