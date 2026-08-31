# Kestrel architecture

## Repository and ownership

Kestrel is a Rust-owned application with a React desktop view. The governing rule is:

> **TypeScript never owns application truth.**

Durable models, state transitions, retry policy, runtime/model state, scheduling decisions, asset
metadata, persistence rules, FFmpeg operations, ComfyUI process ownership, and local-model
orchestration belong in Rust. TypeScript may own only the current view: component state, forms,
rendering, interaction, animation, and unapplied drafts. Cross-boundary types are generated from
Rust and imported through the desktop contract facade.

```text
apps/
  desktop/                  React view and Tauri frontend build
    src/app/                composition, navigation, global styles
    src/features/           feature-owned UI and view helpers
    src/shared/             reusable presentation/collaboration components
    src/platform/           typed IPC adapter
    src/contracts/          generated-contract facade and shrinking legacy quarantine

crates/
  app-core/                 Rust-owned durable and IPC-visible core contracts/policies

packages/
  generated-bindings/       generated TypeScript; never hand-edited

src-tauri/                  native composition root and domain implementations
scripts/                    verification, binding generation, packaging, acceptance harnesses
```

`src-tauri` deliberately remains one composition crate while its most safety-sensitive authorities
share the process-wide work lock and `RuntimeManager` inference semaphore. Splitting it into many
crates before ports and ownership are explicit would hide cycles without creating isolation. New
pure durable contracts go into `crates/app-core`; the next extraction candidates are research,
workspace storage, runtime/model management, and Studio project models. FFmpeg and ComfyUI become
adapter crates only when their process APIs no longer reach through Studio internals.

The generated boundary is operational, not aspirational:

1. Add or change the Rust type in `crates/app-core` and derive `TS`.
2. Run `npm run bindings:generate`.
3. Import it through `apps/desktop/src/contracts/index.ts`.
4. `npm run check` rejects stale generated files, direct facade bypasses, duplicate legacy
   declarations, and additions to the quarantined handwritten contract set.

The quarantine currently contains pre-existing Studio, speech, download, and GPU DTOs. It may only
shrink as those types move to Rust generation; adding a new handwritten application contract is an
architecture-check failure.

The executable is one Tauri application with ten explicit native authorities:

```text
React WebView
  | typed IPC and progress events
  v
Rust application boundary
  |-- RuntimeManager: one authenticated managed llama-server + one inference semaphore
  |     |-- app-wide runtime defaults with optional per-model exceptions
  |     `-- model/workspace selection resolved before every lease
  |-- ResearchHarness: selected-model two-tool loop + citation validation
  |     |-- Kiwix on 127.0.0.1:8085
  |     `-- ResearchStore
  |           |-- immutable directory-atomic HTML/JSON bundles
  |           |-- catalog.jsonl durable index
  |           `-- SQLite FTS5 rebuildable cache
  |-- LocalSpeech: setup-owned or producer-selected ComfyUI on 127.0.0.1:8188
  |     |-- Chatterbox: content-addressed low-bitrate speech plus producer-owned Voice Library conditioning for Research, chat, Tasks, and Studio
  |     |-- Whisper: rolling dictation plus durable word alignment for click-to-seek playback
  |     `-- no browser, operating-system, public-network, or remote model fallback
  |-- Services: installed scripts and NVIDIA telemetry
  |-- AttachmentStore: immutable local objects + bounded document extraction
  |     `-- native image/audio blocks only for advertised projector capabilities
  |-- ModelDownloadManager: explicit public Hugging Face GGUF transfer
  |     |-- bounded repository inspection + HTTPS host allowlist
  |     `-- durable byte ranges, checksums, recovery, and managed model root
  |-- MovieStudio: producer-owned story, scenes, media bindings, and non-destructive edits
  |     |-- tool-free story/scene collaborators through the single inference lease
  |     |-- immutable Markdown revisions, scene-history snapshots, and recoverable conversations
  |     |-- accepted story plus only explicitly selected full scene-card context
  |     |-- producer-selected references, exact audio, first/last frames, renders, and exports
  |     |-- ComfyUI MiniMax H3 on fixed 127.0.0.1:8188
  |     |-- immutable native-audio MP4 masters + continuity stills
  |     `-- FFmpeg first-cut and edited exports
  |-- ImageStudio: producer-owned composition + immutable local PNG takes
  |     |-- any selected local GGUF proposes structured compositions through the single inference lease
  |     |-- native ComfyUI Ideogram 4 graph on fixed 127.0.0.1:8188
  |     `-- exact prompt/graph/model/license/seed/hash receipts; no arbitrary imported workflows
  |-- MusicStudio: producer-owned arrangement + immutable local song takes
  |     |-- any selected local GGUF proposes descriptions or lyrics through the single inference lease
  |     |-- native ComfyUI MiniMax Music 3 on dedicated 127.0.0.1:8189
  |     |-- exact graph/model/seed/hash receipts; no synthetic claim of separate stems
  |     `-- optional explicit MuScriptor audio-to-MIDI transcription with local gated weights
  `-- DeveloperAssistant: optional, user-triggered Codex child
```

