# Studio maintainer guide

This directory contains every model-assisted part of Kestrel Studio. Start here before changing
Director/Reviewer planning, generative movie edits, prompt generation, Producer Copilot, H3 image assets, live previews, Image Studio, or Music. The root
`studio.rs` file remains the domain and persistence facade; child modules own one bounded concern
and must not acquire authority implicitly.

## Non-negotiable boundaries

- Studio works without the public network. Model HTTP is authenticated loopback traffic and ComfyUI
  is fixed to loopback.
- `RuntimeManager` owns the only local language-model process and inference semaphore. A Studio
  feature never starts a second model server or bypasses its lease.
- Model text is data. `movie_agent.rs` mutates only the planning workspace through its deserialized
  `WorkspaceAction` contract. `generation_agent.rs` writes only a checked candidate workspace;
  rendering and storyline placement remain separate, explicit producer actions. Producer Copilot
  proposes edits but cannot apply them.
- External collaboration is a manual data boundary, not a runtime provider. React workspaces may
  copy a bounded, versioned request for a producer to use elsewhere and parse the matching JSON
  response into an unsaved editable draft. Kestrel makes no network request, stores no external
  credentials, never executes returned text, and applies the same field-specific validation used
  for producer-authored content. External responses never receive Computer Tasks or movie-workspace
  mutation authority.
- `project.json`, immutable reference objects, raw masters, edit decisions, receipts, transcripts,
  and planning controls are durable user data. Interrupted work is surfaced, never silently resumed.
- The model receives the original producer request and a fresh authoritative workspace snapshot on
  every Director planning turn. UI/log redaction must never mutate model input or durable history.
- H3, Ideogram 4, and Music 3 rendering begin only after every language-model inference lease is released and the
  runtime is unloaded from the GPU.
- Director and Reviewer bindings are durable project data. Missing pinned models fail visibly; an
  explicit checkpointed role change records provenance and forces producer review.
- Language-model limits resolve in one order: System defaults, selected-model exceptions, then the
  durable movie-project policy. `MovieSettings::runtime_settings_for` is the only movie-layer
  resolver; planning, fresh review, frame analysis, generative edits, and Copilot must use it before
  acquiring a lease. Context changes may restart the one managed runtime; output changes are
  per-request. Never introduce a stage-specific hidden copy of either value.
- Every local model must pass the current local Studio protocol check before standard-mode unattended
  planning. Advanced mode may run an unverified compatible model only with forced producer review.

## Module ownership

| Module | Owns | Must not own |
| --- | --- | --- |
| `studio.rs` | Public Tauri-facing domain types, project persistence, job coordination, rendering and edit facade | Model stream framing or workspace action semantics |
| `agent_flow.rs` | Director planning orchestration, producer-control boundaries, per-turn model leases, tool dispatch, independent-review coordination | Wire parsing or file mutation rules |
| `generation_agent.rs` | Durable shot/transition candidate orchestration, typed workspace actions, two-check gate, fresh-context review, and visible events | Rendering, arbitrary media paths, or storyline mutation |
| `agent_lifecycle.rs` | Pure session, tool-use, and reviewer-budget transitions | HTTP, filesystem, UI events, or project state |
| `agent_protocol.rs` | Exact planning requests, lossless transcript history, assistant/tool-call assembly | Workspace mutation or producer copy |
| `model_stream.rs` | Shared OpenAI-compatible SSE framing, UTF-8 fragmentation, JSON validation, completion markers, and explicit reasoning-channel extraction | Tool schemas or producer-facing UI events |
| `movie_agent.rs` | Sandboxed movie workspace, typed actions/outcomes, native plan compilation and lint | OS commands, arbitrary paths, renderer execution, or network |
| `planning.rs` | Durable redirection/checkpoint controls and typed planning UI event contract | Model inference |
| `prompts.rs` | Planning, lint, resume, and repair prompt text exposed to advanced producers | Hidden control behavior |
| `prompt_collaboration.rs` | Story/image/reference/music-description/lyrics drafting from producer context | Applying proposals, movie-plan mutation, or rendering |
| `copilot.rs` | Timeline advice and validated, unapplied edit proposals | Applying edits or rendering |
| `image_assets.rs` | Durable H3 pseudo-image generations, graph/receipt provenance, imported candidates | Planning authority |
| `image_studio.rs` | Recoverable image projects, structured compositions, native Ideogram 4 graphs, immutable PNG takes, and progress | LLM process ownership, arbitrary imported workflows, bundled license rights, or public-network fallback |
| `live_preview.rs` | TAE preview graph nodes, bounded project-level reconnect state, and producer-visible preview events | Final-render truth or durable base64 preview storage |
| `music.rs` | Recoverable song projects, producer arrangement, native Music 3 graphs, immutable takes, durable lyric cue revisions, progress, and optional MuScriptor adapter | LLM process ownership, fake stem separation, bundled gated weights, or public-network fallback |
| `music_lyrics_model.rs` | Bounded audio-listening and translation suggestions through a caller-owned local-model lease | Durable lyric mutation, runtime ownership, remote fallback, or arbitrary tools |
| `music_midi.rs` | Bounded Standard MIDI parsing/writing, typed piano-roll documents, and recoverable binary replacement | MuScriptor execution, project path selection, source mutation, or UI state |

