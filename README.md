# Kestrel Local

Kestrel is a Windows-first local model control plane, fully offline research workspace, and producer-directed movie, image, and music environment. Research uses the selected compatible local model with an English Kiwix Wikipedia archive; the included Ternary Bonsai 27B is managed by the same runtime and policies as every other local model. Movie Studio uses tool-free local story and scene collaborators while Rust owns every revision, scene card, media binding, render, and export. MiniMax H3 runs through local ComfyUI without research tools. Image Studio uses native Ideogram 4 with producer-owned structured composition and immutable local PNG takes. Music uses native MiniMax Music 3 with producer-owned arrangements and immutable local takes. Research remains usable as plain JSON and self-contained HTML; media projects remain ordinary JSON, FLAC, PNG, and MP4 files without Kestrel, SQLite, Codex, or internet access.

The application has nine adjacent workspaces:

- **Setup** owns blank-Windows onboarding: one click installs the assistant and offline archive, while a second production-suite action installs verified movie finishing, H3, Music 3, Chatterbox narration, and Whisper dictation. Producers who already have the release-profile weights can scan any number of existing AI folders or choose every model file individually; Setup recognizes all Bonsai, H3, Music 3, Ideogram 4, Chatterbox, Whisper, and MuScriptor assets, verifies pinned assets by exact bytes and SHA-256, then hard-links on the same drive or safely copies when required. Ideogram 4 remains a separate opt-in because its model agreement permits non-commercial use only; gated MuScriptor still requires explicit producer acceptance and is size/format checked before its local hash is recorded. Downloads are pinned, integrity-checked, observed, safely resumable, and kept on the producer-selected drive.
- **Control** discovers GGUF files read-only, keeps a recoverable startup catalog, rediscovers bundled/Jan/PATH engines, starts one authenticated managed `llama-server`, shows live VRAM, exposes exact launch arguments, and offers durable multimodal chat and Computer Tasks. The shared header can preview competing NVIDIA processes before a model is loaded: every proposed process is individually excludable, Advanced reveals default-excluded apps that may be deliberately included, and Kestrel-owned, Windows-critical, undisclosed, and graphics-driver processes remain locked. A selected process that refuses the ordinary close request gets a separate confirmed force-close action matching GpuClean's `taskkill /F`; only if that exact in-app action fails does Kestrel reveal a copyable administrator PowerShell command. Releasing Kestrel's own AI memory is available from the same menu.
- **Research** runs the selected local model through a fixed two-tool research protocol, visibly reports six stages, validates every citation, finds related prior work, and publishes immutable editions.
- **Studio** turns loose starting material into a complete editable Markdown story, then a producer-owned set of H3 scene cards and a non-linear offline timeline. Every story response is an immutable revision. The accepted story is always present in scene chat, while producers explicitly choose which scene cards receive full model context. The collaborator may suggest only textual scene operations; the producer alone selects reference media, exact audio, first/last frames, renders, versions, timeline edits, and exports. Split or repeat scenes, choose preserved masters, retime, trim, fade picture and sound, and render archive/publish/review cuts. Producers can also generate durable local character, location, prop, poster, and style-frame assets through H3's pseudo-image workflow: Kestrel preserves six nearby stable frames from one 22-frame pass, records the exact prompt/seed/graph, and attaches only the chosen candidate. Studio has no research, archive, filesystem, or model tool loop and unloads the language-model runtime before H3 receives the GPU.
- **Image** provides a fixed-window image-production desk with a contact sheet, full-resolution viewer, drawn or draggable 0–1000 layout boxes, overlap cycling, ordered layers, exact-text layers, review-only image backdrops, photo/art treatments, palettes, extreme aspect-ratio presets, explicit fixed/random seeds, and one-, two-, or four-variation native Ideogram 4 batches. Any selected local GGUF may stream an unapplied design proposal with visible reasoning state and stop-and-keep-checkpoint control; producers may instead author every field without an agent. Native Rust compiles Ideogram's exact order-sensitive compact JSON schema and fixed Comfy graph, validates every batch output, and preserves each PNG, prompt, seed, model profile, graph, and SHA-256 as an immutable take. Ideogram 4 is an optional non-commercial model and is not part of Kestrel's distributable production suite.
- **Music** provides a fixed-window, Mac-DAW-familiar arranger for producer-owned sections, description, lyrics, transport, take library, and exact generation settings. Any selected local GGUF may stream an unapplied description or tagged-lyrics proposal with stop-and-keep-checkpoint control. Native MiniMax Music 3 renders one honest stereo master through loopback ComfyUI with step progress and ETA; every lossless FLAC take, graph, model identity, seed, and SHA-256 remains durable. Each completed take opens directly in a full-screen audio-reactive visual lyric stage with two durable looks: the paper, weather, wildlife, and waveform ocean of **Living sketchbook**, and Kestrel's nocturnal spectral **Signal bloom**. One 1024-point analyser drives musically distinct sub-bass, bass, body, vocal-presence, air, transient, waveform, and lyric-writing reactions across both bounded renderers. Kestrel starts with editable timing estimates, can replace them with offline Whisper segments and word timestamps, reveals and seeks from every clicked word, supports producer-authored or local-model-assisted translations, and preserves the visual choice and every cue edit as immutable revisions beside the take. Setup can prepare an isolated offline MuScriptor GPU runner after the producer explicitly accepts and imports its gated non-commercial large checkpoint; manually configured compatible runners remain supported. MuScriptor output opens in the same window as a validated piano-roll document with track/instrument inspection, note audition, snapping, quantization, mute choices, synchronized master playback, undo/redo, immutable edit revisions, Reveal, and native MIDI Export.
- **Local speech** uses the setup-installed Chatterbox engine for opt-in narration and an owned, auditable ComfyUI adapter around a pinned OpenAI Whisper checkpoint for dictation and word timing. A shared Voice Library lets producers record or import consent-confirmed reference voices, choose an app-wide default, and override it beside an individual response. Voice identity is separate from the speech engine: reference audio is content-addressed, integrity-checked, and passed through Chatterbox's native `audio_prompt` input without transcoding. It has no browser, Windows, or remote fallback; generated audio, references, and recordings remain in Kestrel's private cache.
- **Developer** runs fixed offline checks. If Codex CLI is installed and signed in, an explicit one-click action can repair this Git workspace under an ephemeral workspace-write sandbox. Research never depends on it.
- **System** exposes local-model/Kiwix state and GPU telemetry, owns app-wide runtime defaults plus optional per-model exceptions, keeps Research-specific overrides explicit, and imports or exports the complete safe portable setup as editable JSON.

