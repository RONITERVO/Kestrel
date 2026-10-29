# Producer foundation acceptance

Verified on 2026-10-29 against the implementation described in
[Producer control](PRODUCER_CONTROL.md). This records release evidence and the limits of that
evidence; it is not a benchmark or a claim that every possible future workflow has been tested.

## Environment and roles

- Windows, RTX 5070 with 12 GB VRAM, 64 GB system RAM.
- Local producer: installed Qwen3.8-Flash-Next GSQ-RCO IQ3_S 125B through Strata, using the
  shared RuntimeManager inference lease. Acceptance used workspace access and a separate library.
- External producer trials: `gpt-6-luna`, low thinking, testing only. Separate trials represented
  a nontechnical customer supplying vague requests and a hands-on producer authoring content.
  Luna did not implement or repair the application.
- Real local services: ComfyUI YuE2 music, MOSS images, H3 video, Chatterbox speech, Whisper
  large-v3-turbo alignment, local research and FFmpeg. Video used the lower native defaults.
- Native receipts, transcripts, media and full logs remain in the ignored local
  `.artifacts/producer-acceptance/` directory. Session capability tokens and machine settings
  are deliberately excluded from Git.

## Required checks

All required checks passed after the final runtime repair:

| Check | Result |
| --- | --- |
| `git diff --check` | Passed |
| `cargo test --all-targets --manifest-path src-tauri/Cargo.toml` | 283 passed, 28 ignored |
| `cargo clippy --all-targets --manifest-path src-tauri/Cargo.toml -- -D warnings` | Passed |
| `npm run check` | Architecture, regenerated bindings and TypeScript passed |
| `npm test -- --run` | 229 passed across 35 files |
| `npm run build` | Passed; existing bundle-size advisory remains |

The applicable ignored live Strata test also passed with the installed model, exercising chat,
structured replies and process ownership. The managed ComfyUI launcher refresh test passed.
Hosted CI cannot replace these installed-model acceptance runs.

## Real producer trials

| Scenario | Verified result |
| --- | --- |
| Vague music/video request with local inspiration | The local producer researched opened sources, made a real YuE2 song, six low-default video scenes, two image choices and a 30-second export; one ordinary duration question was answered. Approximately 36 minutes. |
| Hands-on external producer | Authored lyrics and prompts, generated actual local music, images and video, and inspected their native records. |
| Image-conditioned continuity | A generated robot image conditioned a real first shot; the second used the preceding shot's last frame. Inspection confirmed the endpoint handoff and recurring robot/scarf identity. |
| Speech | Real Chatterbox synthesis with Whisper checking, cached word timings, M4A export and LRC/VTT/JSON/player sidecars. |
| Generic mixed-media composition | Verified a 9-second 1080p H.264/AAC result using stills, crossfade, narration and music. Frames and final media streams were inspected. |
| Independent audio and visual outputs | Verified an audio-only sequence of two songs, a silent image/video sequence, and reuse of a completed composition as a later input. |
| Generic artifact bundles | Delivered media, two JPEG choices, immutable research evidence and prose in one bundle; delivered canvas-demo source and native image/music assets in another. All original copies and manifest hashes matched; JPEG dimensions were checked. |
| Recovery and invalid paths | Cancelled composition retained incomplete native work without delivery or automatic resume. Bundle traversal was rejected without an escape file. Completed bundles remained discoverable after restart. |
| Environment discovery | A fresh producer found actual model-download, FFmpeg and local-service paths and effective generation defaults from native discovery. |
| Fresh vague non-video request | The local producer delivered a complete offline page with existing pictures, narration, research and explanatory text, described below. |

The original music-video trial predated replacement of its specialized final-export action.
That action is absent from the final implementation. The generic composition and bundle trials
above validate the replacement, including independent audio, silent visual and source-file outputs.

The final fresh customer request was:

> Make me a simple offline page explaining summer rain, with pictures and the short narration we
> already made. Put everything together in a folder I can share. Reuse existing material.

The local task `409cabf2-6437-4a4f-a6d9-79fcddbcd5d2` completed in about 6 minutes 45 seconds,
with no follow-up question. Bundle `ff914098-9878-4b07-993c-bee93654c06f` contains an authored
HTML page, two images, narration, research HTML and a README. All six manifest-listed files
matched SHA-256, every local HTML asset reference resolved, and the page contained the narration
text and no external resource requests. This was static content/link verification, not browser
rendering QA. A separate native M4A narration export also completed.

## Failures found and repaired

- A previous local page attempt exhausted its response budget and was incorrectly treated as
  complete without delivery. The runner now rejects truncated tool batches, retries at most twice
  within the existing step budget, and preserves an actionable failure if the limit recurs.
  Strata now receives its supported `reasoning_budget_tokens` field. Regression tests and the
  fresh successful page trial verify the repair; its final finish reason was `stop`.
- A real crossfade failed after timestamp normalization removed the input frame rate. Explicit
  frame-rate normalization repaired the graph; the same actual source segments and the native
  mixed-media export passed afterward.
- Producer feedback improved source-kind/export guidance, environment discovery and scene
  progress labels. Invalid frame/reference combinations fail before mutation.

## YuE2 installation and compatibility

The installed checkpoint was verified before use:

- Repository: `Comfy-Org/YuE2`
- Revision: `8e6fcf0f23252ed188b634bd50d44f4b01fba890`
- File: `yue2_3b_int8_convrot.safetensors`
- Bytes: `3960938800`
- SHA-256: `96fe199377309001ed8cd26a944baeee8cc31a20ba7c36d1d3c0a7e1f4149db6`

New projects default to YuE2 INT8 with tiled decoding. Existing projects lacking the new backend
field continue to use MiniMax Music 3; existing weights and takes are retained. Real YuE2 takes
preserve the generated/edited ABC score, exact native graph and output hash.

## Scope for merge

The durable unit is a native capability with typed arguments/results, explicit access policy,
recoverable action receipts and observable completion. Workflows compose those capabilities;
artifact adapters, media timelines and bundles are independent of output categories. The desktop,
local Computer Tasks and optional external transport use the same native implementation.

Personal-consent controls remain human actions, ordinary chat remains tool-free, and local
research keeps its loopback-only evidence rules. Interrupted tasks/plans never resume themselves.
No remote provider, MCP server, automatic upload, or background download was introduced.

Future classifiers, forecasts, 3D/world generation, an in-app canvas runtime and a native
visualizer renderer remain independent additions. Typed results already preserve numeric,
array, object, boolean and null values; they need not be text or media. The installed YuE2 nodes
do not expose verified sung-word timestamps or reference-audio/continuation input. ABC planning
is not word alignment. The new exact movie render receipts were code-verified after the real
continuity trial; that trial does not independently verify receipt emission from another render.
Local planning can still take several minutes.