If a change appears to belong to two rows, introduce a typed boundary instead of importing private
implementation details across both modules.

The current lyric-sync path has strong extended producer results and should not acquire another
runtime speculatively. If a repeatable acceptance failure appears, follow the dormant, license-aware
plan in [`MUSIC_LYRIC_ALIGNMENT_CONTINGENCY.md`](MUSIC_LYRIC_ALIGNMENT_CONTINGENCY.md) rather than
adding an ad-hoc separator or remote service.

Visual lyric renderer ownership, source parity, audio-band semantics, fixed pool limits, and
interaction requirements are recorded in [`MUSIC_LYRIC_VISUALS.md`](MUSIC_LYRIC_VISUALS.md).

## Studio planning lifecycle

```text
open durable workspace
  -> start fresh context session
  -> consume queued producer directions/checkpoint
       -> checkpoint requested: persist and return
  -> append fresh authoritative story memory to request view
  -> stream one model turn
       -> transport failure: preserve transcript, start fresh context
       -> no workspace call: remind model; third consecutive miss starts fresh context
  -> deserialize WorkspaceAction
  -> execute bounded workspace operation and append typed outcome
       -> check failure: model repairs reported files
       -> first clean check: require whole-film reread
       -> second clean check: permit submit
  -> independent fresh-context review
       -> accepted: preserve canonical plan
       -> rejected: append exact findings and repair; third rejection fails visibly
```

`agent_lifecycle.rs` is the executable source of truth for thresholds and rollover behavior. Change a
threshold there, update its deterministic tests, and then verify that producer copy in
`agent_flow.rs` still describes the same behavior. Do not encode decisions by parsing display text.

Producer directions enter only between complete model/tool turns. This is the safe boundary that
allows redirection without corrupting an accepted assistant/tool-call pair. A checkpoint is graceful;
immediate cancellation remains a separate explicit producer action.

H3 preview frames are approximate process-local state. `LivePreviewRegistry` retains at most one
latest estimate for four movie projects, merges terminal status into the last picture, and lets a
remounted Studio query `get_movie_render_state`. Starting a new render clears the prior estimate;
once the registered render job is no longer active the estimate is discarded and the preserved
full-VAE master becomes the only picture shown. Project status and receipts remain the durable
restart boundary—never put large preview data URLs in `project.json`.

## Model transport

All Studio language-model streams pass response bytes through `OpenAiSseDecoder`. Features retain
their own request bodies and map decoded JSON into their own events, but must not implement another
`data:`/`[DONE]` parser. The decoder intentionally rejects malformed JSON, invalid UTF-8, duplicate
completion markers, and events after completion so token loss cannot look like success.

Explicit `reasoning_content` or `reasoning` deltas are streamed to the same bounded, provisional
thinking pane in prompt collaboration, Director planning and review, Producer Copilot, and the
Generative Director and Reviewer. They are never inferred from ordinary answer text, treated as a
production instruction, or copied into the model's durable tool transcript. A model that exposes no
separate channel is identified honestly in the UI.

Generative-edit sessions additionally append every reasoning, prose, and typed-tool fragment to the
bounded project-local `events.jsonl` journal before emitting it to the window. Attempt boundaries,
schema rejection details, finish reasons, and completion-marker state use the same sequence. Generate
replays that journal after navigation or restart and then reconciles it while a backend request is
active. The accepted transcript remains the model-input authority; the event journal is lossless
producer-facing evidence and must never be treated as an executable tool request.

When adding a compatible runtime variation:

1. Add fragmented-wire tests in `model_stream.rs`.
2. Keep feature-specific interpretation in the owning feature module.
3. Preserve partial producer-visible output and durable receipts on interruption.
4. Never add a public-network fallback.

## Durable planning artifacts

The project directory is the source of truth. Important planning files include:

