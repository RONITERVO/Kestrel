# UI ownership audit and exception register

This document defines the maintained boundary, including cases where moving presentation work
into Rust would not be a practical long-term design. It complements [the UI contributor guide](../apps/desktop/README.md)
and [ARCHITECTURE.md](../ARCHITECTURE.md).

## Source of truth

All desktop DTOs originate in `crates/app-core`. There is no handwritten contract quarantine.
The facade only re-exports generated types and values. The native composition root's actual command
signatures generate `DesktopCommands`; `crates/app-core/src/events.rs` generates `DesktopEvents`.
The event adapter checks the payload against that registry before emission. Every registered
command is covered by the binding generator, and every command argument/result must implement
the Rust TypeScript export trait. Unknown signature forms fail the build instead of becoming `any`.

Private persistence envelopes, service handles, process state and vendor parsers may remain in
native feature modules. They already originate in Rust and do not become desktop contracts merely
because they are serializable. If the UI starts consuming a shape, it must move to the shared
contract crate and be generated. A raw-text artifact editor is discussed separately below.

## Feature audit

| Surface | UI responsibility | Native authority |
| --- | --- | --- |
| Research | Navigation, source rendering, citation links, search forms and progress display | `harness`, `kiwix`, `store`: tools, opened-source evidence, citation validation, immutable publication and catalog recovery |
| Chat | Composer drafts, streamed display, attachments selected by the user | `chat`, `attachments`, `workspace`, `runtime`: tool-free inference, bounded content extraction, durable sessions and inference lease |
| Computer Tasks | Access opt-in controls, questions, tool event/transcript views | `agent`, `workspace`, `lib`: path policy, tool validation, recovery copies, durable transcripts, explicit resume and full-access gates |
| Movie Studio | Story/scene drafts, reference selections, timeline geometry and unsaved undo | `studio/producer`, `producer_chat`, `editor`, `editor_timeline`, `export`: revisions, batches, reference validation, immutable sources and native placement/export |
| Image Studio | Canvas geometry, exact-copy editing, unsaved composition proposals and contact sheet | `studio/image_studio`: saveable-field whitelist, order-sensitive model caption, graph construction, generation and immutable receipts |
| Music, lyrics, MIDI | Arrangement, note/lyric drafts, audition and visualization | `studio/music`, `music_midi`: validation, source/take identity, immutable revisions, model calls, native MIDI and media exports |
| Speech and voices | Browser recording, excerpt preparation, device silence detection and playback | `local_speech`, `voice_library`, `speech_preferences`: voice consent/identity, immutable audio, synthesis/transcription, timing and durable settings |
| Settings, setup and downloads | Unapplied form values, progress and error messages | `config`, `setup`, `model_download`, `profile`, `prompt_catalog`: validation, allowed downloads, installation, recovery, import/export and explicit resume |
| Runtime and GPU | Visible estimates, polling status, producer selections and confirmations | `runtime`, `services`, `gpu_memory`, `lib`: model process/lease, real telemetry, guarded cleanup and execution |
| Developer tools | Explicit request and diagnostics/report display | `developer`: repository-scoped optional maintenance, fixed native diagnostics, research exclusion |

Native image/music saves now accept separate `ImageProjectEdit` / `MusicProjectEdit` shapes. Full
projects expose generated takes and receipts for display, but those fields cannot enter the edit
contract. Native save tests preserve take truth when a caller supplies a modified project.

## Deliberate exceptions and view-only work

