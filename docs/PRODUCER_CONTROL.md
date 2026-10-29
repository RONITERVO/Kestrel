# Producer control

Kestrel's desktop commands are also its agent-neutral producer interface. The native build reads
the actual Tauri signatures and generates the dispatcher and argument descriptions. It does not
keep a second implementation of music, images, research, movies, speech, or exports.

This follows the useful parts of StateBeats' `packages/sdk/src/service.ts` and `docs/TOOLS.md`:
semantic actions, a shared service, observations that do not advance work, and explicit retry IDs.
Kestrel adds recoverable on-disk receipts and sequential production plans. There is no MCP server,
remote model provider, new Codex integration, background download, or automatic workflow resume.

## Human and agent entry points

- Control's **Producer** view opens plain-language Computer Tasks and reviews saved production
  plans. Its Advanced sections discover native actions and contracts, show results, and author
  explicit plans. The normal Music, Image and Studio editors remain direct authoring controls.
- Computer Tasks exposes `app_actions`, `app_describe`, and `app_call` to the selected local model.
  Ordinary chat remains tool-free. `start_producer_task` takes only an `objective` and uses the
  selected local model, configured step budget, and workspace access.
- External agents can use an explicitly enabled local Windows named pipe. Start Kestrel with
  `KESTREL_PRODUCER_SESSION` set to a new absolute JSON filename. The app creates that session file
  with a random pipe name and capability token. A prior file is never overwritten. No TCP or HTTP
  port is opened; remote named-pipe clients are rejected. Closing Kestrel ends the capability.
  Protect the file like a local application-control credential; do not commit it.

For a repository build, a PowerShell launch looks like:

```powershell
$env:KESTREL_PRODUCER_SESSION = 'D:\MyWork\kestrel-session.json'
Start-Process -FilePath '.\src-tauri\target\debug\kestrel-local.exe' -WindowStyle Hidden
node scripts/producer-control.mjs D:\MyWork\kestrel-session.json request.json
```

`request.json` contains one object, for example:

```json
{"requestId":"my-production-1","command":"start_producer_task","args":{"objective":"Make me a music video about the northern lights."}}
```

The thin CLI also accepts `-` to read a single request from stdin. It has no production policy,
scheduling logic, or model client. Its response is `{ok,result}` or `{ok:false,error}`. An action
result is a receipt with `status`, `result`, and `error`; a completed receipt means the command
returned successfully, not that an asynchronous render finished.

`list_app_actions` accepts an optional name `filter`. `describe_app_action` takes `command` and
returns exact argument names, Rust-generated type declarations, access, and guidance. Requests use
the same camelCase arguments as the desktop. Unknown top-level arguments fail rather than vanish.
`get_app_action_receipt` retrieves a request by `requestId`; `get_app_job` retrieves a streaming
draft by `id`. Image/music/movie project reads remain the authority for completed media.

Poll `get_producer_task_status` for a compact current phase, question, and artifact paths instead
of repeatedly loading `get_computer_task`'s full durable transcript. Research and media child jobs
report their current phase through this view and the human Computer Tasks view. A renderer can
spend minutes on a phase; an unchanged phase alone does not mean failure.
Video progress identifies the scene before its sampling count. Music composition reports token
usage against a limit without inventing a remaining-work percentage or ETA; diffusion sampling
has a known step count and is reported separately.

`get_producer_environment` returns the selected local model, approved workspace folders, actual
new-model download destination, and configured FFmpeg, FFprobe, Strata, ComfyUI, MuScriptor and
offline-library paths with filesystem presence. Use Setup's snapshot for service readiness.
It also supplies `generationDefaults` from the same Rust settings used by the human studios.
The faster H3 defaults are 768×448 and 20 steps. A 1080p delivery size does not require 1080p
generation. Preserve the native defaults unless the producer requests a different tradeoff.

## Producer authorship and local collaboration

A producer can write a music caption/lyrics or image composition directly, or explicitly call
`start_studio_prompt_draft` to ask the local model. Read `get_app_job` with that draft's `requestId`:
its durable `text`, status, and provenance survive disconnects and restart. Only `complete` is a
finished proposal; `limited`, `error`, `cancelled`, and `interrupted` require review.