| Artifact | Meaning |
| --- | --- |
| `request.json` | Exact producer request captured at project creation |
| `project.json` | Recoverable current project state |
| `planning-control.json` and recovery copy | Pending directions and graceful checkpoint request |
| `agent-workspace/BRIEF.md` | Current authoritative producer brief |
| `agent-workspace/REFERENCES.md` | Text-only immutable reference manifest |
| `agent-workspace/PRODUCER-NOTES.md` | Deduplicated live producer directions |
| `agent-workspace/movie.json` and `scenes/*.json` | Model-editable plan source compiled by native code |
| `agent-workspace/state.json` | Workspace revision and clean-check gate |
| `agent-workspace/agent-transcript*.json` | Lossless accepted conversation by context session |
| `agent-workspace/agent-last-request.json` | Exact last planning request envelope for audit |
| `agent-workspace/generative-edits/<request>/` | Exact task/context, candidate revisions, native-check state, transcript, fresh review, and accepted result |
| `generations/transition-*/graph.json` and `receipt.json` | Exact endpoint hashes, H3 graph, seed, placement decision, and immutable output hash |
| `agent-workspace/generative-edits/<request>/endpoint-frames/*.png` | Exact bounded endpoint pixels shown to the selected local Frame Analyst |
| `agent-workspace/generative-edits/<request>/frame-analysis*.json` | Vision-model request manifest, per-frame observations, uncertainties, model identity, hashes, and recoverable failure |
| `../../model-qualifications.json` | Recoverable protocol receipts bound to model, engine, runtime profile, and protocol revision |

Atomic replacement and recovery copies are deliberate. Do not trade them for in-memory convenience.
The advanced UI reads bounded redacted views; the unmodified files remain available as durable truth.

## Generative edit lifecycle

```text
select one storyline shot or two exact frame anchors in Generate
  -> producer writes renderer direction directly or opens a durable Generative Director session
  -> when a checked vision model is selected, native code extracts and hashes each exact endpoint PNG
  -> local Frame Analyst streams its reasoning and producer-readable observations, then records separate visible facts and uncertainties per endpoint
  -> append complete current story, plan, references, storyline, and selected source facts every turn
  -> typed generation_write_candidate writes one durable candidate
  -> two clean native checks on the unchanged candidate
  -> empty-object generation_submit references that durable candidate instead of streaming it again
  -> independent fresh-context Reviewer accepts or returns blocking repairs
  -> producer edits or discards the accepted candidate
  -> H3 writes a preserved audition plus exact graph/endpoint/output receipt
  -> producer explicitly chooses an audition or before/between/after placement
```

The React surface sends storyline edit IDs and source times, never absolute input paths. Native code
resolves the selected preserved version inside the project boundary before extracting a frame. Only
these bounded endpoint PNGs may enter the authenticated local vision-model request; the Director and
Reviewer receive the durable observations rather than a claim that they inspected pixels. The request
manifest records the exact image paths and hashes without duplicating base64, and resume reuses an
integrity-checked observation only for the unchanged anchor task. A shot
audition never rewrites the active master or approved plan. A transition defaults to the Masters bin and
changes the storyline only when the producer selected insert or replace before generation. An internal
shot replacement uses frame-aligned In and Out points: native placement splits the existing edit into
untouched leading and trailing decisions, clears only the obsolete fades at the new joins, and inserts the
generated master between them. The original source and every prior audition remain immutable.

## Music production lifecycle

Music uses the same resource boundary without inheriting movie-agent authority:

```text
open recoverable song project
  -> producer edits description, tagged sections, lyrics, and settings
  -> optional selected local GGUF streams an unapplied description or lyrics proposal
  -> producer applies, discards, redirects, or keeps the partial checkpoint
  -> persist project and unload the language-model runtime plus every retained ComfyUI model
  -> submit the native MiniMax Music 3 graph to its GPU-resident loopback ComfyUI service on 8189
  -> stream node phase, sample step, percentage, and ETA
  -> copy and hash the completed lossless stereo master inside the project
  -> append an immutable take and exact generation receipt, then call `/free` on both ComfyUI services
  -> optionally open the take in the visual lyric stage with an immediate producer-editable cue draft
  -> preview Living sketchbook or Signal bloom and preserve the chosen allowlisted visual theme
  -> optionally unload competing runtimes and use Kestrel Whisper on 8188 for local word timestamps
  -> preserve the sync receipt and each later timing/translation edit as an immutable JSON revision
```

