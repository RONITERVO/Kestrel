# Producer editor restoration

Producers should be able to mark a range, describe an action in one short message,
and receive a take without managing an agent loop. The reference is the 0.17 editor
at commit `4035cb4`: “generate, audition, then place”. This restores that workflow
inside the producer-owned story and scene architecture introduced in 0.18.

Acceptance scope:

- A range can span cuts, repeated masters, preserved versions and retimed clips.
  Producers can enter seconds (including decimal commas) or mark the playhead.
- Native code resolves and preserves the two endpoint frames and the exact edit
  used to select them. A short direction can become an H3 prompt in one tool-free
  local-model response. The producer controls duration and placement.
- A completed take is always a durable selectable master. Range replacement
  preserves the original edit and ripples later clips/markers by the duration
  difference. A changed timeline prevents automatic replacement.
- Interrupted jobs remain visible and never automatically resume. Reopening,
  changing scene cards and rendering scene queues must preserve editor takes.
- A shared H3 preview is visible during scene, editor and image-asset generation;
  unavailable previews are explained. Asset creation is available at each movie
  attachment surface, including later scene/frame choices and the editor.
- Verify native timing/recovery boundaries, UI interactions and layout, the full
  repository checks, and a real local Qwen/H3 generation and exported replacement.

Jobs live in `editor-generations/<uuid>/` inside each movie. Their frozen edit,
endpoint PNGs, model prompt, render graph and result hashes are durable records.
The project remains the recoverable timeline projection. Model output never
selects files, references, placement or renderer parameters.

The editor fits the full H3 frame grid to the selected duration at 24 fps. It
preserves both the original render and the fitted take, including hashes and a
media receipt. It does not trim away the conditioned final frames. Video and
audio share one FFmpeg filter graph; separate filters stalled FFmpeg 7.1.1 on an
actual H3 AAC master during acceptance. Cancellation and timeouts preserve the
original render for inspection.

The writing assistant receives scene text, not images. The endpoint previews
let the producer check whether the requested action fits the framing. An
off-screen character cannot be assumed to appear just because its action was
requested. This is a producer review step, not model-controlled frame selection.

Reference libraries hold up to 4096 assets; each scene selects at most nine
pictures, three videos and three audio signals. Search keeps large attachment
pickers bounded. Recent-take browsing reads at most 50 full records; restart
recovery reads older records one at a time and never resumes inference.

## Verified on 2026-09-07

Windows, RTX 5070 12 GB, installed Qwen3.8-27B UD-IQ2_XXS and local H3:

- Qwen wrote a plain prompt with one consistent camera instruction. H3 generated
  a 768 × 448 take at 20 steps in the complete 223.96-second editor test. Native
  preview frames were observed during rendering. Replacing one second with five
  exported a 19-second movie from a 15-second source; hashes and reopening passed.
  Project `950ccab6-e884-4e0d-b46a-b740e5a7fca8`, job
  `9e231fb6-ba04-4706-8bc3-0467fd42161e` identify the retained acceptance bundle.
- A synthetic 124-frame video with AAC audio fitted to 120 frames/five seconds
  retained its distinctive final frame and audio. The live test completes in
  under a second and fails if finishing stalls for 15 seconds.
- Real H3 image generation completed in 44.95 seconds. Its native preview,
  selectable candidates, provenance, attachment to an existing movie and
  reopening passed.
- 202 native tests, 87 contract tests and 166 UI tests passed. Clippy, architecture,
  generated bindings, TypeScript, whitespace checks and the production build passed.
- Browser layout review covered the actual UI components at desktop and 800-pixel
  widths, including the generation panel and image dialog. This used fixture IPC;
  native rendering and recovery were checked separately by the live tests above.

The ignored live tests are `live_editor_qwen_h3_replacement_and_preview`,
`live_editor_duration_fit_preserves_last_conditioned_frame`, and
`live_h3_image_pass_preserves_selectable_candidates_and_provenance`.
