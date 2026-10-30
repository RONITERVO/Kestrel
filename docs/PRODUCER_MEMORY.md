# Durable local producer conversation

## Release objective

Give a nontechnical producer one local conversation for describing work, answering questions,
changing direction and returning later. The local model (primarily Strata) must recover work from
native records after repeated context compactions, cancellation, process restart or a fresh model
conversation. This applies to every existing capability and future artifact types, not a fixed
music/video workflow. Phone/mesh networking is a later transport over the same native contracts.

Ordinary chat remains tool-free. Producer work uses Computer Tasks' existing workspace boundary,
explicit Full Access opt-ins, RuntimeManager inference lease and research lock. Restart never
authorizes automatic execution. Model-written memory cannot grant permissions or rewrite user
messages. No cloud dependency, background networking or new model provider is introduced.

## Design

- A stable task ID identifies the conversation across model turns and explicit continuations.
- Immutable, sequential native journal records preserve original user direction and action
  outcomes. Existing task JSON is the migration base; new records do not rewrite old history.
  Summary indexes are rebuildable, and an interior sequence gap or corrupt journal record is an
  error, not deletion. External deletion of the tail cannot be detected from that journal alone;
  the journal is durable application storage, not a substitute for backups.
- A bounded, revisioned working checkpoint contains the agent's plan, decisions, references and
  next steps. It is a claim by the agent, separate from native action receipts and user direction.
  Every checkpoint revision remains in the journal. Evidence references identify actual events.
- Bounded search and paged reads retrieve older direction and full results on demand. Large
  outputs stay durable without occupying the entire active prompt. Recovery context is rebuilt
  from current records instead of repeatedly appending summaries to the system message.
- A human message sent during work is persisted immediately and consumed before further actions.
  Messages arriving during inference invalidate the proposed batch before it can mutate anything.
  Work already submitted is observed/reconciled; new direction does not erase its receipt.
- A completed production remains a conversation that can accept follow-up work. Long work has
  explicit native budgets, useful progress and questions; hitting a budget is never completion.
- File/project ownership and uncertain operations must be visible in recovery. Existing files
  require fresh inspection before overwrite; an interrupted action is never blindly replayed.
- An OS-held workspace ownership lock prevents another app instance from recovering a running
  task or racing immutable journal writes. Closing/crashing the owning process releases it.
- The desktop presents ordinary messages, progress, questions and results, with technical history
  available separately. Future clients use the same Rust-owned conversation contracts.

## Acceptance criteria

- Migrate old task records without losing events, attachments, artifacts or status.
- Recover journal writes despite stale indexes; reject corruption/gaps; stop before mutation if
  required persistence fails. No auto-resume on restart.
- Preserve early and amended human directions through repeated compactions and fresh starts.
- Bound context, tool previews, memory and retrieval while retaining full original records.
- Demonstrate checkpoint revision conflicts and evidence validation; memory never changes access.
- Demonstrate mid-work steering, follow-up after completion, questions and explicit interruption
  recovery in both native API and desktop presentation.
- Verify ownership/current-file changes and interrupted action reconciliation.
- Run real local Strata acceptance at high thinking using vague nontechnical requests, multiple
  different tool combinations, a cold conversation restart and repeated compaction. Use Luna
  only as a testing customer when needed; never for implementation.
- Run the full AGENTS.md verification suite, applicable ignored live tests and UI inspection.
- Review the final diff and evidence, then prepare the authorized PR. Do not claim readiness from
  the earlier producer foundation's tests; this release needs its own evidence.

## Implementation record

2026-10-30: started on `codex/durable-producer-memory` from merged PR #36. The pre-existing
`src-tauri/Cargo.toml` working-tree change is outside this work. This document records the full
objective and acceptance criteria; unchecked requirements above remain open until verified.

2026-10-30: inspected the user's four PlanetStrata sessions in installed DeepSeek Harness
0.2.0-rc.2 and its compaction/basic/tool-result-pruner packages (read-only). One fresh session's
actual input usage including cache grew from 7,831 to 51,934 tokens in seven steps, predominantly
while reading existing source files. Final requests repeatedly reached 65,528 total tokens.
The longer session recorded both input/output admission errors (32,097 prompt + 37,262 output;
42,471 prompt + 30,212 output, each against 65,536 context) and compaction summaries truncated at
their output cap. Its saved event sizes include stream metadata and are NOT prompt token counts.

Useful design lessons: immutable event sequence references; balanced tool-call/result boundaries;
deterministic result pruning; validating that replacement memory is actually smaller; rejecting
stale checkpoints. Kestrel's recovery must not depend on a successful summarization inference.
Keep original evidence and bounded projections separate, omit historical reasoning from the next
planning prompt, and measure the installed Strata template before generation. The local Strata
`/v1/messages/count_tokens` endpoint does template tokenization without model inference. No cloud
provider is involved in that endpoint's compatibility format.

