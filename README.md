# Kestrel Local

Kestrel is a Windows-first local model control plane, fully offline research workspace, and agent-directed movie studio. The model-specific path uses Bonsai 27B, an English Kiwix Wikipedia archive, and MiniMax H3 through local ComfyUI. Research remains usable as plain JSON and self-contained HTML; movie projects remain ordinary JSON, PNG, and MP4 files without Kestrel, SQLite, Codex, or internet access.

The application has five adjacent workspaces:

- **Control** discovers GGUF files read-only, keeps a recoverable startup catalog, rediscovers installed Bonsai/Jan/PATH engines, attaches to an existing Bonsai service or starts one authenticated `llama-server`, shows live VRAM, exposes exact launch arguments, and offers durable multimodal chat and Computer Tasks.
- **Research** runs a Bonsai-specific two-tool harness, visibly reports six stages, validates every citation, finds related prior work, and publishes immutable editions.
- **Studio** turns one unmodified user prompt into a Bonsai-authored screenplay, continuity bible, native-audio MiniMax H3 clips, and an editable first cut. Optional producer pictures, videos, and audio are imported once, described in plain language, and bound through H3's native reference model. Studio can consult offline Wikipedia through two sequential tools, then unloads Bonsai before H3 receives the GPU.
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
Snapshot:    2024-01-12
Articles:    6,863,660
```

Kestrel reads these assets in place. It does not copy the model or 102.3 GiB archive. A first-run model rescan also checks common Jan, LM Studio, Hugging Face, and Ollama locations plus user-added roots.

Subsequent starts merge an integrity-checked `model-catalog.json` cache with an immediate Bonsai scan, then refresh all configured roots in the background. Missing or size-changed weights are discarded from the cache. A corrupt cache is quarantined and rebuilt without blocking startup.

## Runtime design

One `RuntimeManager` owns Kestrel-managed model processes and one semaphore owns inference. Research and chat share that lease, so a 12 GiB GPU cannot accidentally receive duplicate Kestrel loads or simultaneous generations. If the installed Bonsai endpoint is already healthy, Kestrel attaches without launching another model.

Movie planning uses that same lease. Kestrel explicitly unloads Comfy models before Bonsai planning and stops Bonsai before H3 rendering, so the two large stacks never compete for the RTX 5070. The Studio owns Comfy lifecycle and queue polling; it never routes through or assumes the separate Wan Video Studio product.

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
|-- movies\<uuid>          # recoverable, versioned production
|   |-- request.json       # exact user prompt + production settings
|   |-- references.json    # producer jobs, stable asset IDs, H3 media metadata
|   |-- references\*       # immutable hard-linked/copied project media
|   |-- plan.json          # Bonsai screenplay and H3 clip prompts
|   |-- sources.json       # only archive pages actually opened
|   |-- project.json       # live status, edit decisions, provenance
|   |-- raw\*.mp4          # immutable native-audio H3 masters
|   |-- stills\*.png       # end frames used for chosen continuity chains
|   |-- exports\*.mp4      # first cut and non-destructive edit exports
|   `-- logs\              # local renderer startup diagnostics
`-- reports\YYYY\MM\<title>--<id>\
    |-- index.html        # self-contained, printable research page
    |-- report.json       # complete structured edition
    |-- sources.json      # inspected evidence ledger
    `-- provenance.json   # model, archive, query, profile, lineage
```

Publication builds a complete staging directory and atomically renames it into place. Existing IDs are never overwritten. At startup Kestrel repairs missing derived HTML/source/provenance files from `report.json` and repopulates a missing SQLite catalog. If SQLite cannot be initialized, it is preserved with a timestamped `.corrupt-*` name before a clean cache is built from reports.

This keeps thousands of editions searchable through FTS5 while local models and ordinary tools can always traverse JSONL and report folders directly.

Chat and Computer Tasks can attach any regular local file up to 128 MiB. Kestrel copies it into a SHA-256-addressed object store before inference. Images and audio use loopback-only llama.cpp multimodal content blocks when the selected GGUF projector advertises those modalities. PDF, DOCX, PPTX, XLSX, source code, markup, logs, and common text formats receive bounded local text extraction. Unknown binaries remain durable and clearly marked metadata-only; Kestrel never claims the model read content it could not decode. Computer Tasks can request additional ranges from a declared attachment through a read-only typed tool.

Movie creation stores its project before inference begins and rewrites status atomically after every meaningful transition. On restart, active work becomes explicitly `interrupted`; it is never silently resumed. Resume skips already completed masters. Bonsai gets a compact director contract rather than a template screenplay: the user's text is its own unchanged message, output may use the validated 98,304/32,768 profile, creativity sampling remains adjustable, and only renderer/data-shape limits are enforced. H3 clips are 24 fps with native stereo audio. Bonsai may choose end-frame chaining for continuous action; fresh cuts keep self-contained continuity prompts. Advanced edits change an edit decision list and export a new MP4 without modifying source clips.

Producer references deliberately use a separate large-media store rather than injecting binary media into Bonsai context. The native picker validates real streams with FFprobe, applies bounded image/audio/video sizes and H3 duration limits, copies while hashing, and verifies the SHA-256 object again when a project is created. Each attachment needs a producer description of the job it should do. Bonsai sees only those descriptions and opaque asset IDs, selects IDs per clip, and never claims it inspected raw media. Kestrel—not the model—renumbers and injects `<Picture n>`, `<Video n>`, and `<Audio n>` assignments, constructs Comfy's V3 autogrow inputs, and selects the installed `ref2va` weights. The supported production envelope is 9 pictures, 3 videos, and 3 audio signals; video soundtrack use is explicit and off by default. Reference-conditioned shots do not also chain a prior frame because those are separate native H3 conditioning paths.

Portable setup profiles contain research/runtime tuning, path-independent model identities, and local path hints. They never include weights, chats, research, credentials, developer paths, or Full Access authority. Import validates a bounded JSON file, keeps local developer/workspace paths, locks Full Access, rediscovers a local engine, and rescans weights before returning control.

## Offline boundary

- The WebView CSP permits no external network connection or remote asset.
- Native research clients accept only fixed loopback services.
- Movie rendering accepts only fixed loopback ComfyUI on `127.0.0.1:8188`; completed media is copied into Kestrel's library before use.
- Kiwix runs with external access blocked.
- Research receives only its two citation tools. Ordinary chat receives no mutation tools. Computer Tasks receives only typed, policy-checked tools; attachment reads are restricted to files explicitly selected for that task.
- Codex is isolated to `developer.rs`, requires explicit confirmation, cannot run during research, creates no commit, and is not required for diagnostics or any offline feature.

Wikipedia is a tertiary starting point with a January 2024 cutoff. Reports expose the inspected evidence and open questions; they do not claim finality.

## Run and verify

Requirements are Windows 10/11, Node.js 20.19+ or 22.12+, Rust stable with MSVC, WebView2, FFmpeg on PATH, and the local assets above.

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
