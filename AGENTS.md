# Kestrel maintenance contract

Read this before editing. The UI maintainer may not know Rust; keep backend behavior explicit in code and error messages.

## Non-negotiable invariants

- Research must complete with the public network unavailable and without Codex.
- Research HTTP is fixed to loopback Bonsai and Kiwix endpoints. Never add a remote fallback.
- `RuntimeManager` owns the only Kestrel model process and its semaphore owns the only inference slot. Research, chat, and Computer Tasks must acquire that gate.
- Ordinary chat is tool-free. Computer Tasks is the only local-model path with mutation authority; workspace access is the default and full access requires two explicit user opt-ins.
- Chat sessions and task transcripts are durable user data. Never silently discard them or auto-resume an interrupted computer task.
- Codex exists only in `developer.rs`, is user-triggered, repository-scoped, ephemeral, uncommitted, and unavailable during research.
- Search results are not evidence. Only successfully opened sources receive citation IDs; native code validates citations before publication.
- Published report IDs are immutable. Expansion creates a child edition.
- Report JSON, sources, provenance, HTML, and JSONL are durable truth. SQLite is a rebuildable search cache.
- Do not add benchmarking, leaderboards, autonomous labs, analytics, or background network work.

## Backend map

- `lib.rs`: Tauri commands, strict research lock, state boundaries.
- `runtime.rs`: attach/start/stop model runtime and single inference lease.
- `attachments.rs`: content-addressed local files, bounded extraction, and capability-gated media blocks.
- `chat.rs`: cancellable SSE chat stream; never add tools here.
- `agent.rs`: bounded Computer Tasks loop, typed tools, path policy, recovery copies, visible events.
- `workspace.rs`: recoverable chat/task JSON and restart recovery.
- `harness.rs`: Bonsai-specific two-tool research loop and native citation validation.
- `kiwix.rs`: bounded local Wikipedia search/read and URL validation.
- `store.rs`: immutable directory-atomic bundles, recovery, FTS catalog.
- `model.rs`: bounded read-only GGUF discovery/metadata plus the disposable recoverable model cache.
- `profile.rs`: bounded portable setup import/export; never restore developer paths or Full Access.
- `config.rs`: recoverable settings and explicit Bonsai runtime application.
- `services.rs`: installed Bonsai/Kiwix scripts and live GPU telemetry.
- `studio.rs`: durable Bonsai movie direction, bounded archive tools, direct ComfyUI H3 graphs, recovery, media, and FFmpeg edits.
- `developer.rs`: optional Codex maintainer plus fixed offline diagnostics.

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
