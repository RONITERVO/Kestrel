# Kestrel Local

Kestrel is a Windows-first local model control plane, fully offline research workspace, and producer-directed movie and music environment. Research uses its release-validated Bonsai 27B harness with an English Kiwix Wikipedia archive; the separate Studio can pin any compatible local GGUF as Director and Reviewer and uses MiniMax H3 through local ComfyUI without research tools. Music uses native MiniMax Music 3 with producer-owned arrangements and immutable local takes. Research remains usable as plain JSON and self-contained HTML; media projects remain ordinary JSON, WAV, PNG, and MP4 files without Kestrel, SQLite, Codex, or internet access.

The application has eight adjacent workspaces:

- **Setup** owns blank-Windows onboarding: one click installs the assistant and offline archive, while a second production-suite action installs verified movie finishing, H3, Music 3, Chatterbox narration, and Kestrel's timestamped Whisper adapter. Downloads are pinned, integrity checked, observed, safely resumable, and kept on the producer-selected drive.
- **Control** discovers GGUF files read-only, keeps a recoverable startup catalog, rediscovers installed Bonsai/Jan/PATH engines, attaches to an existing Bonsai service or starts one authenticated `llama-server`, shows live VRAM, exposes exact launch arguments, and offers durable multimodal chat and Computer Tasks.
- **Research** runs a Bonsai-specific two-tool harness, visibly reports six stages, validates every citation, finds related prior work, and publishes immutable editions.
- **Studio** turns one unmodified user prompt into a local-Director-authored screenplay, continuity bible, independently reviewed production-grade MiniMax H3 prompts, native-audio clips, and a non-linear offline timeline. Director and Reviewer are pinned per project; generic chat-template GGUFs receive a recoverable, version-bound local protocol check before standard-mode unattended use, and two different role models are swapped through the sole inference slot rather than loaded together. Split or repeat scenes, choose any preserved version, retime, trim, fade picture and sound, audition the sequence, undo decisions, and render archive/publish/review cuts. Producers can also generate durable local character, location, prop, poster, and style-frame assets through H3's pseudo-image workflow: Kestrel preserves six nearby stable frames from one 22-frame pass, records the exact prompt/seed/graph, and attaches only the chosen candidate. Optional producer pictures, videos, and exact clip audio are imported once, described for placement, and bound through H3's native reference model. Studio has no research or archive tools and unloads the language-model runtime before H3 receives the GPU.
- **Music** provides a fixed-window, Mac-DAW-familiar arranger for producer-owned sections, description, lyrics, transport, take library, and exact generation settings. Any selected local GGUF may stream an unapplied description or tagged-lyrics proposal with stop-and-keep-checkpoint control. Native MiniMax Music 3 renders one honest stereo master through loopback ComfyUI with step progress and ETA; every lossless FLAC take, graph, model identity, seed, and SHA-256 remains durable. Advanced producers may configure their own gated MuScriptor executable and checkpoint for explicit audio-to-MIDI transcription.
- **Local speech** uses the setup-installed Chatterbox model for opt-in narration and an owned, auditable ComfyUI adapter around a pinned OpenAI Whisper checkpoint for dictation and word timing. It has no browser, Windows, or remote fallback; generated audio and recordings remain in Kestrel's private cache.
- **Developer** runs fixed offline checks. If Codex CLI is installed and signed in, an explicit one-click action can repair this Git workspace under an ephemeral workspace-write sandbox. Research never depends on it.
- **System** exposes the installed Bonsai/Kiwix state, GPU telemetry, and opt-in high-capacity research settings.

There is no benchmarking, autonomous model lab, leaderboard, analytics, remote model fallback, or background web research.

## Tested local installation

```text
Model API:   http://127.0.0.1:8080/v1
Model:       D:\LocalAI\Bonsai27B\models\Ternary-Bonsai-27B-Q2_0.gguf
Runtime:     D:\LocalAI\Bonsai27B\runtime\llama-server.exe
Kiwix:       D:\LocalAI\OfflineWikipedia\tools\kiwix-tools-3.8.1\kiwix-serve.exe
Archive:     D:\OfflineInternet\wikipedia_en_all_maxi_2024-01.zim
ComfyUI:     D:\AI\ComfyUI (127.0.0.1:8188)
Video:       MiniMax H3 int8 convrot + Qwen3-VL NVFP4/AWQ encoder
Music:       MiniMax Music 3 int8 DiT + Qwen3-Embedding-4B int8 + DAV VAE
Snapshot:    2024-01-12
Articles:    6,863,660
```

