# Kestrel maintenance contract

Read this before editing. The UI maintainer may not know Rust; keep backend behavior explicit in code and error messages.

## Non-negotiable invariants

- Research must complete with the public network unavailable and without Codex.
- Research HTTP is fixed to loopback Bonsai and Kiwix endpoints. Never add a remote fallback.
- `RuntimeManager` owns the only Kestrel model process (llama-server, or Strata's server with its engine inside one Windows job object) and its semaphore owns the only inference slot. Research, chat, and Computer Tasks must acquire that gate.
- Ordinary chat is tool-free. Computer Tasks is the only local-model path with mutation authority; workspace access is the default and full access requires two explicit user opt-ins.
- Chat sessions and task transcripts are durable user data. Never silently discard them or auto-resume an interrupted computer task.
- Codex exists only in `developer.rs`, is user-triggered, repository-scoped, ephemeral, uncommitted, and unavailable during research.
- Search results are not evidence. Only successfully opened sources receive citation IDs; native code validates citations before publication.
- Published report IDs are immutable. Expansion creates a child edition.
- Report JSON, sources, provenance, HTML, and JSONL are durable truth. SQLite is a rebuildable search cache.
- Do not add benchmarking, leaderboards, autonomous labs, analytics, or background network work.

## Backend map

- `crates/app-core`: Rust-owned durable/core IPC contracts and generated TypeScript source of truth.
- `lib.rs`: Tauri commands, strict research lock, state boundaries.
- `models.rs`: compatibility re-export only; new application contracts belong in `crates/app-core`.
- `runtime.rs`: attach/start/stop model runtime and single inference lease.
- `strata.rs`: read-only Strata install discovery, validated loopback launch plans, and the job object that owns Strata's process tree. Strata-only GGUF architectures never reach llama.cpp.
- `structured_output.rs`: JSON-schema replies across engines; llama.cpp gets `response_format`, Strata gets the schema in the prompt, and native parsers stay the only authority. Schemas whose key order is part of the format (Ideogram 4 compositions) travel as literal JSON text, because `serde_json` sorts keys and llama.cpp builds its grammar in the order it receives.
- `speech_text.rs`: numbers, units, and percentages as spoken words; the only place digits become words before Chatterbox and Whisper.
- `narration.rs`: the optional narration mistake check (Whisper's words against the spoken words) and audio exports (narration joined sample-exactly, songs as M4A) through FFmpeg with fixed arguments.
- `timed_text.rs`: word-timed lines saved beside exported audio as enhanced LRC, WebVTT, JSON, and a self-contained word-by-word player page (`templates/word-player.html`).
- `attachments.rs`: content-addressed local files, bounded extraction, and capability-gated media blocks.
- `chat.rs`: cancellable SSE chat stream; never add tools here.
- `agent.rs`: bounded Computer Tasks loop, typed tools, path policy, recovery copies, visible events.
- `workspace.rs`: recoverable chat/task JSON and restart recovery.
- `harness.rs`: Bonsai-specific two-tool research loop and native citation validation.
- `kiwix.rs`: bounded local Wikipedia search/read and URL validation.
- `store.rs`: immutable directory-atomic bundles, recovery, FTS catalog.
- `model.rs`: bounded read-only GGUF discovery/metadata plus the disposable recoverable model cache.
- `model_download.rs`: explicit allowlisted Hugging Face GGUF inspection/download, durable byte-range recovery, and no automatic resume.
- `profile.rs`: bounded portable setup import/export; never restore developer paths or Full Access.
- `config.rs`: recoverable settings and explicit Bonsai runtime application.
- `services.rs`: installed Bonsai/Kiwix scripts and live GPU telemetry.
- `gpu_memory.rs`: bounded producer-triggered NVIDIA process preview and guarded VRAM cleanup.
- `studio.rs`: durable Bonsai movie direction, bounded archive tools, immutable producer references, direct ComfyUI H3 fl2va/ref2va graphs, recovery, media, and FFmpeg edits.
- `developer.rs`: optional Codex maintainer plus fixed offline diagnostics.

## Frontend map

- `apps/desktop/src/app`: desktop composition, the banner, and shared dialogs.
- `apps/desktop/src/app/book`: the Kestrel book: design tokens (the only place colors are defined), index-tab navigation, spreads, page turns, and the on-demand WebGL book. See `docs/KESTREL_BOOK.md`.
- `apps/desktop/src/features`: feature-owned UI and view-only helpers.
- `apps/desktop/src/shared`: reusable presentation components without application authority, including `shared/book` paging.
- `apps/desktop/src/platform`: Tauri IPC adapter.
- `apps/desktop/src/contracts`: re-exports of generated Rust contracts and values; no local declarations.
- `apps/desktop/src/preview`: development-only sample data; never application authority.
- `packages/generated-bindings`: generated from Rust; never edit by hand.

TypeScript never owns application truth. New durable state or IPC-visible contracts must originate
in Rust and be generated with `npm run bindings:generate`. There is no handwritten contract quarantine.
Command signatures in `lib.rs` and the Rust event registry also generate the IPC maps. Only
`platform/transport.ts` imports the native SDK. Rust must not include policy files from UI folders.
UI contributors should start with `apps/desktop/README.md`; documented boundaries and practical
exceptions are in `docs/UI_BOUNDARIES.md`. UI-only checks are available with `npm run ui:check`.

Prefer small typed modules, fixed command argument arrays, bounded reads, loopback-only URLs, recoverable file replacement, and actionable errors. Never directly execute model text: parse tool JSON, require absolute paths, resolve it through the selected access policy, reject wildcards, use argument arrays without a shell, and persist the result.

## Required verification

Run all of these after a change:

```powershell
git diff --check
cargo test --all-targets --manifest-path src-tauri\Cargo.toml
cargo clippy --all-targets --manifest-path src-tauri\Cargo.toml -- -D warnings
npm run check
npm test -- --run
npm run build
```

For harness/runtime changes, also run the applicable ignored live tests with local services available. Do not weaken a test to make a repair pass. Never commit automatically from the in-app Codex flow.