`work_active` is the process-wide strict lock for chat, research, Computer Tasks, movie production, image production, music production, local speech, producer-triggered competing-process VRAM cleanup, and the explicit model-download network exception. Model downloads therefore cannot overlap strict research, and an interrupted transfer never auto-resumes. Model changes, runtime restarts, native diagnostics, VRAM cleanup, and Codex repair are rejected while work is active. VRAM cleanup is never automatic: native code obtains a bounded `nvidia-smi` preview, lets the producer exclude any default candidate or explicitly include a common app from Advanced, receives only that approved PID set, then revalidates it. Kestrel-owned, Windows-critical, undisclosed, and graphics-driver processes are never eligible. The first termination attempt uses fixed non-force arguments. A failed process can enter GpuClean-equivalent `taskkill /F` only through a second producer confirmation; native code rescans, requires the same PID, name, and executable path, and repeats the critical-process check. A copyable administrator PowerShell command appears only after that exact in-app force attempt fails. No path resets the driver or uses a shell. All local-model inference is additionally serialized by `RuntimeManager`; story, scene, summarization, image, and music requests take turns through that one lease. ComfyUI generation begins only after the language-model lease is returned and the runtime is stopped. The speech boundary remembers that model and restores it only after the user stops playback or the final dictation pass has durably completed, avoiding duplicate VRAM residents and stop/start churn between rolling Whisper passes.

Setup owns one native reusable-model catalog for the release profiles rather than duplicating filenames in React. Its bounded recursive scan matches only known filenames and exact sizes; every distributable selected source then passes the pinned SHA-256 check before it can reach a canonical runtime path. The separately gated MuScriptor checkpoint is instead size/format checked and receives a recorded local hash after explicit acceptance. Same-volume sources are hard-linked when Windows allows it, cross-volume sources are copied and re-verified, and recoverable replacement preserves any prior destination until the verified model is live. Component download estimates subtract recognized local assets so a producer is never told to redownload weights that are already present.

Local speech is opt-in at every response. Hidden reasoning, system prompts, tool schemas, and raw tool arguments are never narrated. User microphone capture uses the WebView only as a bounded 32-kbit/s recorder; recognition is exclusively the setup-installed OpenAI Whisper checkpoint behind Kestrel's owned ComfyUI adapter. Provisional passes may revise the visible draft while the user speaks, and the final whole-recording pass owns the durable transcript, segment timings, word timings, compressed audio, and recoverable JSON sidecar. Assistant Chatterbox output is cached as 64-kbit/s Opus with a receipt under its source kind and durable source ID. The speech engine/model pack and the producer-facing voice identity are separate typed values. Custom voice references require an explicit rights confirmation, live in a SHA-256-addressed private object store, are reverified before use, and condition the pinned Chatterbox node through its native `audio_prompt`; profile ID, reference hash, performance preset, engine revision, and model identity all participate in deterministic cache and receipt provenance. Playback begins immediately with duration-weighted timing when needed, then a background Whisper pass aligns and durably records exact words. Clicking the passage scrubber or a visible word changes `currentTime` on that original Opus; it never creates, transcodes, or substitutes a seek copy. One shared UI playback owner prevents overlapping voices. The native alignment request and receipt remain adapter boundaries so a future offline forced aligner can improve timing without changing product UIs or durable paths.

The research prefix has only `search_archive` and `read_source`. Candidate references are compact shared memory, not evidence. Citation IDs are issued on successful reads and validated natively. FTS similarity proposes existing editions; deterministic code owns report IDs, parent linkage, edition numbers, and output paths.

Publication writes every artifact into a hidden staging directory, then renames that complete directory into its immutable final path before indexing. Startup treats report JSON as truth, regenerates missing derived artifacts, and rebuilds missing/broken catalog acceleration without changing report identity.

The optional developer boundary executes fixed diagnostic commands without a shell. Codex uses `codex exec --ephemeral --ignore-user-config --sandbox workspace-write`, receives its prompt over stdin, runs only inside the validated Git root, and leaves changes uncommitted. No research or runtime module imports it.

