# MOSS native resolutions and official-example comparison

Follow-up acceptance on 2026-10-21, RTX 5070 12 GB / 64 GB RAM, permanent ComfyUI v0.42.1.
No additional model weights were installed. Generated media and original user projects remain intact.

## Supported scope

The [technical report](https://arxiv.org/html/2610.23374v1) describes a final training-stage area
cap of 1024² pixels. The [official guide](https://github.com/OpenMOSS/MOSS-Image/blob/main/USERGUIDE.md)
recommends approximately 1–2 MP and permits extrapolation up to approximately 4 MP, with dimensions
divisible by 16. Training resolution and supported extrapolation are different claims.

Kestrel now offers **2048×1152 (2K, 16:9)** and **2560×1440 (QHD, commonly called 2.5K)**.
Both generate directly at the selected size. There is no upscaler or resized export. Above 2 MP,
Compose explains the experimental quality range. Advanced allows sides up to 2560 while Rust
enforces the existing 2048² total-area budget, alignment and 6:1 aspect limit. Portrait QHD can
be entered manually. Existing 2048-square projects remain valid.

The producer explicitly chose to leave 3840×2160 UHD 4K out. Its 8.29 MP exceeds the published
range. The 2K preset is **not DCI 2048×1080**; 1080 is not divisible by 16, and an exact DCI
delivery would require a separate crop policy.

The released Base workflow has no dedicated reference-image conditioning input. The guide lists
reference-to-image generation under future post-training work. Kestrel's backdrop is still a
visual alignment aid. The separately published [community color/depth adapter](https://huggingface.co/darask0/MOSS-Image-color-depth-control)
is unofficial, controls spatial maps, and is neither installed nor a tested identity/style reference path.

## Steps and official example

The [official Comfy workflow](https://github.com/OpenMOSS/MOSS-Image/blob/main/comfyui/moss-image/example_workflows/moss-image-api.json)
uses 50 steps, CFG 3, shift 5, Euler / sgm_uniform and unconditional layer-22 skipping.
Kestrel Standard matches these settings. Quality uses 75 steps, within the guide's recommendation
of at least 50; more steps are not a quality guarantee. Draft remains 30 steps and is explicitly
labelled a preview. It is not a distilled model.

The test used the official refrigerator / Chinese shopping-note prompt verbatim, seed 42,
1024², and an empty negative prompt. Its HF-directory loader was replaced by the already-installed
native INT8 component loaders. As required by the [native migration instructions](https://github.com/OpenMOSS/MOSS-Image/blob/main/comfyui/README.md),
the plugin's ModelSamplingSD3 override was omitted: native MOSS already uses shift 5 and multiplier 1.
This is an adapted official example, not a bit-identical reproduction of full-precision HF inference.

The comparison uses the same installed weights and settings through Kestrel's actual create/save/render
pipeline, completing its normal style/background fields and adding object/text boxes. This tests
complete prompt strategies; extra scene treatment and placement instructions mean it is not a
serialization-only ablation. No local language model rewrote either prompt.

| Take | Native size | Time | Visual review |
| --- | --- | ---: | --- |
| Official prose | 1024×1024 | 38.0 s | Correct three-line Chinese note; refrigerator fills most of frame and fruit bowl is partly cut off. |
| Kestrel structured composition | 1024×1024 | 32.6 s | Correct note and requested left/right placement; fruit bowl is fully visible. Unrequested white print border and softer background. |
| 2K poster | 2048×1152 | 76.8 s | Correct EXPLORE, one mountain, clean palette and edges. |
| QHD poster | 2560×1440 | 139.0 s | Correct EXPLORE and one mountain; thinner lettering and a different snowcap. Title extends beyond its horizontal box. |
| QHD kitchen photograph | 2560×1440 | 143.1 s | Note remains readable and subjects occupy their intended sides. Soft photographic detail and an unwanted dark frame; higher resolution does not improve every quality attribute. |

These are individual examples, not a statistical ranking. The official prose works well without
an elaborate layout. Kestrel's format buys explicit composition control, not guaranteed aesthetic
or text-quality improvements. All timings include local submission/processing and are observations,
not controlled performance comparisons.

The producer observed physically inconsistent sharpness around the note in the Kestrel example.
Neither prompt requests blur, shallow depth of field, selective focus, a lens or an aperture.
Kestrel adds `Photograph`, everyday-scene aesthetics, natural kitchen lighting, a wide-view
treatment and a kitchen background. Four boxes separate the refrigerator, counter, fruit bowl
and exact note text; the text caption explicitly preserves three lines. Preferential text-region
rendering is a plausible explanation for the sharp note against a softer door, not an isolated
causal finding. The official prose has no instruction that prevents this artifact. As requested,
no focus-related fix or prompt change was made. Reproduce the 1024-square comparison with seed 42,
Standard 50, empty negative conditioning and the saved exact prompts/graphs in the local gallery.

## Placement and reproducibility

Both high-resolution PNG dimensions and SHA-256 values match their immutable take receipts.
Their initial latent dimensions equal the output dimensions and neither graph contains an upscale
node. Blue-pixel masks put 100% of the 2K title and mountain ink inside their requested boxes.
At QHD, title containment falls to **89.4%**, while mountain ink containment is **99.998%**.
The QHD title is centered closely but extends 59 pixels left and 37 right beyond its box.
The mask excludes the cream snowcap, so its blue-ink bounds are not the whole mountain's bounds.
Approximate placement remains a model limitation; these images do not establish strict box parity.

Reproduce the poster runs with the existing ignored `live_image_producer_acceptance` test and
`KESTREL_IMAGE_CASE=native-2k,native-qhd`; their exact edits are in
`tests/image-acceptance/cases.json`. Use `KESTREL_LIVE_COMFY_ROOT` for the selected installation
and `KESTREL_IMAGE_ACCEPTANCE_ROOT` for a separate evidence folder.

Full images, upstream source snapshot/hash, adapted graph, proposals, receipts and mask measurements
are retained under `.artifacts/moss-acceptance/official-example-2026-10-21/`. The local `review.html`
compares images with optional requested-box overlays. Compact results are in
`tests/image-acceptance/resolutions-2026-10-21.json`.

Verification: 263 native tests passed (27 explicitly live tests ignored by the ordinary suite),
209 core tests, 224 UI tests, Clippy with warnings denied, architecture/bindings/TypeScript,
UI build and diff checks. The release application was rebuilt for the desktop shortcut.

## Plain prompts through Kestrel

Image → Compose → Prompt mode → **Plain prompt · exact text** now sends precisely the positive
text the producer enters. It skips automatic style/background additions and ignores the saved
layout boxes. Both the composition and plain draft remain editable after switching modes and
reopening a project. An optional **Use producer brief** button copies the brief explicitly;
**Develop design** is not required. Negative prompt is editable directly in this mode, including
without Advanced enabled. Clear it to reproduce the official example's empty negative branch.

Two additional real runs used Kestrel's create/save/begin/render/copy/dimension-check/hash path
with the same installed model, seed 42, 1024×1024 and Standard 50:

| Prompt | Time | Verification and visual result |
| --- | ---: | --- |
| Verbatim official refrigerator prompt | 58.7 s, including cold startup | Four inactive boxes and all composition fields remain saved, but only the original prose reaches CLIP. **Decoded RGB pixels match the earlier direct official-workflow take exactly.** The same close crop, readable Chinese note and partially cropped fruit bowl remain. |
| `a cat` | 32.6 s | Empty style/background/high-level fields, no boxes, no agent rewrite. A coherent close-up cat photograph with dark lighting; the model chose framing and treatment from the minimal description. |

These samples demonstrate the exact-text path, not a guarantee that vague prompts produce every
intended detail. Exact input, graph nodes, output dimensions, file hashes and preservation of
inactive composition fields were checked; images were also visually inspected. Timings include
runtime preparation and are not controlled performance comparisons. PNG file hashes differ from
the earlier official take because workflow metadata differs, despite identical decoded pixels.

Compact results: `tests/image-acceptance/plain-prompt-2026-10-21.json`. Full proposals, native
projects, immutable receipts, two images and a comparison gallery are under
`.artifacts/moss-acceptance/plain-prompt-2026-10-21/`. Reproduce either proposal using
`KESTREL_IMAGE_PROPOSAL` with the existing ignored `live_image_producer_acceptance` test.

Schema 5 defaults older projects to Composition while retaining prior take receipts. Regression
checks cover whitespace/Unicode fidelity, short and empty prompts, size bounds, mode persistence,
hidden-box keyboard protection, negative prompt access and save-before-render. Browser preview
checks at 1280×720 verified pagination, switching modes and restoration of the existing layout.
Verification: 264 native tests passed (27 live tests ignored), 210 core tests, 226 UI tests,
Clippy with warnings denied, architecture/bindings/TypeScript, UI build and diff checks.