2026-10-30 first acceptance checkpoint (historical; not ready at this stage):

- High-thinking Strata authored an owl story in the isolated acceptance library, saved native
  working memory and asked for style. A cold app restart preserved the question without
  automatic execution. A normal follow-up renamed Pip to Milo, retained Dot, reduced the story
  to four pages and generated no images.
- Follow-up after completion was accepted. A second message during inference superseded a
  request to generate images with an offline HTML booklet. The resulting 9,072-byte booklet
  and revised text are in `.artifacts/producer-memory-acceptance/outputs`; the 117-event journal
  remains in the isolated library. No images were generated in this scenario.
- Four real high-thinking Strata requests passed forced compaction of synthetic historical tool
  results: input 34,319 / 37,168 / 37,168 / 37,168 tokens fell to 3,178 each time. Native preflight
  and actual generation counts both equalled 3,178, output cap 2,048. Recovery marker and phase
  survived each turn. This is a tokenizer/compaction integration test, not four completed
  production projects. Ordinary cold recovery was checked in the real story conversation above.
- Native reconciliation now gates mutations behind unfinished action records. File hashes and
  existing app receipts can establish observed state without replay; otherwise a separate
  producer review is required. Task/workflow tools cannot grant their own review. Review does
  not mean success, change access or start work. Original arguments and every review persist.
- The serialized recovery packet is bounded below 24 KB even with large Unicode directions
  and a maximal checkpoint. Truncated originals remain paged at stable record sequences.
- The desktop conversation was visually checked. Duplicate headings and expanding auxiliary
  sections crowded replies into eleven tiny pages; separate conversation/memory/activity views
  reduced the same reply to three pages while keeping the composer and artifacts visible.
  Native memory is paginated. Sending a task message can load its selected model itself.
- Most recent full suites before the final progress/UX refinements: Rust 300 passed/29 ignored;
  UI 235 passed/36 files; Clippy and build passed. Focused follow-up tests passed after refinements.
  All required checks must run again on the final changes.
- Luna is currently a nontechnical customer only, using the real high-thinking local Strata
  producer for a Moon guide with one picture and narration. It reported opaque progress and
  excessive preparatory latency before the first artifact. Native progress projection and more
  concise producer guidance are being improved from that feedback; the active trial remains
  on its original build until it finishes. No implementation work is delegated to Luna.

2026-10-30 second acceptance checkpoint (historical; fresh trial still required):

- The Moon-guide trial generated its text and illustration, but a 300-second silent Strata engine
  request returned 503 after speech. Natural follow-ups recovered the task and produced a local
  five-source research report. Delivery remained incomplete, and the agent drifted into an extra
  image draft after the fact-check request. After the customer's stop-and-summarize direction it
  spent several more minutes inspecting state; native Stop ended the trial at 292 events. This is
  a failed customer acceptance, not a successful finished production.
- User screenshots showed near-full RAM and a mostly idle GPU during the stall. A later process
  snapshot found Strata at about 59 GiB private committed memory, ComfyUI at about 13 GiB, and
  only about 1.6 GiB physical RAM free. This correlates with memory pressure, not proof of the
  exact engine failure. The agent now releases media/Whisper caches before reloading a stopped
  language runtime. Generic ComfyUI /free did not release the separately cached Whisper model.
- The agent submitted a whole long guide as one voice passage; the desktop ordinarily splits
  at about 320 characters. Native validation now rejects takes exceeding the voice token budget
  or 640 spoken characters, with an actionable split/join instruction. SpeechClip exposes the
  saved narrationCheck, including failed/unfinished checks; success cannot be inferred from the
  synthesis receipt alone. The old trial's check matched only about 5.5% after two attempts.
- A 503 during decision generation receives one visible inference-only retry per continuation,
  still bounded by the task step budget and direction/cancellation checks. Tools from failed
  inference are never executed or replayed. Repeated failure remains an explicit failure.
- Message acknowledgements now return compact ProducerTaskStatus. Full transcript retrieval
  remains explicit. Model task_read retrieves payloads without the duplicate event wrapper and
  can select a JSON pointer; tool previews prioritize results over echoed long request arguments.
  Default history search omits reasoning, which remains retrievable by explicit kind.
- Recovery no longer changes the first user-message prefix on every planning turn. Ordinary
  steps retain the engine's reusable prompt prefix; entry, new direction, byte compaction and
  token compaction rebuild fresh native recovery, which is measured again before generation.
- Research report HTML and exported narration now surface as native task artifacts. Guidance
  asks for the smallest current deliverable and treats status questions as part of ongoing work.