Attachments are imported through a native picker and copied before use; original paths are not the durable source of truth. Chat messages and task records reference immutable attachment metadata. Native media travels only as base64 data on the authenticated loopback request. Extracted document text is bounded per object and per turn. The Computer Tasks `read_attachment` tool accepts only IDs declared on that task, so it cannot become an arbitrary path-reading bypass.

Movie projects use `project.json` as the recoverable renderer/edit projection and retain exact starting material and settings in `request.json`. `producer/workspace.json` owns current story pointers, conversation summaries, and scene cards; complete story revisions, scene snapshots, and conversations live beside it as recoverable or immutable records. Story collaboration is one tool-free complete Markdown replacement per turn. Scene collaboration is a bounded JSON operation response: native code validates revision, stable IDs, durations, sizes, and count before changing text fields. The accepted story is always in context, only producer-selected scenes are expanded, and references/frame/audio choices never enter model context. Exact module ownership, lifecycle transitions, durable artifacts, and cross-boundary contracts are documented in [`src-tauri/src/studio/README.md`](src-tauri/src/studio/README.md).

Rendering uses a code-owned MiniMax H3 graph rather than a mutable Comfy template. Every clip has explicit prompt, duration, seed, dimensions, step count, and model filenames. A completed Comfy output is copied under `movies/<id>/raw` before its state becomes complete. Producers choose previous-scene, reference-image, or independent first/last frames and select native visual/exact-audio reference bindings. Rust injects their exact H3 requirements immediately before graph construction. Reference-conditioned clips use the native `ref2va` graph and cannot mix that separate path with unsupported frame conditioning. Any renderer-affecting scene edit moves the old current master into immutable versions and queues a new render.

Image projects use recoverable `images/<id>/project.json` as truth. The producer owns the high-level description, exclusive photo/art style, background, palettes, exact text, normalized element boxes, layer order, seed policy, and batch size; a selected local language model can stream an unapplied structured proposal but cannot save or render it. Native code validates those fields, serializes Ideogram's order-sensitive schema as compact JSON, and compiles the fixed Ideogram 4 graph shipped by current ComfyUI. A one-, two-, or four-image batch becomes successful only after every expected full-resolution PNG is copied, dimension-checked, hashed, and registered as a separate immutable take. Each take records the exact prompt text and parsed view, graph, model filenames, pinned non-commercial agreement revision, prompt ID, seed, dimensions, byte size, and SHA-256. A prior take may be shown behind layout guides for visual alignment, but it is never conditioning input. Startup marks every unfinished batch take interrupted instead of resubmitting it. Ideogram 4 remains an explicit non-commercial Setup option rather than part of Kestrel's distributable commercial production suite.

Music projects use recoverable `music/<id>/project.json` as truth. Arrangement sections, song description, lyrics, and advanced generation settings remain editable producer data. Native code compiles those fields into the installed MiniMax Music 3 caption-and-lyrics contract and submits a code-owned ComfyUI graph; a selected local language model may propose either field but cannot render or apply its proposal. A successful lossless stereo master is copied into the project before a take becomes complete, while prior takes remain immutable. Each take records the exact graph, resolved model filenames, prompt ID, seed, duration, size, and SHA-256. Startup marks unfinished projects and takes interrupted instead of resuming them. MuScriptor remains a separate fixed-argument audio-to-MIDI adapter because its gated non-commercial weights are not a music generator and cannot be bundled or silently downloaded. Setup may prepare a pinned isolated GPU runner only after the producer accepts the official conditions and explicitly supplies the completed checkpoint; native transcription then forces that managed runner offline. Native code parses each completed Standard MIDI File into a bounded typed piano-roll document. The transcription source is immutable; every producer save creates a new MIDI, edit JSON, and receipt revision. The frontend can edit only that validated document and cannot replace paths, hashes, sources, or prior revisions.

Producer media has its own bounded SHA-256 object store under `movies/_references`, sized for multi-gigabyte video rather than chat attachments. FFprobe validates stream type and H3 duration before import. Project creation re-hashes each object and hard-links or copies an immutable snapshot under `movies/<id>/references`; `request.json`, `references.json`, and `project.json` preserve descriptions and IDs. Imported reference media and its manifest never enter language-model context. The producer selects stable asset IDs per scene; native code rejects invented IDs, owns one-based H3 prompt labels, and owns Comfy's dotted zero-based V3 autogrow paths. Startup marks incomplete renders interrupted and never silently restarts them. The edit decision list has stable timeline-item IDs so one source may be split or repeated. Native validation resolves preserved scene versions and bounds trim, speed, fades, gain, quality preset, and loudness target before constructing the FFmpeg filter graph. Exports are unique immutable MP4s; an append-only project ledger plus colocated JSON sidecar records the exact decisions, duration, size, and SHA-256 identity while raw clips remain unchanged.
