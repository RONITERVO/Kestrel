# MOSS-Image replacement acceptance

Status, 2026-10-21: **Implementation and headless comparison complete. Draft PR; not cleared for merge.**
The producer must accept the observed placement tradeoff and complete the native UI check below.
Successful image files alone are not quality acceptance.

## Integration

The requested [video](https://www.youtube.com/watch?v=cUeZMUGCxrw) describes MOSS-Image.
This integration uses Base, the variant documented for experimental layout control, through native
ComfyUI nodes. It does not install a separate custom-node plugin, LoRA trainer, or image-editing model.

Primary references:

- [OpenMOSS model and guide](https://github.com/OpenMOSS/MOSS-Image/blob/main/USERGUIDE.md)
- [Native integration instructions](https://github.com/OpenMOSS/MOSS-Image/blob/main/comfyui/README.md)
- [ComfyUI v0.42.1 layout conversion](https://github.com/Comfy-Org/ComfyUI/blob/v0.42.1/comfy_extras/nodes_moss_image.py)
- [Pinned model distribution](https://huggingface.co/Comfy-Org/MOSS-Image/tree/ee64003102c8470ee32501cf883419235db81884)

Setup pins ComfyUI v0.42.1, Base INT8 ConvRot, Qwen3.5 9B INT8 ConvRot, and Kestrel's existing
full-quality Flux 2 VAE by exact bytes and SHA-256. The existing editor and collaborator schema
retain normalized `[top, left, bottom, right]` coordinates. Rust converts lower edges with floor,
upper edges with ceil, and subtracts half the canvas height/width. This matches the official node
and preserves nonempty boxes at small sizes. Text, line breaks, item order and element palettes
survive translation. Global style and background become the scene caption. Without boxes, the
model receives a natural-language caption, matching the upstream node's free-form path.

Sampling uses native shift 5, Euler / sgm_uniform, CFG 3 and negative-branch layer-22 skipping.
Standard uses 50 steps, Quality 75, and Draft 30. The legacy stored `turbo` preset maps to Draft;
this is not a distilled checkpoint. Negative conditioning is independently editable. The old
alignment backdrop remains review-only. Historical takes keep their original model, terms and
inference receipts; a new render uses MOSS. Existing custom collaborator prompts remain editable
and compatible with the unchanged composition schema.

## Comparison protocol

The ignored developer tests use Kestrel's real create → save → begin generation → render → copy,
dimension-check and hash pipeline. Ideogram's original adapter and graph are retained under
`cfg(test)` solely for the requested comparison. They are not a production fallback. Both models
use the same isolated ComfyUI runtime and GPU, their own correct prompt formats, and their own
published sampling presets. A numeric seed is reproducible within a model, not equivalent noise
or content across different models.

`tests/image-acceptance/cases.json` defines reproducible producer edits and explicit visual criteria:
square/portrait/landscape box placement, exact text, Unicode and line breaks, free-form photography,
art and palette fidelity, overlap/layer intent, 6:1 aspect ratio, batches of two and four, minimum
and maximum canvas dimensions, all sampling presets, negative conditioning, repeated fixed seeds,
and random seeds. Negative prompts have no equivalent in the old Ideogram graph; record that
capability difference rather than scoring an unsupported baseline as a failure.

For each pair, review subject fidelity, placement, typography, style/material quality, unwanted
artifacts, latency and failures. Simple colored-shape cases additionally measure actual colored
pixel bounds, containment and center error against the requested boxes. Compare decoded-pixel
hashes for repeated fixed seeds. Do not equate a correctly converted box with a model following it.

The live collaborator test runs the same `PromptDraftJob`, structured request preparation, SSE
decoder and `RuntimeManager` lease used by the desktop. It records real initial and revised Develop design
proposals, reasoning events, request receipts and user briefs for the selected local model.
It invokes `scripts/image-acceptance-proposal.ts` to apply each proposal with the actual UI parser;
the next request receives the resulting UI composition preview.
This separates agent planning errors from renderer adherence and from deliberately authored
model-ready compositions. UI tests cover explicit proposal application, invalid checkpoints,
save-before-render, normalized edits/layers, new controls and cancellation dispatch.

## Observed results

The run used an RTX 5070 with 12 GB VRAM, 64 GB system RAM, Windows, and an isolated copy of
ComfyUI v0.42.1. Model files were verified against their pinned hashes. Kestrel used its normal
low-VRAM/offload launcher with Hugging Face and Transformers offline flags. The existing shared
ComfyUI installation and production image projects were not modified. This is a finite acceptance
sample, not a statistical model ranking or a promise about every possible prompt.

**38 case/model runs produced 46 full-resolution PNGs:** 17 authored cases and two real-agent
stages for each renderer. Every PNG's dimensions and SHA-256 matched its immutable take receipt.
Both two- and four-image batches contained distinct decoded images. Repeated fixed seeds reproduced
the initial pixels exactly for both models. Random-seed runs recorded nonzero seeds. Separate live
cancellation/recovery tests reached submitted Comfy prompts, drained the queue, preserved existing
completed takes byte-for-byte, and marked interrupted work without resubmitting it.

The checked-in [compact evidence](../tests/image-acceptance/results-2026-10-21.json) contains all
per-case timings, visual findings, image hashes and box measurements. Full images, exact graphs,
prompt receipts, agent streams, cancellation receipts, GPU samples and the side-by-side gallery
are retained locally under `.artifacts/moss-acceptance/results/`; they are intentionally not
published with the source. Open `review.html` there and toggle the requested-box overlay.

| Area | Ideogram observation | MOSS observation |
| --- | --- | --- |
| Square/portrait boxes | Stronger offset adherence; distorted shapes and unwanted black outlines/grid | Cleaner shapes and no outlines, but square/portrait vertical offsets weakened |
| Landscape boxes | Good placement; thin unwanted circle outline | Good placement and clean shapes |
| Exact poster text | Correct EXPLORE; textured cream paper | Correct EXPLORE; flatter clean artwork |
| Unicode and line breaks | Correct CAFÉ, HELSINKI and 茶 • 2026 | Correct accents, Chinese, punctuation and line breaks |
| Photo, free-form | Good cup/steam/rain detail, ruined by unwanted safety-message lettering | Clean cup/rain scene, steam faint or absent |
| Overlap and palettes | Correct circle behind triangle and foreground FORM | Correct order and lettering, darker orange |
| 6:1 canvas | Correct LOCAL and opposite-side objects; slight shape widening | Same semantic success and slight shape widening |
| Two vase variations | Both replaced by safety-message images | Two coherent vases; one unwanted cream inset panel |
| Four Draft birds | Distinct birds; one extra logo, one border | Four distinct clean geometric birds |
| 2048 Quality fern | Detailed veins/glaze; cropped pot | Detailed veins/glaze, whole pot; some fronds cropped |
| 256 minimum | Blue circle plus unwanted red fragments/letters | Clean blue circle and white margins |
| People | Correct positions/colors, plausible faces/hands; cropped lower bodies | Correct positions/colors and plausible people; unwanted white print border |
| Negative conditioning | Unsupported; negative/control pixels identical | Pixels changed slightly, but visible exclusions were weak |
| Random vase | Safety-message image instead of vase | Correct vase with an unwanted cream inset |

The Ideogram safety-message text above is present in the generated pixels, including a misspelling
in one case; it is not a Kestrel error banner. These are content failures despite valid PNG files.
MOSS still occasionally adds a panel/border or misses a detail, and negative prompts are guidance,
not a reliable exclusion switch.

### Bounding boxes

Numbers below describe the largest red/blue connected color region. IoU compares its rectangular
bounds to the requested box; center error is a percentage of the canvas diagonal. This does not
score whether the subject is a true circle/square or detect black outlines, so read it with the
visual findings. A subject can legitimately occupy less than its placement region.

| Canvas / subject | Ideogram box IoU | MOSS box IoU | Ideogram center error | MOSS center error |
| --- | ---: | ---: | ---: | ---: |
| Square / red | 0.495 | 0.675 | 0.15% | 5.52% |
| Square / blue | 0.980 | 0.536 | 0.11% | 8.70% |
| Landscape / red | 0.972 | 0.987 | 0.32% | 0.06% |
| Landscape / blue | 0.981 | 0.992 | 0.12% | 0.00% |
| Portrait / red | 0.641 | 0.585 | 1.79% | 8.32% |
| Portrait / blue | 0.985 | 0.587 | 0.14% | 8.34% |

Only 87.2% of the square MOSS blue region stayed inside its requested box. The adapter's coordinate
math matches the upstream implementation and is unit-tested across aspect ratios and narrow boxes;
the observed drift is renderer adherence, not evidence of a coordinate conversion error. **Strict
placement parity is not established.** The normal Compose panel now explains that edges/offsets
may vary, so this changed behavior is visible without Advanced mode.

### Real collaborator and revision

Both prompt packs ran through the selected local Qwen3.8-Flash-Next GSQ-RCO IQ3_S model on Strata,
using the same streaming job, inference lease and UI parser as Kestrel. Initial and revised
proposals parsed successfully. Both correctly planned and rendered the requested left/right swap
of a cobalt vase and orange sphere while retaining NIGHT / FORM and HELSINKI 2026.

The Ideogram initial proposal included curly quotation marks in the exact text, and those marks
appeared in the image: an agent planning issue. Its first image also duplicated the sphere and
both images added panel lines. The MOSS pair had single separated objects, legible wording and
clean cream backgrounds. Its initial title used mixed-height stylized glyphs. Both renderers
changed the vase shape on revision: this workflow regenerates from a revised design, not from an
input image, and does not preserve object identity. The backdrop remains review-only.

### Time and resource observations

| Render | Ideogram | MOSS |
| --- | ---: | ---: |
| Warm repeated square | 36.7 s | 32.7 s |
| Landscape boxes | 62.9 s | 52.9 s |
| Four 512-square Draft variations | 16.5 s | 20.6 s |
| 2048-square Quality fern | 774.3 s | 249.7 s |
| Agent initial / revised poster | 42.7 / 40.6 s | 34.6 / 34.7 s |

These are complete producer render timings with each model's own preset (Ideogram 20/48/12 steps;
MOSS 50/75/30), not equal-compute speed measurements. First-load timings include cold loading;
the first Ideogram square took 107.1 s. Two-second GPU sampling observed up to 11,478 MiB total
device memory during MOSS renders. It includes unrelated device allocations and may miss short
peaks. Early Ideogram runs lack full telemetry coverage. No out-of-memory failure occurred in
this matrix; this does not establish every resolution/batch combination on every 12 GB GPU.

Both models used v0.42.1 so the runtime was held constant; the original Ideogram prompt adapter,
graph, weights and presets were retained. This does not compare an untouched v0.33.1 installation
against v0.42.1. One early MOSS Unicode input was corrupted by a Windows script's default encoding;
that invalid fixture/output is archived separately and excluded. The fixture was restored as
UTF-8, guarded by explicit assertions, and rerun before scoring.

## Verification

- `cargo test --all-targets --manifest-path src-tauri/Cargo.toml`: 259 passed, 24 ignored.
- `cargo test --manifest-path crates/app-core/Cargo.toml`: 209 passed, including the box adapter.
- `cargo clippy --all-targets --manifest-path src-tauri/Cargo.toml -- -D warnings`: passed.
- `npm run check`: architecture, generated bindings and TypeScript checks passed.
- `npm test -- --run`: 221 passed in 35 files, including seven image desk flow tests.
- `npm run build` and `git diff --check`: passed.
- Explicit live authored renders, real collaborator flows, and cancellation/recovery: passed
  their operational checks; visual outcomes are recorded separately above.

The headless UI tests cover proposal/apply/save/render, malformed checkpoints, drawn/dragged/resized
boxes, keyboard edits/layer order, batch/preset/negative/seed controls, generation locking and Stop,
historical model terms without Advanced mode, and selected-take handoff without another render.
They use mocked IPC and jsdom layout; native renders and local-model streaming were exercised
separately through the actual Rust paths. Setup assets were downloaded and hash-verified; the
interactive Setup installer, native file dialogs, actual WebView layout and physical pointer
interaction remain part of the manual check.

### Existing-project runtime regression

The first native UI check exposed a case the original fresh-project matrix missed: an existing
Ideogram project retained its old ComfyUI folder after Setup selected a new installation. The
renderer also attached to any server answering on port 8188. When the old server lacked MOSS
nodes, the failed take was saved correctly, but the progress strip disappeared and the old
successful take kept the project status `ready`, hiding the error from the canvas.

New image generations now read the current Setup folder in the native command and freeze it with
the new batch. Older take receipts remain unchanged. Before submitting a graph, Image Studio
checks the running server's reported absolute `main.py` path against the selected installation.
A mismatched or unverifiable server produces an explicit stop-and-retry instruction; Kestrel does
not terminate an unrelated server. Persistent failures are visible on the canvas after generation
ends and after reopening the project, with individual errors available under Takes. Shared render
errors use the media-service label instead of incorrectly naming H3 for image failures.

Regression coverage includes legacy folder replacement without changing old receipts, invalid
folder rejection without saving changes, matching/mismatched/unverifiable runtime identity, and
failure display while earlier takes remain usable. The opt-in tests
`live_existing_image_project_uses_selected_setup_runtime` and
`live_image_rejects_a_different_comfy_installation` exercise the actual local service boundary.
The former reads `KESTREL_IMAGE_EXISTING_PROJECT`, copies the project into the acceptance folder,
and verifies that the source manifest and completed PNG hashes remain unchanged.

Both live checks passed. The copied existing project generated and preserved a new 1536×1024
MOSS image in 53.1 seconds, with all five earlier take records and the original completed image
unchanged. Visual inspection confirmed a new coherent castle/forest/snow composition. The source
project manifest was byte-identical after the test. The receipt is retained locally at
`.artifacts/moss-acceptance/results/runtime-migration/existing-project.json`. The corrected native
release executable was also built successfully; the producer still needs to retry in that build.

## Reproduction

Use an isolated ComfyUI directory containing the pinned assets and no competing GPU job. All
results belong outside the production research library. Tests are opt-in and never run on startup.

```powershell
$env:KESTREL_IMAGE_ACCEPTANCE_ROOT = 'D:/path/to/acceptance/results'
$env:KESTREL_LIVE_COMFY_ROOT = 'D:/path/to/isolated/ComfyUI'
$env:KESTREL_IMAGE_BACKEND = 'moss' # repeat with ideogram
cargo test --manifest-path src-tauri/Cargo.toml live_image_producer_acceptance -- --ignored --nocapture

$env:KESTREL_IMAGE_CONFIG_ROOT = 'C:/Users/producer/Kestrel Research'
# For the baseline, set KESTREL_IMAGE_PROMPT_PACK to the original default.json saved from main.
cargo test --manifest-path src-tauri/Cargo.toml live_image_collaborator_producer_flow -- --ignored --nocapture
# The test already writes develop/revise.edits.json through the actual UI parser.
$env:KESTREL_IMAGE_PROPOSAL_NAME = 'agent-develop' # or agent-revise
$env:KESTREL_IMAGE_PROPOSAL = 'D:/path/to/acceptance/results/agent-moss-producer/develop.edits.json'
cargo test --manifest-path src-tauri/Cargo.toml live_image_producer_acceptance -- --ignored --nocapture

Remove-Item Env:KESTREL_IMAGE_PROPOSAL -ErrorAction SilentlyContinue
cargo test --manifest-path src-tauri/Cargo.toml live_image_cancel_and_restart_preserve_completed_takes -- --ignored --nocapture
# Use a Python with Pillow, NumPy and SciPy (the isolated ComfyUI Python has these).
python scripts/image-acceptance-review.py 'D:/path/to/acceptance/results'
```

`KESTREL_IMAGE_CASE` optionally selects comma-separated case names. Each case manifest records
full project receipts and starts with `qualityVerdict: pending`. A render failure retains the
failed project and error. Resume must not turn missing, changed or failed evidence into a pass.

## Merge gate

Do not give an unconditional green to merge from this report. The observed box-placement change
is a producer decision, and native UI verification is still required. No source or graph repair
has been identified that would justify claiming exact placement parity.

Run this branch with `npm run tauri dev`. For the prepared local environment, select the existing
folder `.artifacts/moss-acceptance/ComfyUI_windows_portable/ComfyUI` in Setup. This avoids another
model download. Do not overwrite the separate shared ComfyUI installation. The app uses the normal
Kestrel library; use a new throwaway image project for this check.

1. Confirm Setup recognizes MOSS and the folder; inspect the production-suite/model-reuse UI.
2. Create a poster brief, Develop design, inspect the proposal, Apply, then Create. Confirm the
   progress, full-resolution viewer, contact sheet and preserved take agree.
3. Draw, drag and resize object/text boxes, cycle overlaps and change layer order. Render a
   square and portrait design with deliberate vertical offsets; decide whether the observed
   placement is acceptable for your use.
4. In Advanced, try Draft, a negative prompt, fixed seed and multiple variations. Stop one render,
   then reopen the project; completed takes must remain selectable and unchanged.
5. Open an existing Ideogram project. Confirm its old take/model terms are visible and unchanged;
   a new render should identify MOSS. Check backdrop opacity and the placement note at normal scale.
6. Download a take, reveal its folder, and hand a selected take to Movie Studio. Verify native
   dialogs/navigation and that the chosen image is used without regenerating it.

After those checks, either explicitly accept approximate layout control and the documented visual
tradeoffs, or keep the PR draft and specify the placement requirements that remain unmet.
