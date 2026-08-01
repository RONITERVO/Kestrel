# Kestrel architecture

The executable is one Tauri application with four explicit native authorities:

```text
React WebView
  | typed IPC and progress events
  v
Rust application boundary
  |-- RuntimeManager: one process + one inference semaphore
  |     |-- attach existing Bonsai on 127.0.0.1:8080
  |     `-- or authenticated managed llama-server on loopback
  |-- ResearchHarness: two-tool Bonsai loop + citation validation
  |     |-- Kiwix on 127.0.0.1:8085
  |     `-- ResearchStore
  |           |-- immutable directory-atomic HTML/JSON bundles
  |           |-- catalog.jsonl durable index
  |           `-- SQLite FTS5 rebuildable cache
  |-- Services: installed scripts and NVIDIA telemetry
  |-- AttachmentStore: immutable local objects + bounded document extraction
  |     `-- native image/audio blocks only for advertised projector capabilities
  `-- DeveloperAssistant: optional, user-triggered Codex child
```

`research_active` is a process-wide strict lock. Model changes, runtime restarts, native diagnostics, and Codex repair are rejected while research is active. Research/chat inference is additionally serialized by `RuntimeManager` so no Kestrel path creates concurrent GPU generations.

The research prefix has only `search_archive` and `read_source`. Candidate references are compact shared memory, not evidence. Citation IDs are issued on successful reads and validated natively. FTS similarity proposes existing editions; deterministic code owns report IDs, parent linkage, edition numbers, and output paths.

Publication writes every artifact into a hidden staging directory, then renames that complete directory into its immutable final path before indexing. Startup treats report JSON as truth, regenerates missing derived artifacts, and rebuilds missing/broken catalog acceleration without changing report identity.

The optional developer boundary executes fixed diagnostic commands without a shell. Codex uses `codex exec --ephemeral --ignore-user-config --sandbox workspace-write`, receives its prompt over stdin, runs only inside the validated Git root, and leaves changes uncommitted. No research or runtime module imports it.

Attachments are imported through a native picker and copied before use; original paths are not the durable source of truth. Chat messages and task records reference immutable attachment metadata. Native media travels only as base64 data on the authenticated loopback request. Extracted document text is bounded per object and per turn. The Computer Tasks `read_attachment` tool accepts only IDs declared on that task, so it cannot become an arbitrary path-reading bypass.
