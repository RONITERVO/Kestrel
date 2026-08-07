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
  |-- MovieStudio: durable orchestration + non-destructive edit decisions
  |     |-- same single Bonsai inference lease for creative direction
  |     |-- at most two sequential archive tools; no parallel calls
  |     |-- ComfyUI MiniMax H3 on fixed 127.0.0.1:8188
  |     |-- immutable native-audio MP4 masters + continuity stills
  |     `-- FFmpeg first-cut and edited exports
  `-- DeveloperAssistant: optional, user-triggered Codex child
```

`work_active` is the process-wide strict lock for chat, research, Computer Tasks, and movie production. Model changes, runtime restarts, native diagnostics, and Codex repair are rejected while work is active. All Bonsai inference is additionally serialized by `RuntimeManager`; MiniMax H3 begins only after the lease is returned and Bonsai is stopped.

The research prefix has only `search_archive` and `read_source`. Candidate references are compact shared memory, not evidence. Citation IDs are issued on successful reads and validated natively. FTS similarity proposes existing editions; deterministic code owns report IDs, parent linkage, edition numbers, and output paths.

Publication writes every artifact into a hidden staging directory, then renames that complete directory into its immutable final path before indexing. Startup treats report JSON as truth, regenerates missing derived artifacts, and rebuilds missing/broken catalog acceleration without changing report identity.

The optional developer boundary executes fixed diagnostic commands without a shell. Codex uses `codex exec --ephemeral --ignore-user-config --sandbox workspace-write`, receives its prompt over stdin, runs only inside the validated Git root, and leaves changes uncommitted. No research or runtime module imports it.

Attachments are imported through a native picker and copied before use; original paths are not the durable source of truth. Chat messages and task records reference immutable attachment metadata. Native media travels only as base64 data on the authenticated loopback request. Extracted document text is bounded per object and per turn. The Computer Tasks `read_attachment` tool accepts only IDs declared on that task, so it cannot become an arbitrary path-reading bypass.

Movie projects use `project.json` as recoverable truth and retain the exact user request separately in `request.json`. Planning can expose only `search_archive` and `read_source`, with parallel tool calls disabled and six bounded turns. Search listings never enter the evidence ledger. The final plan is schema-constrained, but the director contract contains no example screenplay or genre/content guardrails.

Rendering uses a code-owned MiniMax H3 graph rather than a mutable Comfy template. Every clip has explicit prompt, duration, seed, dimensions, step count, and model filenames. A completed Comfy output is copied under `movies/<id>/raw` before its state becomes complete. When Bonsai marks a transition as continuous, FFmpeg preserves the prior final frame and the next H3 `fl2va` graph receives it through `first_frame`; ordinary cuts remain independent. Reference-conditioned clips instead use the native `ref2va` graph and cannot mix that separate path with prior-frame conditioning.

Producer media has its own bounded SHA-256 object store under `movies/_references`, sized for multi-gigabyte video rather than chat attachments. FFprobe validates stream type and H3 duration before import. Project creation re-hashes each object and hard-links or copies an immutable snapshot under `movies/<id>/references`; `request.json`, `references.json`, and `project.json` preserve the producer descriptions and IDs. Bonsai receives the original user message unchanged plus a text-only reference manifest and chooses stable asset IDs per clip. Native code rejects invented IDs, ensures attached assets are not silently unused, owns one-based H3 prompt labels, and owns Comfy's dotted zero-based V3 autogrow paths. Raw media never enters Bonsai context. Startup changes incomplete projects to `interrupted`, and resume skips every valid completed master. Edits store enabled/order/trim/audio-gain decisions, then create new exports while raw clips remain unchanged.
