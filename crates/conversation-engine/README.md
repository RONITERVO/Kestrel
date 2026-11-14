# Native conversation engine

CPU speech and audio devices for Kestrel. The desktop executable dispatches
`--conversation-voice-worker <absolute asset root>` before starting Tauri. The parent owns
the child in a Windows job object and communicates over bounded JSON lines. There is no HTTP
listener, download client, Python interpreter or GPU backend in this runtime.

`worker.rs` retains one selected Pocket model/voice and one English-only `whisper-medium.en`
model (hybrid F32 encoder + Q8_0 KV-cached decoder) on independent bounded executors (`xn` pinned
to 2 threads for PocketTTS, Rayon bounded to 2–8 threads via `(available_parallelism / 4).clamp(2, 8)`
for Whisper). `device.rs` owns CPAL
streams, streaming resampling, Sonora
AEC3/noise suppression, endpoint detection and bounded playback. Capture session IDs and
cancellation invalidate stale speech across mute, navigation, language changes and recovery.
`whisper.rs` uses greedy text decoding; it does not claim word alignment.

The normal app launches its own executable. `examples/worker.rs` exercises the identical
entry point for explicit live acceptance tests before rebuilding the full desktop.

## Upstream provenance

- Pocket wrapper files `config.rs`, `defaults.rs`, `model.rs`, `rng.rs`, `text.rs` and
  `weights.rs` derive from the official [Pocket TTS Rust implementation at
  9e8413ebc128dcbef682cce1906c7135f5b1e281](https://github.com/kyutai-labs/pocket-tts/tree/9e8413ebc128dcbef682cce1906c7135f5b1e281/rust).
  Its MIT license is retained in `LICENSE-Pocket-TTS`. Kestrel removes CLI/WAV output,
  runtime downloads, the C++ SentencePiece tokenizer, recorded-voice encoding and unused
  voice catalogs. It uses the existing local JSON tokenizer and supplied conditioning,
  exposes cooperative cancellation, and bounds the synthesis channel to two latent frames.
  Retained upstream configuration, text preparation and sampling tests cover these adapters.
- Inference follows that upstream revision: `xn-ptts` is pinned to
  `7efa26973f81bba42cf2ace8da2c1eb9b605d54d`, with `xn` 0.2.11. Only CPU features are enabled;
  `.cargo/config.toml` sets `+avx2,+fma,+f16c` for `x86_64-pc-windows-msvc` builds.
- Whisper uses Candle 0.9.2 and an adapted greedy decoder from Candle's Whisper example.
  `LICENSE-Candle-MIT` retains its MIT license. The 80-bin `melfilters.bytes` constants
  originate in Candle's `candle-examples/examples/whisper/melfilters.bytes`;
  SHA-256 is `85818f156f7e189453901a515e4726d270d307f976e161cf9403e8caab405498`.
  Model/tokenizer/config files come from the official `openai/whisper-medium.en` checkpoint,
  revision `2e98eb6279edf5095af0c8dedb36bdec0acd172b`, through Kestrel's explicit Setup manifest.
- CPAL 0.16 provides native devices; Sonora 0.3.0 provides Rust WebRTC audio processing.
  Their source and dependency licenses remain in the Cargo distribution. Exact transitive
  versions and checksums are recorded in the repository's `Cargo.lock`.

Model revisions, byte lengths and SHA-256 hashes live in
`src-tauri/resources/conversation_voice/assets.json`. The seven local configurations use
only those public checkpoints and supplied voice states. Custom voices and manual alignment
remain in Kestrel's separate production speech route.

## Verification

Run `cargo test -p kestrel-conversation-engine` and
`cargo clippy -p kestrel-conversation-engine --all-targets -- -D warnings`, in addition to
the app's required checks. The device tests cover endpointing, resampling, utterance bounds
and synthetic delayed echo with simultaneous near speech. Physical device and language
acceptance instructions are in `docs/POCKET_TTS_PLAN.md`.
