# Studio maintainer guide

This directory contains every model-assisted part of Kestrel Studio. Start here before changing
Director/Reviewer planning, prompt generation, Producer Copilot, H3 image assets, live previews, or Music. The root
`studio.rs` file remains the domain and persistence facade; child modules own one bounded concern
and must not acquire authority implicitly.

## Non-negotiable boundaries

- Studio works without the public network. Model HTTP is authenticated loopback traffic and ComfyUI
  is fixed to loopback.
- `RuntimeManager` owns the only local language-model process and inference semaphore. A Studio
  feature never starts a second model server or bypasses its lease.
- Model text is data. Only `movie_agent.rs` may mutate the planning workspace, and only through its
  deserialized `WorkspaceAction` contract. Producer Copilot proposes edits but cannot apply them.
- `project.json`, immutable reference objects, raw masters, edit decisions, receipts, transcripts,
  and planning controls are durable user data. Interrupted work is surfaced, never silently resumed.
- The model receives the original producer request and a fresh authoritative workspace snapshot on
  every Director planning turn. UI/log redaction must never mutate model input or durable history.
- H3 and Music 3 rendering begin only after every language-model inference lease is released and the
  runtime is unloaded from the GPU.
- Director and Reviewer bindings are durable project data. Missing pinned models fail visibly; an
  explicit checkpointed role change records provenance and forces producer review.
- Non-Bonsai models must pass the current local Studio protocol check before standard-mode unattended
  planning. Advanced mode may run an unverified compatible model only with forced producer review.

## Module ownership

| Module | Owns | Must not own |
| --- | --- | --- |
| `studio.rs` | Public Tauri-facing domain types, project persistence, job coordination, rendering and edit facade | Model stream framing or workspace action semantics |
| `agent_flow.rs` | Director planning orchestration, producer-control boundaries, per-turn model leases, tool dispatch, independent-review coordination | Wire parsing or file mutation rules |
| `agent_lifecycle.rs` | Pure session, tool-use, and reviewer-budget transitions | HTTP, filesystem, UI events, or project state |
| `agent_protocol.rs` | Exact planning requests, lossless transcript history, assistant/tool-call assembly | Workspace mutation or producer copy |
| `model_stream.rs` | Shared OpenAI-compatible SSE framing, UTF-8 fragmentation, JSON validation, completion markers | Feature-specific tokens, tool schemas, or UI events |
| `movie_agent.rs` | Sandboxed movie workspace, typed actions/outcomes, native plan compilation and lint | OS commands, arbitrary paths, renderer execution, or network |
| `planning.rs` | Durable redirection/checkpoint controls and typed planning UI event contract | Model inference |
| `prompts.rs` | Planning, lint, resume, and repair prompt text exposed to advanced producers | Hidden control behavior |
| `prompt_collaboration.rs` | Story/image/reference/music-description/lyrics drafting from producer context | Applying proposals, movie-plan mutation, or rendering |
| `copilot.rs` | Timeline advice and validated, unapplied edit proposals | Applying edits or rendering |
| `image_assets.rs` | Durable H3 pseudo-image generations, graph/receipt provenance, imported candidates | Planning authority |
| `live_preview.rs` | TAE preview graph nodes and producer-visible preview events | Final-render truth |
| `music.rs` | Recoverable song projects, producer arrangement, native Music 3 graphs, immutable takes, progress, and optional MuScriptor adapter | LLM process ownership, fake stem separation, bundled gated weights, or public-network fallback |

If a change appears to belong to two rows, introduce a typed boundary instead of importing private
implementation details across both modules.

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

## Model transport

All Studio language-model streams pass response bytes through `OpenAiSseDecoder`. Features retain
their own request bodies and map decoded JSON into their own events, but must not implement another
`data:`/`[DONE]` parser. The decoder intentionally rejects malformed JSON, invalid UTF-8, duplicate
completion markers, and events after completion so token loss cannot look like success.

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
| `../../model-qualifications.json` | Recoverable protocol receipts bound to model, engine, runtime profile, and protocol revision |

Atomic replacement and recovery copies are deliberate. Do not trade them for in-memory convenience.
The advanced UI reads bounded redacted views; the unmodified files remain available as durable truth.

## Music production lifecycle

Music uses the same resource boundary without inheriting movie-agent authority:

```text
open recoverable song project
  -> producer edits description, tagged sections, lyrics, and settings
  -> optional selected local GGUF streams an unapplied description or lyrics proposal
  -> producer applies, discards, redirects, or keeps the partial checkpoint
  -> persist project and unload the language-model runtime
  -> submit the native MiniMax Music 3 graph to loopback ComfyUI
  -> stream node phase, sample step, percentage, and ETA
  -> copy and hash the completed stereo WAV inside the project
  -> append an immutable take and exact generation receipt
```

`music/<uuid>/project.json` is recoverable truth. `takes/<uuid>.wav` and its graph receipt are immutable;
editing the arrangement never rewrites an older take. Startup changes active generations to
`interrupted` and never submits them again. MiniMax Music 3 produces a stereo master, so the arranger
may show semantic lanes for structure and lyrics but must not label generated audio as separate stems.
MuScriptor remains an explicit advanced adapter: the producer supplies both its executable and gated
checkpoint, Kestrel invokes a fixed argument array, and the receipt preserves its non-commercial
license notice and output hash.

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