Kestrel reads these assets in place. It does not copy the model or 102.3 GiB archive. A first-run model rescan also checks common Jan, LM Studio, Hugging Face, and Ollama locations plus user-added roots.

Subsequent starts merge an integrity-checked `model-catalog.json` cache with an immediate Bonsai scan, then refresh all configured roots in the background. Missing or size-changed weights are discarded from the cache. A corrupt cache is quarantined and rebuilt without blocking startup.

Control also provides an explicit observed Hugging Face GGUF downloader. A producer may paste a public repository or exact file page, inspect bounded GGUF choices with publisher sizes and LFS checksums, and start one durable transfer. Kestrel uses byte-range checkpoints, visible speed/ETA/retry state, free-space checks, SHA-256 plus GGUF validation, and an automatically scanned managed-model folder. Windows is kept awake only while that approved transfer is active (the display may turn off). Stop, shutdown, sleep, and network loss preserve partial bytes; restart marks the transfer interrupted and never resumes public-network work without a new producer action. Model inspection and transfer hold the same process-wide work gate as research, so strict research cannot overlap this network exception.

## Runtime design

One `RuntimeManager` owns Kestrel-managed model processes and one semaphore owns inference. Research and chat share that lease, so a 12 GiB GPU cannot accidentally receive duplicate Kestrel loads or simultaneous generations. If the installed Bonsai endpoint is already healthy, Kestrel attaches without launching another model.

Movie planning and Music's optional writing assistants use that same lease. Kestrel explicitly unloads Comfy models before language-model work and stops the language-model runtime before H3 or Music 3 rendering, so the two large stacks never compete for the GPU. Different Director and Reviewer models are switched only between complete turns. Kestrel owns Comfy lifecycle and queue polling; it never routes through or assumes the separate Wan Video Studio product.

Managed launches bind to `127.0.0.1`, use a random session API key, one slot, strict full-GPU placement, no prompt RAM cache, no silent fit/offload, and Bonsai-specific Q4 KV plus flash attention. The API key is redacted from the visible launch proof.

The validated advanced Bonsai profile is 98,304 context tokens and 32,768 maximum response tokens. Advanced mode deliberately adds no Kestrel upper caps to context, output, lane count, result count, source target, tool turns, thinking budget, or source excerpt size.

> Warning: invalid or oversized values can stop startup or exhaust VRAM. Runtime and hardware limits still apply.

## Research harness

The harness gives Bonsai two logical tools:

- `search_archive(query, limit)` searches local Kiwix plus the existing Kestrel catalog.
- `read_source(source_ref, section, max_chars)` opens an exact prior report or Wikipedia result and records evidence.

Search candidates cannot be cited. A source ID exists only after a successful read, and native Rust intersects every model citation with that evidence ledger before publication.

Solo expedition uses high capacity without duplicate agents or KV caches: one planning pass creates complementary lanes, Kestrel performs their CPU/I/O archive searches concurrently, and compact candidates become shared context for the same lead model. The model then chooses and opens evidence through the normal tool loop.

## Durable library

The default root is the unsynced local home directory:

```text
C:\Users\<you>\Kestrel Research\
|-- README.txt
|-- catalog.sqlite3       # disposable/rebuilt FTS5 cache
|-- catalog.jsonl         # open one-record-per-line index
|-- model-catalog.json    # recoverable local GGUF discovery cache
|-- setup-profiles\       # safe portable tuning/model identity profiles
|-- maintenance\          # optional Codex repair transcripts
|-- workspace\attachments # content-addressed local context objects and extractions
|-- workspace\chats       # recoverable chat transcripts and attachment references
|-- workspace\tasks       # recoverable computer-task transcripts
|-- movies\_references     # SHA-256 producer-media objects + validated metadata
|   `-- generations\<uuid> # recoverable image-pass request, exact graph, status, candidate receipt
|-- movies\<uuid>          # recoverable, versioned production
|   |-- request.json       # exact user prompt + production settings
|   |-- references.json    # producer jobs, stable asset IDs, H3 media metadata
|   |-- references\*       # immutable hard-linked/copied project media
|   |-- agent-workspace\   # durable Director brief, scenes, checks, transcripts, exact requests
|   |-- plan.json          # checked screenplay and H3 clip prompts
|   |-- sources.json       # only archive pages actually opened
|   |-- project.json       # live status, non-linear edit decisions, export ledger, provenance
|   |-- raw\*.mp4          # immutable native-audio H3 masters
|   |-- stills\*.png       # end frames used for chosen continuity chains
|   |-- exports\*.mp4      # immutable first cuts and timeline exports
|   |-- exports\*.json     # exact edit decision sidecar + SHA-256 export record
|   `-- logs\              # local renderer startup diagnostics
|-- music\<uuid>           # recoverable producer-owned song project
|   |-- project.json       # arrangement, lyrics, settings, take history, and status
|   |-- takes\*.flac        # immutable lossless stereo masters
|   |-- receipts\<take_id>.graph.json # exact submitted graph and generation receipt
|   `-- midi\*.mid          # optional explicit MuScriptor transcriptions
`-- reports\YYYY\MM\<title>--<id>\
    |-- index.html        # self-contained, printable research page
    |-- report.json       # complete structured edition
    |-- sources.json      # inspected evidence ledger
    `-- provenance.json   # model, archive, query, profile, lineage