`apply_music_text` accepts project `id`, `field` (`caption` or `lyrics`), and `text`.
Tagged lyrics preserve section IDs and match repeated tags by occurrence; unrecognized tags fail.
`apply_image_composition` accepts project `id` and the composition JSON `text`, checks style, boxes,
and palettes natively, then uses the existing image save. Full typed project saves remain available.
These adapters accept either producer-authored material or an inspected local draft. A drafting
model cannot choose to save/render its own proposal. The human or agent producer makes that choice.

Computer Tasks returns the inference lease after each decision. Its separate ownership gate keeps
other work from racing its app actions; each app action uses the usual work lock and the one runtime
lease. Rendering unloads the language model before using the GPU. The task waits for its child job
to release that work slot before planning again. Cancellation propagates to active child jobs.

## Recovery and overnight plans

Every intentional action needs a unique request ID. An identical retry returns the same durable
receipt. Reusing an ID for another command or different arguments fails with a retry conflict.
The receipt is saved **before** execution. A crash before the outcome is saved becomes `interrupted`;
Kestrel cannot safely assume whether a side effect happened and will never replay it automatically.
Inspect the project, then deliberately issue a new request if appropriate.

`create_producer_workflow` takes a `plan` with a title and 1–256 sequential steps. Saving does not
start it. `start_producer_workflow` explicitly starts/resumes it; `get_producer_workflow` and
`list_producer_workflows` read progress. `stop_producer_workflow` prevents subsequent steps; an
already-submitted render has its own native cancel action.

Each step contains `id`, `command`, `args`, and optional `wait`. An exact object
`{"$from":"earlier-step","pointer":"/id"}` substitutes a prior native result without string
evaluation or a shell. Forward/missing references fail. A wait contains a read-only `command`,
`args`, JSON `pointer`, expected `equals` value, and a 1–43,200-second `timeoutSeconds`. On success,
the completed observation becomes the step's result. Use `/status` = `complete` for a draft, or a
specific take's status for generation; an older successful take does not prove a new render worked.
Waits stop on native failure and preserve the partial plan. A stopped app never resumes a plan by
itself. An uncertain last mutation requires reconciliation, not a blind resume.

Computer Tasks can save and explicitly start these same plans. It waits for its plan before
making another decision. Plans started by a local task retain approved workspace folders, and
native code checks paths again after resolving prior-result references. Even a one-off Full Access
task saves a workspace-scoped plan; an overnight resume never inherits that temporary opt-in.
Plans cannot nest or start another task, chat, or runtime. Stopping the owning Computer Task also
stops its plan and signals its active child jobs.

A model response with `finish_reason: length` never completes the task or executes a partial tool
batch. The runner retries at most twice within the existing step budget, asking for a smaller
concrete action, then stops with an actionable error. Strata receives its supported
`reasoning_budget_tokens` field rather than the llama.cpp/Bonsai field, so low thinking leaves
space for the tool call instead of consuming the entire response budget.

Requests, results, partial drafts and workflow records live under the library's `producer/`
directory. All records are bounded and use recoverable replacement. Native media/project records
remain the underlying truth; producer receipts do not fabricate completion or change provenance.

## Files, exports, and personal consent

`import_context_paths` and `import_movie_reference_paths` are the path equivalents of file pickers,
using the same bounded importers. Audio and MIDI export commands accept `destinationPath`; agent
calls must provide it and cannot overwrite an existing file. Computer Tasks resolves supplied
paths through its selected access policy, including output folders. App-owned project IDs are
capabilities to managed artifacts, not arbitrary file paths.

The action catalog explicitly identifies controls that still require personal consent in their
desktop UI: access/runtime policy changes, developer tools, downloads/installations, destructive
data actions, voice rights, and GPU termination. The producer cannot grant itself Full Access or
bypass two-step consent. These boundaries are discoverable and produce actionable errors.
Use `get_producer_environment` for path/model discovery, `get_setup_snapshot` for readiness and
`list_model_downloads` for transfer history. Discovering a download path is separate from authorizing
a transfer.