There is no benchmarking, autonomous model lab, leaderboard, analytics, remote model fallback, or background web research.

## Maintainer layout

Rust owns application truth; React owns the current view. The desktop lives in `apps/desktop`,
Rust-owned cross-feature contracts live in `crates/app-core`, generated TypeScript lives in
`packages/generated-bindings`, and the native composition root remains in `src-tauri`. See
[`ARCHITECTURE.md`](ARCHITECTURE.md#repository-and-ownership) for dependency rules and the staged
extraction plan.

## Tested local installation

```text
Model:       X:\KestrelAI\Bonsai\models\Ternary-Bonsai-27B-Q2_0.gguf
Runtime:     X:\KestrelAI\Bonsai\runtime\llama-server.exe
Kiwix:       X:\KestrelAI\OfflineWikipedia\tools\kiwix-serve.exe
Archive:     X:\KestrelAI\OfflineWikipedia\archives\wikipedia_en.zim
ComfyUI:     X:\KestrelAI\ComfyUI (127.0.0.1:8188)
Video:       MiniMax H3 int8 convrot + Qwen3-VL NVFP4/AWQ encoder
Music:       MiniMax Music 3 int8 DiT + Qwen3-Embedding-4B int8 + DAV VAE
Image:       Ideogram 4 conditional/unconditional NVFP4 + Qwen3-VL 8B NVFP4 + Flux 2 VAE
Snapshot:    2024-01-12
Articles:    6,863,660
```

Kestrel reads these assets in place. It does not copy the model or 102.3 GiB archive. A first-run model rescan also checks common Jan, LM Studio, Hugging Face, and Ollama locations plus user-added roots.

Subsequent starts merge an integrity-checked `model-catalog.json` cache with an immediate Bonsai scan, then refresh all configured roots in the background. Missing or size-changed weights are discarded from the cache. A corrupt cache is quarantined and rebuilt without blocking startup.

Control also provides an explicit observed Hugging Face GGUF downloader. A producer may paste a public repository or exact file page, inspect bounded GGUF choices with publisher sizes and LFS checksums, and start one durable transfer. Kestrel uses byte-range checkpoints, visible speed/ETA/retry state, free-space checks, SHA-256 plus GGUF validation, and an automatically scanned managed-model folder. Windows is kept awake only while that approved transfer is active (the display may turn off). Stop, shutdown, sleep, and network loss preserve partial bytes; restart marks the transfer interrupted and never resumes public-network work without a new producer action. Model inspection and transfer hold the same process-wide work gate as research, so strict research cannot overlap this network exception.

## Runtime design

One `RuntimeManager` owns Kestrel-managed model processes and one semaphore owns inference. Research, chat, Computer Tasks, and Studio writing assistants share that lease, so the detected hardware cannot accidentally receive duplicate Kestrel loads or simultaneous generations. Kestrel does not attach to or depend on a private model service.

Movie story/scene chat, Image Studio design assistance, and Music's optional writing assistants use that same lease. Kestrel explicitly unloads Comfy models before language-model work and stops the language-model runtime before H3, Ideogram 4, or Music 3 rendering, so large stacks never compete for the GPU. Kestrel owns Comfy lifecycle and queue polling; it never routes through or assumes a separate hosted product.

Managed launches bind to a private loopback port, use a random session API key, one slot, strict full-GPU placement, no prompt RAM cache, and no silent fit/offload. The API key is redacted from the visible launch proof.

System defines the default engine, context, output allowance, thread count, and advanced policy for every local model. Optional per-model exceptions override those defaults; a workspace's explicit model setting overrides both. Research keeps its additional archive and tool-loop limits in a separate, clearly labeled override.

> Warning: invalid or oversized values can stop startup or exhaust VRAM. Runtime and hardware limits still apply.

## Research harness

The harness gives the selected local model two logical tools:

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
|-- speech-cache\voices   # recoverable profiles + content-addressed private voice references
|-- movies\_references     # SHA-256 producer-media objects + validated metadata
|   `-- generations\<uuid> # recoverable image-pass request, exact graph, status, candidate receipt
|-- movies\<uuid>          # recoverable, versioned production
|   |-- request.json       # exact user prompt + production settings
|   |-- references.json    # producer jobs, stable asset IDs, H3 media metadata
|   |-- references\*       # immutable hard-linked/copied project media
|   |-- producer\          # recoverable workspace, story revisions, scene history, conversations
|   |-- plan.json          # native renderer projection of producer-owned scene cards
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
|   |-- lyrics\<take>\<sync>\ # Whisper/draft receipt plus immutable editable cue revisions
|   `-- midi\<take>\<transcription>\ # immutable source, typed edit JSON, receipts, and MIDI revisions
|-- images\<uuid>          # recoverable producer-owned image project
|   |-- project.json       # brief, structured composition, settings, take ledger, status
|   |-- takes\*.png         # immutable full-resolution local masters
|   `-- receipts\          # exact structured prompts and submitted Comfy graphs
`-- reports\YYYY\MM\<title>--<id>\
    |-- index.html        # self-contained, printable research page
    |-- report.json       # complete structured edition
    |-- sources.json      # inspected evidence ledger
    `-- provenance.json   # model, archive, query, profile, lineage
```

Publication builds a complete staging directory and atomically renames it into place. Existing IDs are never overwritten. At startup Kestrel repairs missing derived HTML/source/provenance files from `report.json` and repopulates a missing SQLite catalog. If SQLite cannot be initialized, it is preserved with a timestamped `.corrupt-*` name before a clean cache is built from reports.

This keeps thousands of editions searchable through FTS5 while local models and ordinary tools can always traverse JSONL and report folders directly.

Chat and Computer Tasks can attach any regular local file up to 128 MiB. Kestrel copies it into a SHA-256-addressed object store before inference. Images and audio use loopback-only llama.cpp multimodal content blocks when the selected GGUF projector advertises those modalities. PDF, DOCX, PPTX, XLSX, source code, markup, logs, and common text formats receive bounded local text extraction. Unknown binaries remain durable and clearly marked metadata-only; Kestrel never claims the model read content it could not decode. Computer Tasks can request additional ranges from a declared attachment through a read-only typed tool.

Movie creation stores the exact starting material, settings, and attached references before inference begins. One tool-free response creates the first complete Markdown story revision; later collaborator and producer edits append revisions instead of overwriting them. Accepting a revision opens a continued or fresh scene conversation without deleting existing scene cards. The accepted story is always supplied in full, but only producer-selected cards enter full scene context. Native code applies bounded add/update/remove/split operations around stable IDs, validates every save, and preserves a scene-history snapshot. H3 clips are 24 fps with native stereo audio. Producers choose end-frame chaining, independent first/last frames, visual references, and exact audio. A renderer-affecting scene edit versions the old master and queues a new one rather than presenting stale output as current. Timeline decisions have stable item IDs and may repeat or split a source while selecting any immutable scene version. Trims, 0.25-4× retiming, picture/audio fades, gain, optional LUFS normalization, and export presets are validated natively before FFmpeg receives fixed arguments. Each successful export is atomically renamed into place and recorded with byte length, duration, SHA-256, and a complete JSON decision-list sidecar; raw clips are never modified. Maintainer-facing Studio ownership and lifecycle documentation lives in [`src-tauri/src/studio/README.md`](src-tauri/src/studio/README.md).

Producer references deliberately use a separate large-media store and never enter language-model context. The native picker validates real streams with FFprobe, applies bounded image/audio/video sizes and H3 duration limits, copies while hashing, and verifies the SHA-256 object again when a project is created. Kestrel supplies a useful default description so attaching media does not require busywork; the producer may edit it and explicitly chooses visual, exact-audio, guidance, and first/last-frame use on each scene card. At render time Kestrel injects only compact native bindings such as `Use <Picture 1> as a visual reference` and `Use <Audio 1> exactly as it is`, constructs Comfy's V3 autogrow inputs, and selects the installed `ref2va` weights. Audio is treated as existing native clip audio rather than an abstract voice-identity profile. The supported production envelope is 9 pictures, 3 videos, and 3 audio signals. Reference-conditioned shots do not also chain a prior frame because those are separate native H3 conditioning paths.

Generated picture references use the installed H3 `fl2va` model without any download or remote call. The built-in graph is derived from the fixed `MiniMax-H3-Pseudo-Image-Generation-Workflow.json` revision `1abf4a61eddffd08fa407e013ea7b7e62fbbbbf4`: requested length 8 resolves to H3's 22-frame native grid, then frames 8-13 are saved as a producer choice strip. Portrait, landscape, and square presets stay within the standard local pixel budget. Advanced mode exposes the exact rendered prompt, model filenames, sampler, scheduler, seed, and API graph. Every run is written before ComfyUI submission, interrupted runs are marked rather than resumed, and selected candidates enter the same content-addressed integrity path as imported media.

Portable setup profiles contain app-wide and per-model runtime policy, Research tuning, path-independent identities for every discovered model, and safe component-location hints for the complete installed application. The System page exposes the bounded JSON next to Import and Export so it can be reviewed or edited before validation. Profiles never include weights, chats, reports, media projects, credentials, developer paths, or Full Access authority. Import keeps local workspace/developer authority, rediscovers a local engine, validates usable component locations, and rescans weights before returning control.

## Offline boundary

- The WebView CSP permits no external network connection or remote asset.
- Native research clients accept only fixed loopback services.
- Movie rendering accepts only fixed loopback ComfyUI on `127.0.0.1:8188`; completed media is copied into Kestrel's library before use.
- Image generation accepts only fixed loopback ComfyUI on `127.0.0.1:8188`; the code-owned Ideogram graph accepts no arbitrary workflow or remote model call, and completed PNGs are copied into the project before success.
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
cargo test --manifest-path src-tauri\Cargo.toml live_ideogram_graph_preserves_a_full_resolution_png -- --ignored --nocapture
```

The in-app Developer screen runs the deterministic checks offline. Its optional Codex repair uses the same contract captured in `AGENTS.md`, but code, tests, recovery paths, and actionable errors remain the primary maintenance surface.

### Desktop launcher shortcut (Windows)

From this branch, you can build and launch the app directly from a desktop shortcut path by running:

```powershell
npm run launch
```

By default, the launcher updates (or creates) a `Kestrel Local.lnk` shortcut on your Desktop (including OneDrive-synchronized Desktop locations), and then:

- runs `npm run build`
- runs `npm run tauri build`
- starts `src-tauri\target\release\kestrel-local.exe`

You can pass a custom shortcut target via `-ShortcutPath <path>` or use `npm run launch -- --SkipBuild` to launch an existing binary without rebuilding.

## Next useful improvements

- Add artifact hashes and an in-app integrity/rebuild action.
- Supply the release signing certificate and validate the signed offline installer on a clean disconnected Windows VM.
- Add local papers/books/document adapters behind the same evidence contract.
- Add collection/tag and parent/child comparison views after sustained library use.
- Capture firewall evidence for a complete strict offline run.

Kestrel's code is MIT, but installed model licenses still govern model use and output workflows. In particular, the published Ideogram 4 agreement permits only non-commercial purposes and explicitly excludes outputs used in or to advertise revenue-generating products or services. Setup requires an explicit acknowledgement, stores the pinned agreement beside the weights, and keeps Ideogram 4 outside the commercial production-suite action. A producer needs separate rights from Ideogram before commercial use.

MIT. Bonsai, llama.cpp, Kiwix, Wikipedia content, and installed components retain their own licenses and provenance.