```

Publication builds a complete staging directory and atomically renames it into place. Existing IDs are never overwritten. At startup Kestrel repairs missing derived HTML/source/provenance files from `report.json` and repopulates a missing SQLite catalog. If SQLite cannot be initialized, it is preserved with a timestamped `.corrupt-*` name before a clean cache is built from reports.

This keeps thousands of editions searchable through FTS5 while local models and ordinary tools can always traverse JSONL and report folders directly.

Chat and Computer Tasks can attach any regular local file up to 128 MiB. Kestrel copies it into a SHA-256-addressed object store before inference. Images and audio use loopback-only llama.cpp multimodal content blocks when the selected GGUF projector advertises those modalities. PDF, DOCX, PPTX, XLSX, source code, markup, logs, and common text formats receive bounded local text extraction. Unknown binaries remain durable and clearly marked metadata-only; Kestrel never claims the model read content it could not decode. Computer Tasks can request additional ranges from a declared attachment through a read-only typed tool.

Movie creation stores its project and exact model-role bindings before inference begins and rewrites status atomically after every meaningful transition. On restart, active work becomes explicitly `interrupted`; it is never silently resumed. Resume skips already completed masters and refuses to substitute a missing pinned model. The Director works through a bounded project-local movie workspace and receives the producer's unchanged request plus the complete current story on every turn. Producers can stream its text, redirect it between safe turns, or request a graceful durable checkpoint. The plan must pass two clean native checks and a separate fresh-context whole-film review by the pinned Reviewer; three unresolved independent-review rounds fail visibly with the workspace intact. Native checks require every 5-15 second renderer prompt to contain 120-450 words plus camera, timed action, visual treatment, and audio direction. H3 clips are 24 fps with native stereo audio. The Director may choose end-frame chaining for continuous action; fresh cuts keep self-contained continuity prompts. Timeline decisions have stable item IDs and may repeat or split a source while selecting any immutable scene version. Trims, 0.25-4× retiming, picture/audio fades, gain, optional LUFS normalization, and export presets are validated natively before FFmpeg receives fixed arguments. Each successful edit export is atomically renamed into place and recorded with its byte length, duration, SHA-256 hash, and complete JSON decision-list sidecar; raw clips are never modified. Maintainer-facing Studio ownership and lifecycle documentation lives in [`src-tauri/src/studio/README.md`](src-tauri/src/studio/README.md).

Producer references deliberately use a separate large-media store rather than injecting binary media into language-model context. The native picker validates real streams with FFprobe, applies bounded image/audio/video sizes and H3 duration limits, copies while hashing, and verifies the SHA-256 object again when a project is created. Each attachment needs a producer description that tells the Director where to place it. The Director sees those planning descriptions and opaque asset IDs, selects IDs per clip, and never claims it inspected raw media. The descriptions are not sent to H3. At render time Kestrel injects only compact native bindings such as `Use <Picture 1> as a visual reference` and `Use <Audio 1> exactly as it is`, constructs Comfy's V3 autogrow inputs, and selects the installed `ref2va` weights. Audio is treated as existing native clip audio rather than an abstract voice-identity profile. The supported production envelope is 9 pictures, 3 videos, and 3 audio signals; video soundtrack use is explicit and off by default. Reference-conditioned shots do not also chain a prior frame because those are separate native H3 conditioning paths.

Generated picture references use the installed H3 `fl2va` model without any download or remote call. The built-in graph is derived from the fixed `MiniMax-H3-Pseudo-Image-Generation-Workflow.json` revision `1abf4a61eddffd08fa407e013ea7b7e62fbbbbf4`: requested length 8 resolves to H3's 22-frame native grid, then frames 8-13 are saved as a producer choice strip. Portrait, landscape, and square presets stay within the standard local pixel budget. Advanced mode exposes the exact rendered prompt, model filenames, sampler, scheduler, seed, and API graph. Every run is written before ComfyUI submission, interrupted runs are marked rather than resumed, and selected candidates enter the same content-addressed integrity path as imported media.

Portable setup profiles contain research/runtime tuning, path-independent model identities, and local path hints. They never include weights, chats, research, credentials, developer paths, or Full Access authority. Import validates a bounded JSON file, keeps local developer/workspace paths, locks Full Access, rediscovers a local engine, and rescans weights before returning control.

## Offline boundary

- The WebView CSP permits no external network connection or remote asset.
- Native research clients accept only fixed loopback services.
- Movie rendering accepts only fixed loopback ComfyUI on `127.0.0.1:8188`; completed media is copied into Kestrel's library before use.
- Music generation and optional transcription are local-only; generated lossless stereo masters and MIDI files are copied into the project before Kestrel reports success.
- Kiwix runs with external access blocked.
- Research receives only its two citation tools. Ordinary chat receives no mutation tools. Computer Tasks receives only typed, policy-checked tools; attachment reads are restricted to files explicitly selected for that task.
- Codex is isolated to `developer.rs`, requires explicit confirmation, cannot run during research, creates no commit, and is not required for diagnostics or any offline feature.

Wikipedia is a tertiary starting point with a January 2024 cutoff. Reports expose the inspected evidence and open questions; they do not claim finality.

## Run and verify

The installed product requires Windows 10/11 and a supported NVIDIA production GPU. The NSIS package embeds the offline WebView2 installer; Setup installs FFmpeg, ComfyUI, engines, and model assets without requiring a terminal, Python, Git, Node.js, or Rust installation.

Building Kestrel from source requires Node.js 20.19+ or 22.12+, Rust stable with MSVC, and the normal Windows build toolchain:

```powershell
npm install
npm run tauri dev
```

Required deterministic checks:

```powershell
git diff --check
cargo test --all-targets --manifest-path src-tauri\Cargo.toml
cargo clippy --all-targets --manifest-path src-tauri\Cargo.toml -- -D warnings
npm run check
npm test -- --run
npm run build
```

Create a current-user Windows installer that remains installable without a network connection:

```powershell
npm run package:offline
```

The release script emits SHA-256 hashes, signature status, a machine-readable manifest, and supports mandatory Authenticode signing when a certificate is supplied. See [RELEASING.md](RELEASING.md).

Live archive and model acceptance:

```powershell
cargo test --manifest-path src-tauri\Cargo.toml live_archive_search_and_read -- --ignored --nocapture
cargo test --manifest-path src-tauri\Cargo.toml live_bonsai_research_creates_a_complete_offline_bundle -- --ignored --nocapture
cargo test --manifest-path src-tauri\Cargo.toml live_solo_expedition_uses_shared_lanes_and_high_output_budget -- --ignored --nocapture
cargo test --manifest-path src-tauri\Cargo.toml live_bonsai_movie_plan_clears_the_production_prompt_gate -- --ignored --nocapture
cargo test --manifest-path src-tauri\Cargo.toml live_one_prompt_movie_produces_a_native_audio_first_cut -- --ignored --nocapture
cargo test --manifest-path src-tauri\Cargo.toml live_one_prompt_movie_uses_native_picture_and_audio_references -- --ignored --nocapture
```

The in-app Developer screen runs the deterministic checks offline. Its optional Codex repair uses the same contract captured in `AGENTS.md`, but code, tests, recovery paths, and actionable errors remain the primary maintenance surface.

## Next useful improvements

- Add artifact hashes and an in-app integrity/rebuild action.
- Supply the release signing certificate and validate the signed offline installer on a clean disconnected Windows VM.
- Add local papers/books/document adapters behind the same evidence contract.
- Add collection/tag and parent/child comparison views after sustained library use.
- Capture firewall evidence for a complete strict offline run.

MIT. Bonsai, llama.cpp, Kiwix, Wikipedia content, and bundled components retain their own licenses and provenance.