| Case and location | Why it belongs here long term | Boundary that still applies |
| --- | --- | --- |
| Browser microphone and playback (`features/speech`, music visualizers) | `MediaRecorder`, Web Audio, decoded browser audio buffers and canvas are WebView-owned resources. Moving their live handles and 20–60 Hz UI interaction through IPC would add latency and couple native code to view lifecycles. | Browser audio is a user-provided draft until native import/transcription accepts it. The UI cannot create authoritative voice identity, consent, source paths, timing receipts or generated takes. |
| Saved speech preferences | No ongoing UI-storage exception exists. A read-only `platform/legacySpeechPreferences.ts` bridge imports the old five WebView keys for existing users. Keeping this bridge lets someone skip releases without losing their preferences. | Rust validates/imports only when native preferences do not exist, then owns recoverable `speech-preferences.json`. The bridge cannot write or clear browser storage; original keys are retained for older installations. |
| Timeline, MIDI, image canvas and lyric undo | A drag or keystroke must render immediately, and incomplete forms must remain editable. Transient undo history is not a durable revision ledger. | Requests use generated shapes. Rust validates the proposed edit, verifies native source identity and writes the accepted revision. UI undo cannot delete preserved media or change a job's completion state. |
| Speech text splitting and highlighting (`shared/speech`, `shared/components`) | These choose what passage is presented/read and where the current playback word is highlighted. They depend on the rendered view and browser playback clock. | Source reports/transcripts remain unchanged. Native speech accepts text as input and owns produced audio and alignment. These transformations cannot create report citations or rewrite evidence. |
| Form choices, runtime hints and local speed display | Curated size/quality choices, reference-picker hints and disabled buttons explain the current workflow. Forms can reject obviously invalid keystrokes; elapsed-time estimates are presentation. A synchronous display does not need a new model/runtime operation. | Runtime limits, producer defaults and saved speech defaults are Rust-generated. Native code revalidates dimensions, reference combinations, seed ranges and runtime policy before saving or running. UI hints cannot grant capability or decide whether the model slot is free; update the presentation when native capabilities change. |
| Image composition preview and pasted proposals | A producer must be able to inspect/edit an unsaved proposal in a browser without a model or a native round trip per keystroke. | The order-sensitive caption shapes originate in Rust and are generated. The UI preview is labeled as a draft; native `structured_prompt` constructs the exact inference string and saves it in the take receipt. UI proposal validation is an editing aid, never execution authority. |
| Clipboard collaborator exchange (`shared/collaboration`) | Copy/paste is explicitly user-triggered interaction with an external author. It transfers a draft, not a tool invocation. | Envelope/target/version/limits originate in Rust. The UI checks that a pasted response fits the selected field, then proposes a draft. Applying/saving/rendering still follows native commands and validation. |
| Raw JSON/text editors for profiles and prompt packs | A text editor must represent malformed, partial and future-version input without losing the user's text. Forcing that buffer into a valid typed DTO on every keystroke prevents normal editing. | IPC explicitly carries text. Rust owns the durable format and validates it before apply/import/export. `PromptPack` is generated for the visual editor; draft placeholder hints do not replace native validation. Unknown artifact fields are retained while editing. |
| Opaque `JsonValue` in exact graphs and old provenance | Vendor graphs and legacy payloads are versioned outside the UI and may contain unknown nodes/fields. A fixed UI schema would discard valid data when a vendor changes or a legacy project is saved. | Rust owns graph construction, preservation and execution allowlists. The UI displays provenance and cannot submit an arbitrary graph. Exact native artifacts remain the durable source. |
| Preview fixtures (`src/preview`) | UI contributors need a reproducible browser workspace without installing models, Rust or native services. Sample values are presentation material. | Handlers satisfy the generated command map, exist only in development, cannot be installed by a desktop window, and have no persistence or process/network access. Packaged builds do not load the preview entry point. |
| One native composition crate | The work lock and model lease are intentionally shared within one process. Splitting service modules into crates is useful only if it clarifies actual dependency ports. | All UI-facing types live in `app-core`; platform is one typed seam. Native crate count does not require a UI contributor to know Rust. |

## Serialization compatibility

Generated optional fields reflect the native distinction between an omitted key and a JSON `null`.
Do not erase `null` from a contract with a TypeScript assertion; a view can normalize it to its
own empty selection or placeholder.

The seed wire format preserves all 64 unsigned bits; each native renderer still enforces its
supported seed range. Values above JavaScript's exact integer range are serialized
as decimal strings; smaller values remain numbers. Rust accepts both, including numeric seeds in
older project files. Movie manifests now use version 8, image manifests version 3, and music
manifests version 2. New saves can contain string seeds and require this or a newer application;
older applications cannot reliably read those newly saved files. Opening an old file does not
bulk-convert the library. Native graph execution still uses exact integers. Opaque graph displays
are previews: use the native artifact for byte-exact vendor JSON, whose large numbers can otherwise
round in a browser. The UI never writes those displayed graphs back into generation receipts.

Other integer counters (bytes, ticks, revisions and elapsed time) are view numbers. Their bounded
local workloads stay below JavaScript's exact range; native counters and validations remain
authoritative. Do not use browser arithmetic to create durable identities or seeds.

## Enforcement and review

`architecture:check` checks imports/re-exports/dynamic imports, facade-only generated imports,
duplicate generated contract names, forbidden browser storage/network capabilities, the read-only
legacy bridge, native dependence on UI source files, raw native event emission, and commands
outside the composition root. Negative fixtures test those guardrails. `ui:check` also compiles the
typed command/event rejection examples. `bindings:check` compiles the actual native boundary and
compares regenerated content, normalizing Windows line endings.

Static checks prevent ordinary accidental drift; they are not a security sandbox or a proof of
every future semantic decision. Reviews must still ask whether a new helper is an unapplied view
calculation or application authority. Add a narrowly scoped exception here only with its exact
location, reason, native authority and verification. Do not add a general legacy allowlist or
restore a contract quarantine.

When a command is added, native registration and generation must change together. When an event
is added, define its name/payload once in the Rust registry. When a durable field changes, update
its Rust serialization/default/recovery behavior and regenerate bindings before changing the UI.
