# Studio maintainer guide

Kestrel Studio is producer-owned. A producer can start with a text-message-length idea: the local
model expands the creative writing, while the producer decides where references belong. Native
code handles long queues because small local models should not be trusted to schedule a batch.
Every clip in a queue receives a distinct written scene, rather than a repeated prompt variation.
Rust stores the creative record and performs every durable or
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
  model context. Starting a distinct-scene queue also authorizes the four latest scenes written by
  that queue as continuity context, as explained beside the queue controls.
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
| `producer/batch.rs` | Native queue validation, durable checkpoints and append receipts | Inference or automatic resume |
| `producer_chat/batch.rs` | Fixed sequential one-scene requests using the shared inference gate | Model-selected scheduling, media selection or rendering |
| `export.rs` | Bounded FFmpeg groups and manifest assembly | Creative decisions or source mutation |
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

New movies default to 768 × 448. The producer can explicitly choose 1344 × 768 for more detail;
runtime is hardware-dependent, so the UI does not promise a fixed number of minutes.

Producer scene cards are the source of renderer direction. Native code appends exact audio and
reference requirements immediately before graph construction. First/last frame conditioning and
native reference conditioning use their distinct H3 graph paths and cannot be combined when H3 does
not support the combination.

If any renderer-affecting scene field changes, the active master moves into immutable `versions`, the
scene returns to `queued`, and its current path is cleared. Unchanged scene cards retain their master.
Never let UI state decide whether an old render still matches a scene.

Preview frames are approximate process-local state. Starting a render clears the previous estimate;
the preserved full-VAE master and receipts remain durable truth.

## Long productions

The producer can request 1–4096 distinct scenes in a native queue. Each inference requests exactly
one new scene with a fixed duration and no media authority. A malformed or repeated prompt stops
at that checkpoint without skipping a scene. Each completed scene and its queue progress are saved
together in the recoverable workspace. Reopening never resumes inference. Resume preserves the
original direction and accepted story and rejects changes made after the checkpoint.
Reference-only changes during a pause retain the checkpoint because media never enters model
context. Changing scene text, order, or the accepted story requires a new queue.

Each append has an immutable receipt in `producer/scene-history/`; batch drafting does not save
thousands of full historical copies of a long film. Starting another batch archives the previous
queue record in `producer/draft-batches/`. Transcripts rotate every 16 scenes and remain on disk.
The UI paginates scene cards, masters, and the timeline index. Timeline tracks and ruler ticks render
only around the visible scroll window. Scene saves preserve repeated items and custom export titles.
Ordinary scene-history snapshots also preserve the prior edit so scene removal remains recoverable.

Exports support up to 4096 timeline items. Native code encodes groups of at most 16 inputs with
file-based filter graphs, then joins them with a local concat manifest. Video is encoded once;
FLAC intermediate audio avoids per-group AAC delay. Final AAC and optional loudness processing run
over the whole cut. Only owned scratch files are removed; source masters and exports are immutable.

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
