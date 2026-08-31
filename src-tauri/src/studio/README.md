# Studio maintainer guide

Kestrel Studio is producer-owned. Rust stores the creative record and performs every durable or
media-affecting operation; React renders an editable view of that record. Local-model prose is an
unapplied suggestion until a typed native boundary saves it.

## Non-negotiable boundaries

- Studio remains useful without the public network. Model HTTP and ComfyUI traffic are authenticated
  loopback traffic only; there is no remote fallback.
- `RuntimeManager` owns the only language-model process and its semaphore owns the only inference
  slot. Story chat, scene chat, prompt drafting, research, ordinary chat, and Computer Tasks all use
  that gate.
- Movie collaboration is tool-free. The story collaborator returns one complete Markdown document.
  The scene collaborator returns bounded JSON operations that native code parses and applies to
  producer-owned scene IDs. It has no filesystem, renderer, reference, frame, or audio authority.
- The producer chooses scene context explicitly. The accepted story is always included; only checked
  scene cards are sent in full. Reference assets, frame choices, and native H3 tags never enter scene
  model context.
- `project.json`, producer workspaces, every story revision, scene-history snapshot, conversation,
  source object, master, edit decision, and export receipt are durable user data. Interrupted work is
  surfaced and partial collaborator text is preserved. It is never silently resumed or discarded.
- H3, Ideogram 4, and Music 3 rendering starts only after language-model inference releases its lease
  and Kestrel unloads the model runtime from the GPU.
- Model output is never executed. Parse bounded typed data, validate IDs and sizes, use fixed argument
  arrays, and persist the native result.

## Module ownership

| Module | Owns | Must not own |
| --- | --- | --- |
| `studio.rs` | Movie project compatibility, reference store, H3 graphs, masters, edit/export facade | Conversation policy or React view state |
| `producer.rs` | Recoverable producer workspace, immutable story revisions, scene cards/history, conversations, project-plan projection | Model HTTP or renderer execution |
| `producer_chat.rs` | One-shot story/scene/summarization requests, strict scene-operation parsing, streaming events | Reference selection, frame selection, rendering, or arbitrary tools |
| `prompt_draft.rs` | Tool-free image/reference/music drafting from producer context | Applying proposals, movie-scene mutation, or rendering |
| `model_stream.rs` | OpenAI-compatible SSE framing and explicit reasoning-channel extraction | Feature prompts, schemas, or persistence |
| `image_assets.rs` | Durable H3 pseudo-image generations and exact graph/receipt provenance | Story or scene authority |
| `image_studio.rs` | Recoverable image projects, structured composition, native Ideogram graphs, immutable PNG takes | LLM process ownership or public-network fallback |
| `live_preview.rs` | Bounded process-local preview state and visible preview events | Durable final-render truth |
| `music.rs` | Recoverable song projects, producer arrangements, native Music 3 graphs, immutable takes and lyric revisions | LLM process ownership or fake stem claims |
| `music_lyrics_model.rs` | Bounded audio-listening and translation suggestions through a caller-owned lease | Durable lyric mutation or runtime ownership |
| `music_midi.rs` | Bounded MIDI parsing/writing and recoverable piano-roll revisions | Source mutation or arbitrary path selection |

If a change crosses rows, add a typed boundary rather than importing private state.

## Movie producer lifecycle

```text
starting material
  -> create durable project and producer workspace
  -> one tool-free local-model response becomes an immutable Markdown story revision
  -> producer edits/saves any number of revisions
  -> producer accepts one revision and chooses continued or fresh scene conversation
  -> accepted story + explicitly selected scene cards enter scene chat
  -> native code validates add/update/remove/split operations around stable scene IDs
  -> every scene save creates a scene-history snapshot and projects cards into the renderer plan
  -> producer binds first/last frames, visual references, exact audio, and guidance
  -> explicit render queues changed scene cards; older masters remain immutable versions
  -> producer arranges the timeline and explicitly exports an immutable cut
```

There is no Director, Reviewer, autonomous workspace, hidden plan exchange, or model-selected media.
Legacy project fields and old folders may remain on disk for non-destructive compatibility, but no
current command reads them as authority.

## Durable movie artifacts

| Artifact | Meaning |
| --- | --- |
| `request.json` | Exact producer starting material, settings, and attached references |
| `project.json` | Recoverable renderer/edit compatibility projection |
| `producer/workspace.json` | Current story pointers, conversation summaries, and scene cards |
| `producer/story-revisions/*.json` | Immutable complete Markdown revisions |
| `producer/scene-history/*.json` | Immutable scene-card snapshots |
| `producer/conversations/*.json` | Recoverable full story or scene chat transcripts and summaries |
| `plan.json` | Native projection of current scene cards for rendering |
| `raw/*.mp4` and clip versions | Immutable H3 masters and preserved earlier masters |
| `exports/*` and receipts | Immutable explicit deliverables and hashes |

All replaceable JSON uses recovery copies. Never delete an old `agent-workspace` or other unknown
legacy artifact while opening or saving a project.

## Story and scene context rules

Story requests contain the starting material, the chosen current revision when present, the saved
conversation summary, and bounded message history. Each successful response is a complete Markdown
replacement and therefore a new immutable revision.

Scene requests contain the accepted story in full, the conversation summary/history, a compact list
of all scene IDs/titles, and the full text of only producer-selected scene cards. The model may add,
update, remove, or split around IDs through the response schema. Native code rejects stale revisions,
unknown IDs, duplicates, invalid duration, oversized text, and scene-count overflow. It preserves
reference and frame selections when updating model-owned text fields.

Conversation reset archives the prior transcript and creates a new one. Carrying a saved summary is
explicit. Summarization is a separate one-shot local inference and saves the result before returning.

## H3 rendering

Producer scene cards are the source of renderer direction. Native code appends exact audio and
reference requirements immediately before graph construction. First/last frame conditioning and
native reference conditioning use their distinct H3 graph paths and cannot be combined when H3 does
not support the combination.

If any renderer-affecting scene field changes, the active master moves into immutable `versions`, the
scene returns to `queued`, and its current path is cleared. Unchanged scene cards retain their master.
Never let UI state decide whether an old render still matches a scene.

Preview frames are approximate process-local state. Starting a render clears the previous estimate;
the preserved full-VAE master and receipts remain durable truth.

## Model transport

All Studio streams pass bytes through `OpenAiSseDecoder`. It rejects malformed JSON, invalid UTF-8,
duplicate completion markers, and events after completion. Explicit `reasoning_content`/`reasoning`
deltas may appear in a bounded provisional UI pane; never infer reasoning from ordinary text or copy
it into an executable request.

When adding a runtime variation:

1. Add fragmented-wire tests in `model_stream.rs`.
2. Keep feature-specific interpretation in the owning module.
3. Preserve partial producer-visible output and durable receipts on interruption.
4. Never add public-network fallback.

Music lyric alignment and visual-renderer ownership remain documented in
[`MUSIC_LYRIC_ALIGNMENT_CONTINGENCY.md`](MUSIC_LYRIC_ALIGNMENT_CONTINGENCY.md) and
[`MUSIC_LYRIC_VISUALS.md`](MUSIC_LYRIC_VISUALS.md).