`music/<uuid>/project.json` is recoverable truth. `takes/<uuid>.flac` and its graph receipt are immutable;
editing the arrangement never rewrites an older take. Startup changes active generations to
`interrupted` and never submits them again. MiniMax Music 3 produces a stereo master, so the arranger
may show semantic lanes for structure and lyrics but must not label generated audio as separate stems.
The visual lyric stage never sends the master or lyrics to another application or service. Draft cue
positions derive from the immutable take lyrics; local Whisper may replace them with sung transcript
segments and word timestamps only after the master SHA-256 is rechecked. Each sync creates a new
`lyrics/<take>/<sync>` session, and each producer edit appends a numbered JSON revision while the take
continues to point at the newest document. The audio-reactive canvas is presentation only and cannot
modify or masquerade as the preserved master. Visual themes share one 1024-point analyser and one
multi-band/transient/adaptive-beat frame with the lyric typography, live in separate bounded renderer modules,
and enter durable JSON only through the native theme allowlist; a theme is never downloaded code.
The shared H3/speech service remains on port 8188 with its conservative low-VRAM profile. Music uses
port 8189 with async weight offload disabled, dynamic VRAM as an OOM fallback, and one GiB reserved
for the desktop. The installed INT8 text encoder, INT8 DiT, and VAE run one stage at a time; Kestrel
does not pretend that all three can remain resident together on a 12 GiB GPU. MuScriptor startup first
releases both ComfyUI services and the local language model so its checkpoint cannot overlap them.
MuScriptor remains an explicit separate adapter. Setup can verify a producer-supplied gated large
checkpoint and prepare the pinned official package in an isolated NVIDIA runtime after explicit
license confirmation; manually supplied compatible runners remain supported. Native code validates
the paths, launches a fixed argument array, forces the managed runner offline, and preserves the
non-commercial license notice and output hash in its receipt.

The first successful transcription is copied into a unique `midi/<take>/<transcription>/source.mid`
and never edited. `music_midi.rs` parses metrical Standard MIDI into bounded tempo, time-signature,
track, program, and note records. Opening a legacy transcription migrates it through the same source
preservation boundary. Every piano-roll save writes a new numbered `.mid`, typed `.json`, and receipt;
the take points to the latest revision while older revisions remain addressable. Muted tracks are
omitted only from the exported revision, not deleted from the edit document. Native export uses a
producer-selected absolute `.mid`/`.midi` destination and recoverable replacement.

## Image production lifecycle

Image Studio uses the same resource boundary without inheriting movie-agent authority:

```text
open recoverable image project
  -> producer edits the brief, exclusive photo/art style, palette, exact text, layer order, and normalized layout boxes
  -> optional selected local GGUF streams an unapplied structured-composition proposal
  -> producer applies, discards, redirects, or keeps the partial checkpoint
  -> persist the project and unload the language-model runtime plus retained ComfyUI models
  -> serialize Ideogram's order-sensitive compact JSON and compile Kestrel's native graph on loopback ComfyUI port 8188
  -> stream node phase, sampling progress, percentage, and ETA
  -> require every expected batch PNG, then copy, dimension-check, and hash each inside the project
  -> append separate immutable takes with exact prompt/graph/model/license receipts, then call `/free`
```

`images/<uuid>/project.json` is recoverable truth. `takes/*.png` and each generation receipt are
immutable; changing the design never rewrites an older take. Startup marks an active generation
`interrupted` and never submits it again. The React surface sends typed producer fields, not ComfyUI
graphs. Native code owns node names, model filenames, limits, output extraction, and provenance.
The layout desk supports direct box drawing, overlap cycling, layer ordering, duplication, keyboard
nudge, extreme canvas presets, explicit seed modes, and one/two/four-image batches. A completed take
can be used as an opacity-adjustable alignment backdrop, but it is never sent as model conditioning.
ComfyUI Manager and KJNodes are not Image Studio runtime dependencies: those projects informed the
producer interaction, while Kestrel keeps the submitted workflow on versioned Comfy core nodes so a
custom-node update cannot silently alter an existing project contract.
Ideogram 4 is installed only after explicit acceptance of its pinned non-commercial agreement and is
not included in the commercial production-suite action. Kestrel's MIT license does not grant model
or output rights.

## Typed cross-boundary contracts

- `WorkspaceAction` is the only accepted tool action set.
- `WorkspaceOutcome` drives orchestration and producer status. Its human-readable `message` is model
  feedback, not a control channel.
- `PlanningEventKind` and `PlanningStage` are serialized Rust enums mirrored exactly by
  `src/types.ts`. Adding a variant requires updating both sides and the wire-name test.
- Tool schemas remain JSON because they are sent over the OpenAI-compatible protocol, but model
  arguments must deserialize into native types before use.

## Safe change sequence

1. Identify the owning row in the module table.
2. Add or change the native type before changing display copy or JSON construction.
3. Add a pure unit test for parsing, transition, lint, or provenance behavior.
4. Add integration coverage only where filesystem/HTTP behavior is the subject under test.
5. Preserve existing project schema defaults and recovery behavior.
6. Run every command required by the repository `AGENTS.md`.
7. For runtime/harness changes, run the applicable ignored live test with local services available.

Fast tests are the first defense: lifecycle rules do not require a live model, stream framing does not
require HTTP, and workspace semantics do not require ComfyUI. The ignored live acceptance tests prove
runtime compatibility and realistic production quality; they are not a substitute for deterministic
coverage.
