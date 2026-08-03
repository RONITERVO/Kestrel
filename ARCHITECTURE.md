# Kestrel architecture

The executable is one Tauri application with six explicit native authorities:

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
  |-- VideoManager + VideoStore
  |     |-- exact owned ComfyUI process/profile on 127.0.0.1:8188
  |     |-- durable plans, clip ledger, retries, hashes, and restart recovery
  |     `-- optional local FFmpeg concat after verification
  `-- DeveloperAssistant: optional, user-triggered Codex child
```

`work_active` is a process-wide strict lock. Research, Computer Tasks, and video execution claim it; model changes, runtime restarts, native diagnostics, and Codex repair are rejected while GPU work is active. Research/chat inference is additionally serialized by `RuntimeManager`. Before video execution Kestrel waits for that lease, stops its managed llama.cpp process and Bonsai service, then gives the GPU to one exact ComfyUI child.

The research prefix has only `search_archive` and `read_source`. Candidate references are compact shared memory, not evidence. Citation IDs are issued on successful reads and validated natively. FTS similarity proposes existing editions; deterministic code owns report IDs, parent linkage, edition numbers, and output paths.

Publication writes every artifact into a hidden staging directory, then renames that complete directory into its immutable final path before indexing. Startup treats report JSON as truth, regenerates missing derived artifacts, and rebuilds missing/broken catalog acceleration without changing report identity.

The optional developer boundary executes fixed diagnostic commands without a shell. Codex uses `codex exec --ephemeral --ignore-user-config --sandbox workspace-write`, receives its prompt over stdin, runs only inside the validated Git root, and leaves changes uncommitted. No research or runtime module imports it.

Attachments are imported through a native picker and copied before use; original paths are not the durable source of truth. Chat messages and task records reference immutable attachment metadata. Native media travels only as base64 data on the authenticated loopback request. Extracted document text is bounded per object and per turn. The Computer Tasks `read_attachment` tool accepts only IDs declared on that task, so it cannot become an arbitrary path-reading bypass.

Video planning and execution are deliberately separate state transitions. Planning may take a serialized local-model lease but never starts ComfyUI. Native code bounds and expands the hierarchical outline into clip records. Execution refuses unowned port 8188, launches the selected immutable argument profile, submits native ComfyUI graphs serially, validates output paths against the ComfyUI output root, copies and hashes completed artifacts, and persists every transition atomically. On restart, in-flight clips/projects are marked interrupted and remain stopped until an explicit resume.