`export_production_bundle` delivers any selected `ProductionArtifact` handles with producer-chosen
relative filenames and descriptive roles, plus exact authored text/code files. No modality,
platform or output category is required. The same operation can deliver multiple object views,
music, generated images, narrated media, research evidence, or an application's imported/generated
assets and source files. Original copies retain their source extension and bytes; optional JPEG
conversion has explicit dimensions and contain/cover fit. Thumbnail choices are ordinary image
items, with no dedicated music-video command. Published research HTML, report JSON, sources and
provenance are selectable immutable artifacts. Imported attachments provide a format-independent
path for additional local files, subject to the existing import and workspace boundaries.

Every bundle has a request, source/output hashes and manifest, and is delivered through a new
child folder. Relative names reject path escapes, Windows devices, aliases, collisions and reserved
manifest names. Text/code is saved without execution; a produced app is not thereby installed or
running inside Kestrel. No upload occurs. `cancel_production_job` stops a bundle or composition using
its caller-chosen `jobId`. Partial native work is preserved without automatic resume.
`list_production_bundles` and `get_production_bundle` expose saved manifests and incomplete requests
after restart. Native completion and successful copying to a delivery folder are distinct: use
the action receipt to confirm delivery.

The build-time policy coverage test requires a reviewed disposition for every desktop command.
Adding a command without a policy fails verification. Any future MCP adapter should call this
same boundary instead of reimplementing application behavior.

## Growing the production system

Music video is one acceptance scenario, not the production architecture. The shared unit is a
native capability with typed inputs, a durable receipt, cancellation and observable completion.
Discovery includes the native successful `resultType` and transitive result declarations, not
only argument types. Workflow references preserve JSON value types (including arrays, numbers
and objects). Future classification, regression, forecasting, geometry or control capabilities
can therefore return their own validated structured contracts, with provider-specific probability,
confidence and provenance where appropriate. There is no requirement to generate natural-language
text or a media file, and no universal confidence scale is assumed. Such providers and their
resource/authority requirements remain future work, independently of this foundation.
Studios produce artifacts; workflows arrange actions; delivery combines chosen artifacts. Neither
a local model nor an external agent owns a second implementation of these operations. A producer
can author material directly or request a local draft at any step. Research is optional, and its
published evidence/citation rules continue to apply when it supplies a lesson or lyric.

`export_media_composition` is the general media timeline operation. Its Rust request has optional
ordered `visuals` and independently timed `audio` layers: at least one must be present. Visuals can
be native image takes, movie clips/edits or a prior composition; audio can be native songs, movie
sound, generated speech or prior compositions. Each source is resolved natively by identity,
verified inside the library and recorded with its hash. There are no arbitrary file inputs or
model-authored FFmpeg expressions. Video sound is included only as an explicit audio layer.

Image holds and trimmed video share a timeline. `transitionSeconds` overlaps a segment into its
predecessor; zero is a cut. Visual durations minus overlaps must equal the requested duration.
Audio layers have source offsets, placement, duration, gain and optional fades. This supports
image-only slideshows, stills plus narration, mixed image/video lessons, narration over music,
and sequential/crossfaded music excerpts without requiring a movie or music-video project.
Landscape, portrait and square frames are native fixed presets. An empty visual track exports
AAC M4A; a visual track exports H.264 MP4, with AAC only when audio was requested.

The compositor uses a one-hour timeline limit and at most 64 visual segments/64 audio layers.
It validates source ranges and probes the final streams, dimensions and duration. It does not
implicitly repeat media or claim that arranging song excerpts is model-based continuation.
The native request, exact fixed FFmpeg arguments, source hashes and output hash are saved under
`producer/compositions`. Delivery uses a new child folder and never overwrites prior exports.
`cancel_production_job` uses the request's `jobId`; partial work survives cancellation or exit.
`list_media_compositions` and `get_media_composition` expose completed and incomplete records
without resuming them. A completed ID is a reusable `composition` source.