- Current verification after these changes: Rust 307 passed/29 ignored, UI 235 passed/36 files,
  Clippy, architecture/bindings/TypeScript checks and frontend build passed. The fresh executable
  and customer trial remain pending. No PR has been opened and merge readiness is not claimed.

2026-10-30 fresh customer acceptance and final verification:

- A fresh Luna low customer sent only: "Make me a little goodnight card with a fox picture and
  someone saying goodnight." The real local high-thinking Strata producer completed the task in
  about 11 minutes 23 seconds, across 26 planning steps and 153 native events. Luna inspected
  the generated picture and reported that the request was satisfied. Luna wrote no application
  code or production material and did not invoke individual media actions for the local agent.
- The delivery has a readable fox picture, narration, words, and a composed video in a native
  bundle. FFprobe independently confirmed a 12.5-second 1920×1080 H.264/AAC MP4; the source
  PNG is 1536×1024. The saved narration check passed with a matched fraction of 1.0. This is an
  automated transcript check, not a guarantee of every aspect of audio quality.
- The successful trial still exposed friction: initial/repeated Strata loads of roughly 40–70
  seconds and three corrected input-format calls. Action discovery now lists names/access
  concisely; action descriptions carry detailed guidance. The image composition description
  includes the exact existing native draft schema, and speech guidance distinguishes voice-pack
  IDs from voice-profile IDs. No specialized card or music-video workflow was introduced.
- The test machine remained close to its memory limit: about 1.1 GiB physical RAM and 1.4 GiB
  commit capacity free after completion, with about 59 GiB private memory in Strata and 7 GiB
  in the media process. Unloading the idle language runtime released its allocation. Media
  cache release helps, but this release does not promise a small RAM footprint for that model.
- Accepted follow-ups now persist a queued event before the response, so a crash before the
  runner starts recovers as interrupted rather than appearing to be completed old work. A
  regression test preserves the previous answer and new user direction through that restart.
- Running the two live Strata tests consecutively exposed an unreliable process-count assertion:
  the count was one both before and after a new load as the previous engine disappeared. The
  test now compares exact new PIDs and waits for those PIDs to disappear on stop. Unlike counts,
  this also prevents an unrelated process exit from masking a leaked test engine. Separately,
  explicit stop now terminates the owned Windows job and waits, with a 30-second bound, for zero
  active members before clearing runtime ownership. A process-tree regression checks that its
  grandchild is already gone when Stop returns; crash/drop cleanup remains.
- File writes use exclusive unique staging files and unique recovery-copy names, so repeated
  writes cannot replace another writer's temporary file or a same-second recovery copy. Tests
  preserve both earlier file versions and an unrelated staging file across consecutive writes.
- Final required checks: Rust 309 passed/29 ignored; frontend 239 passed/36 files;
  strict Clippy, regenerated bindings, architecture/TypeScript checks, frontend production build
  and diff whitespace checks passed. Both ignored live Strata tests passed consecutively in
  71.48 seconds: four exact preflight/generation token-count matches after compaction, plain and
  schema replies, reuse of the installed fixed context, and cleanup of the exact new engine PID.
- A cold restart retained the completed fox conversation at 153 events without resuming it. The
  returning customer asked: "Could you make the goodnight words shorter but keep the fox? Please
  show me the new wording before changing anything." Strata loaded itself, inspected the saved
  narration and checkpoint, then offered three complete shorter wordings and waited for a choice
  at event 170, about 80 seconds later. No files or media were changed. Native validation rejected
  one checkpoint proposal with invalid evidence references; the previous checkpoint and original
  new direction remained intact. The customer stopped at the approval question as requested.
- Standalone desktop inspection found stale history status when another native client continued
  an unselected task, plus horizontal overflow from long suggested answers. History now refreshes
  on native start/terminal events without accumulating unselected transcripts. Selected tasks
  follow external continuation, and transcript opening merges concurrent events without duplicate
  messages. Full answer choices appear in the paginated conversation; the choice control stays
  within the page. Three regressions cover external continuation, unselected history and opening
  during an incoming question. Model loading and all production still belong to the native runner.
- The rebuilt embedded desktop displayed the complete wording choices and composer within the
  page. Another restart preserved the waiting question and all 170 events with the runtime
  stopped. After unloading the idle model, free physical memory returned to about 50 GiB.

These checks support merging this as the durable producer-conversation foundation. They do not
establish a 99% completion rate, instantaneous replies, or acceptance of every possible media
combination. The failed Moon trial remains part of the evidence; the later successful production
and cold-restart follow-up demonstrate the repaired paths without hiding the observed limits.

The immutable journal currently replays a task's history for native reads. Model-facing previews,
recovery packets and paged responses are bounded, but full desktop transcript reads and journal
replay are not constant-cost as a conversation grows. A future rebuildable index/cache can improve
that without replacing the authoritative records or changing the agent-facing memory contract.
