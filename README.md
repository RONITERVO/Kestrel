# Kestrel Local

Kestrel is a Windows-first local model control plane and fully offline research workspace. The current model-specific path is Bonsai 27B plus an English Kiwix Wikipedia archive. It produces source-traceable research editions that remain usable as plain JSON and self-contained HTML without Kestrel, SQLite, Codex, or internet access.

The application has four adjacent workspaces:

- **Control** discovers GGUF files read-only, keeps a recoverable startup catalog, rediscovers installed Bonsai/Jan/PATH engines, attaches to an existing Bonsai service or starts one authenticated `llama-server`, shows live VRAM, exposes exact launch arguments, and offers durable multimodal chat and Computer Tasks.
- **Research** runs a Bonsai-specific two-tool harness, visibly reports six stages, validates every citation, finds related prior work, and publishes immutable editions.
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
`-- reports\YYYY\MM\<title>--<id>\
    |-- index.html        # self-contained, printable research page
    |-- report.json       # complete structured edition
    |-- sources.json      # inspected evidence ledger
    `-- provenance.json   # model, archive, query, profile, lineage
```

Publication builds a complete staging directory and atomically renames it into place. Existing IDs are never overwritten. At startup Kestrel repairs missing derived HTML/source/provenance files from `report.json` and repopulates a missing SQLite catalog. If SQLite cannot be initialized, it is preserved with a timestamped `.corrupt-*` name before a clean cache is built from reports.

This keeps thousands of editions searchable through FTS5 while local models and ordinary tools can always traverse JSONL and report folders directly.

Chat and Computer Tasks can attach any regular local file up to 128 MiB. Kestrel copies it into a SHA-256-addressed object store before inference. Images and audio use loopback-only llama.cpp multimodal content blocks when the selected GGUF projector advertises those modalities. PDF, DOCX, PPTX, XLSX, source code, markup, logs, and common text formats receive bounded local text extraction. Unknown binaries remain durable and clearly marked metadata-only; Kestrel never claims the model read content it could not decode. Computer Tasks can request additional ranges from a declared attachment through a read-only typed tool.

Computer Tasks offer per-task **Approve for me** and **Approve manually** policies. Both explain state-changing or externally visible native actions in the durable transcript. Manual mode pauses before every action. Approve for me automatically accepts only bounded changes contained by an approved workspace and escalates program launches, moves, external opens, protected metadata, and out-of-workspace effects to an exact one-time confirmation. Approval never expands the selected Workspace or Full Access boundary.

Portable setup profiles contain research/runtime tuning, path-independent model identities, and local path hints. They never include weights, chats, research, credentials, developer paths, or Full Access authority. Import validates a bounded JSON file, keeps local developer/workspace paths, locks Full Access, rediscovers a local engine, and rescans weights before returning control.

## Offline boundary

- The WebView CSP permits no external network connection or remote asset.
- Native research clients accept only fixed loopback services.
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