This is a bounded compositor and artifact bundler, not a slide-authoring format, 3D reconstruction
engine or sandboxed application runtime. A set of object views remains a set of image artifacts;
an application can consume exported assets and source files. Add PPTX, visualizers, new model
providers and an application runtime as independent capabilities when needed.
Do not turn every use case into a new top-level workflow engine. Keep generation providers,
artifact adapters and delivery presets independent so new modalities do not require changes to
the agent transport. The advanced human Producer actions use this same compositor today; a
future visual timeline editor should submit the same Rust request.

For recurring characters or objects, import an image take with
`import_image_take_as_movie_reference`, attach its returned asset to the movie, and bind the
scene's `firstFrame`/`lastFrame` or native `references`. `previousScene` binds the preceding
render's endpoint. H3 does not permit first/last-frame mode and native reference mode together
within a scene. These are explicit visual signals; repeated prose is not continuity evidence.
Inspect actual render graphs and frames when validating a sequence, including identity drift.

Add new generation backends to the native studio, with Rust settings and immutable take receipts,
then expose their native commands and policy. The generated dispatcher, descriptions, desktop
controls and Computer Tasks use that same implementation. Do not add model-specific tool loops
to an adapter, a second runtime, or a provider URL to ordinary chat.

Generation, lyric timing and delivery are separate operations. A finished song can be exported
without transcription. A timing-capable music backend should preserve its native timing artifact
and provenance, then validate it against the corresponding take before publishing a lyric document.
An ABC composition describes the planned notes and syllables; it does not by itself prove when
each word was actually sung. Speech synthesis keeps its independent Whisper mistake-check and
alignment path through `synthesize_local_speech`, `align_local_speech`, and `export_narration`.

New music projects use YuE2. Existing projects without a backend field retain MiniMax Music 3,
so reopening a prior project does not change its model or reinterpret its saved takes. The human
Music Studio and native project settings both select the backend. Setup installs the pinned,
SHA-256-verified YuE2 INT8 checkpoint; the shared launcher does not require MiniMax's weights.
Existing weights are not deleted. The conservative defaults use INT8, tiled decoding, 32 steps,
and a bounded duration. BF16 can be selected when that checkpoint is already installed.

YuE2's full or melody planning can generate an ABC score, or a producer can supply/edit the score
directly. The finished take preserves the score, exact graph, resolved checkpoint name, seed,
audio hash and render settings; the score is also saved in its receipts directory. Reusing it
for a new take is explicit. Composition off skips the score stage. Native rendering uses fixed
known nodes; the app never executes an arbitrary ComfyUI graph authored by an agent.

The adapter was checked against `C:/Users/ronit/Downloads/yue2_full.json`, the user-supplied
`transcriptMusicModelYTvideo.txt`, installed ComfyUI `comfy_extras/nodes_yue2.py`, and the
[official generation guide](https://github.com/multimodal-art-projection/YuE/blob/main/docs/generation.md).
The graph produces ABC, conditioning, total seconds and audio, with no word-timestamp output.
Consequently YuE2 audio generation needs no Whisper, but word-aligned lyric exports still require
the independent alignment path. SheetSage2 reference-audio extraction is a separate future
capability; it is not required or downloaded for the current text/score-to-song path.

The noFocus visualizer reference uses a deterministic frame compositor, offline spectrum and
word-timed lyric input. Its preview/export agreement and retained original audio are useful
requirements for a future native visualizer renderer. Kestrel's existing
`apps/desktop/src/features/studio/music/MusicLyricVisualizer.ts` is also a preview reference;
it is not an offline video exporter. Keep that renderer independent of music generation.
`production_artifacts.rs` resolves shared native IDs; `media_composition.rs` composes timelines,
`media_delivery.rs` validates streams, and `production_bundle.rs` packages arbitrary selected assets.
Store renderer/version, input take hash,
timing-source identity and render settings with the finished visual artifact; feed that artifact
through the same fixed-argument packaging/verification stage as generated video. Do not run
arbitrary model-authored JavaScript or claim a visualizer exists before its real renderer is added.
