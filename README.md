# Kestrel Local

Kestrel is a Windows-first local model control plane with fully offline research and video-production workspaces. The research path uses Bonsai 27B plus an English Kiwix Wikipedia archive and produces source-traceable editions that remain usable as plain JSON and self-contained HTML without Kestrel, SQLite, Codex, or internet access. Video Studio adds durable local ComfyUI planning and generation without changing that offline boundary.

The application has five adjacent workspaces:

- **Control** discovers GGUF files read-only, keeps a recoverable startup catalog, rediscovers installed Bonsai/Jan/PATH engines, attaches to an existing Bonsai service or starts one authenticated `llama-server`, shows live VRAM, exposes exact launch arguments, and offers durable multimodal chat and Computer Tasks.
- **Research** runs a Bonsai-specific two-tool harness, visibly reports six stages, validates every citation, finds related prior work, and publishes immutable editions.
- **Video** turns a prompt into a durable, reviewable production plan, then owns one exact local ComfyUI process while it serially generates, verifies, retries, and optionally assembles clips.
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
Snapshot:    2024-01-12
Articles:    6,863,660
```

Kestrel reads these assets in place. It does not copy the model or 102.3 GiB archive. A first-run model rescan also checks common Jan, LM Studio, Hugging Face, and Ollama locations plus user-added roots.

Subsequent starts merge an integrity-checked `model-catalog.json` cache with an immediate Bonsai scan, then refresh all configured roots in the background. Missing or size-changed weights are discarded from the cache. A corrupt cache is quarantined and rebuilt without blocking startup.

## Runtime design

One `RuntimeManager` owns Kestrel-managed model processes and one semaphore owns inference. Research and chat share that lease, so a 12 GiB GPU cannot accidentally receive duplicate Kestrel loads or simultaneous generations. If the installed Bonsai endpoint is already healthy, Kestrel attaches without launching another model.

Managed launches bind to `127.0.0.1`, use a random session API key, one slot, strict full-GPU placement, no prompt RAM cache, no silent fit/offload, and Bonsai-specific Q4 KV plus flash attention. The API key is redacted from the visible launch proof.

The validated advanced Bonsai profile is 98,304 context tokens and 32,768 maximum response tokens. Advanced mode deliberately adds no Kestrel upper caps to context, output, lane count, result count, source target, tool turns, thinking budget, or source excerpt size.

> Warning: invalid or oversized values can stop startup or exhaust VRAM. Runtime and hardware limits still apply.

## Offline Video Studio

Video Studio separates planning from generation. A selected local llama.cpp model may create a compact story bible and chapter outline; deterministic native code expands it into a bounded clip ledger, so even a multi-hour request does not ask the model to emit thousands of records. The user reviews runtime, clip count, model preset, offload policy, retry limit, failure boundary, runtime boundary, disk reserve, and individual unfinished clip prompts before ComfyUI starts.

Four explicit RTX 5070 12 GiB profiles are supported:

- **Wan 2.1 1.3B GPU only** is the fast medium-quality path. Kestrel launches ComfyUI with `--gpu-only`, disables asynchronous and dynamic offload, and fails the job rather than silently changing the timing policy.
- **Kandinsky 5 Lite Distilled** is the recommended 16-step daily driver. Its declared resident profile permits only predictable stage-boundary movement.
- **Kandinsky 5 Lite SFT** uses the same declared memory profile with 100 quality-first steps.
- **Wan 2.2 TI2V 5B** always uses the declared low-VRAM asynchronous-offload profile.

Kestrel validates required local model files before enabling a preset. It refuses an existing unowned service on port 8188 because its launch flags cannot be proven. During generation it copies each completed clip into the project directory, verifies size and SHA-256, retries within the reviewed limit, restarts its owned backend between retries, and pauses at any failure, runtime, disk, or cancellation boundary. Final assembly trims the native-clip sequence at the reviewed target runtime. Interrupted projects become explicitly resumable after restart and never auto-resume.

ComfyUI stays warm across the entire serial batch, then Kestrel releases it at every terminal or paused state before llama.cpp can reclaim the GPU. This keeps generation fast within a batch without allowing two model runtimes to contend afterward.

Configure the ComfyUI root from Video Studio. The default tested root is `D:\AI\ComfyUI`; FFmpeg is optional unless final assembly is enabled. Projects live under `Kestrel Research\video-studio\projects` with open JSON state and ordinary video files.

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
|-- video-studio\          # durable plans, verified clips, logs, final assemblies
`-- reports\YYYY\MM\<title>--<id>\
    |-- index.html        # self-contained, printable research page
    |-- report.json       # complete structured edition
    |-- sources.json      # inspected evidence ledger
    `-- provenance.json   # model, archive, query, profile, lineage
```

Publication builds a complete staging directory and atomically renames it into place. Existing IDs are never overwritten. At startup Kestrel repairs missing derived HTML/source/provenance files from `report.json` and repopulates a missing SQLite catalog. If SQLite cannot be initialized, it is preserved with a timestamped `.corrupt-*` name before a clean cache is built from reports.

This keeps thousands of editions searchable through FTS5 while local models and ordinary tools can always traverse JSONL and report folders directly.

Chat and Computer Tasks can attach any regular local file up to 128 MiB. Kestrel copies it into a SHA-256-addressed object store before inference. Images and audio use loopback-only llama.cpp multimodal content blocks when the selected GGUF projector advertises those modalities. PDF, DOCX, PPTX, XLSX, source code, markup, logs, and common text formats receive bounded local text extraction. Unknown binaries remain durable and clearly marked metadata-only; Kestrel never claims the model read content it could not decode. Computer Tasks can request additional ranges from a declared attachment through a read-only typed tool.

Portable setup profiles contain research/runtime tuning, path-independent model identities, and local path hints. They never include weights, chats, research, credentials, developer paths, or Full Access authority. Import validates a bounded JSON file, keeps local developer/workspace paths, locks Full Access, rediscovers a local engine, and rescans weights before returning control.

## Offline boundary

- The WebView CSP permits no external network connection or remote asset.
- Native research clients accept only fixed loopback services.
- Video generation accepts only Kestrel's fixed loopback ComfyUI endpoint and never attaches to an unowned backend whose offload policy cannot be proven.
- Kiwix runs with external access blocked.
- Research receives only its two citation tools. Ordinary chat receives no mutation tools. Computer Tasks receives only typed, policy-checked tools; attachment reads are restricted to files explicitly selected for that task.
- Codex is isolated to `developer.rs`, requires explicit confirmation, cannot run during research, creates no commit, and is not required for diagnostics or any offline feature.

Wikipedia is a tertiary starting point with a January 2024 cutoff. Reports expose the inspected evidence and open questions; they do not claim finality.

## Run and verify

Requirements are Windows 10/11, Node.js 20.19+ or 22.12+, Rust stable with MSVC, WebView2, and the local assets above.

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
```

The in-app Developer screen runs the deterministic checks offline. Its optional Codex repair uses the same contract captured in `AGENTS.md`, but code, tests, recovery paths, and actionable errors remain the primary maintenance surface.

## Next useful improvements

- Add artifact hashes and an in-app integrity/rebuild action.
- Supply the release signing certificate and validate the signed offline installer on a clean disconnected Windows VM.
- Add local papers/books/document adapters behind the same evidence contract.
- Add collection/tag and parent/child comparison views after sustained library use.
- Capture firewall evidence for a complete strict offline run.

MIT. Bonsai, llama.cpp, Kiwix, Wikipedia content, and bundled components retain their own licenses and provenance.
