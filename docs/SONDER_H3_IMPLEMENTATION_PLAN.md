# Sonder H3 integration and producer recovery plan

Status: planned, not implemented. Audit performed 8 November 2026; priority updated 9 November 2026.

The current PR implements [resident, streaming Pocket TTS](POCKET_TTS_PLAN.md). This H3 work follows it.

## Decision

Keep Kestrel's native Rust project, task journal, commands and render queue. Repair result and
recovery correctness, then reproduce the useful H3 behavior of pinned Sonder/model-author
recipes. The producer should complete work from native facts after a simple "continue", without
needing the user to understand ComfyUI or diagnose misleading receipts.

This is a plan, not a claim of feature parity. The audit did not run a new render or change a
production. It inspected Kestrel HEAD `4119f38` (merged PR #39), installed ComfyUI 0.42.1, and
[Sonder commit 92e7f04e1d381e8fb533c0740b80b3e4f1f472a3](https://github.com/SonderSaid/ComfyUI-Sonder-Editor/tree/92e7f04e1d381e8fb533c0740b80b3e4f1f472a3).

## Current implementation and evidence

Kestrel is a Sonder-inspired native implementation, not a direct import of the editor, bridge
nodes, queue or save nodes. Rust owns its reference library, timed staging, ordinal tags, prompts,
geometry, graphs, take placement, export and compact producer digest. PR #39 referenced the older
H3 references example; the newer references/swaps workflow contains additional behavior.

Real November 7–8 user productions support retaining this foundation: an 80-second film with
16 completed takes and a separate 62-second production with a quieter revision had export files
whose sizes and hashes matched their receipts. Plain "continue" recovered step-budget and
response-limit pauses. Restart recovery preserved prior work and required explicit continuation.
These checks establish artifact production and integrity, not visual identity or exact dialogue.
Private task transcripts, local paths and identifying journal records are excluded from this plan.

### Fix current correctness before adding modes

- **Aligned guides:** `studio/h3_graph.rs` diverts guides into ordinary reference slots, loses
  frame positions, silently skips guides when slots are full, and emits no `MiniMaxH3AddGuide`.
  Its incompatibility comment is contradicted by the inspected installed ComfyUI code, which
  collects both keyframes and references. Add a live compatibility check before removing the
  workaround and implementing actual timed guidance.
- **Swap recipes:** task names and custom prompts exist, but mandatory character/head swap
  LoRAs and their complete aligned source-video route are not enforced. Generic H3 LoRAs are
  classified as style. Validate actual model identities, conditioning and prompt formats.
- **Exact recovery:** `h3_engine.rs` can reuse a failed/cancelled job's raw output using only
  scene, pass and time range. Prompt, source, reference, mask, model and seed changes are ignored.
  Match exact operation identity before adopting an output. This static defect was not observed
  in the two checked productions.
- **Truthful placement:** an observed `assetOnly` no-op returned "Placed asset ... onto scene
  timeline". Return whether a clip was created and its exact lane/ID; do not claim placement
  when only the library changed.
- **Revision-aware completion:** timeline occupancy and an existing export do not prove that a
  newly requested edit is finished. Digest coverage, next actions and export readiness must
  refer to the requested revision and its dependencies.
- **Sound preservation:** swaps conditionally freeze source audio but decode generated audio
  through the VAE; new swap clips can include audio already retained on separate lanes.
  Preserve the original sound independently when requested, and prevent duplicate placement.
- **Typed task outcomes:** a no-tool answer currently uses terminal punctuation to distinguish
  a question from completion. An observed question followed by more prose became `done`.
  Use explicit asking/completion state and evidence appropriate to the task.
- **Accurate producer guidance:** remove conflicting enum descriptions. Separate technical
  readability from semantic quality: the current `verified` flag only checks nonzero dimensions
  and a minimal duration, while wrong dimensions or missing audio can remain warnings.

Relevant code: [graph](../src-tauri/src/studio/h3_graph.rs),
[compiler](../src-tauri/src/studio/h3_prompt.rs),
[engine](../src-tauri/src/studio/h3_engine.rs),
[digest](../src-tauri/src/studio/h3_digest.rs),
[media checks](../src-tauri/src/studio/h3_media.rs),
[task runner](../src-tauri/src/agent.rs),
[action guidance](../src-tauri/src/app_control/policies.json).

## What to preserve and adapt from Sonder

The pinned upstream has substantial versioned project updates, bounded conflict reconciliation,
prompt-ID-scoped cancellation with race handling, coordinated audio/task modes, and careful H3
mask conversion. Preserve those semantics where applicable rather than assuming browser tools
lack lifecycle handling.

Directly copying the full H3 graph also carries these limitations:

- The supplied guide bridge exposes strength, but its strength output is unconnected in the H3
  example. Do not expose a control that has no effect in the selected recipe.
- The guide loader intentionally substitutes a black image when an existing guide cannot decode.
  For unattended production, return an actionable input error instead of silently changing it.
- The example uses per-frame SAM3 detection, unions matching objects and returns a zero mask on
  no detection. It relies on human mask preview. Add target identity, detection count, coverage
  and temporal checks; distinguish an intentional empty region from failed detection.
- Visible timeline content supplies source footage. A new attempt can unintentionally edit the
  previous attempt. Native operations must specify source take IDs and whether the action is an
  independent attempt or a refinement.
- Swap prompts derive from the selected replacement member, not the ordinary timeline prompt.
  Model-facing controls must communicate which fields actually affect a recipe.
- Queued conditioning is snapshotted, while render clips/audio and some bridge/placement state
  stay live. Replace implicit mutable dependencies with named prior-job/take dependencies.
- SDK release records and tokens are intentionally process-local. ComfyUI history completion
  can precede Sonder save-bridge finalization. Engine completion is not a durable committed take.
- The full canvas graph needs more models and node packs than one active operation. Use a
  capability manifest for each operation instead of requiring all optional branches.

Sources: [H3 workflow](https://github.com/SonderSaid/ComfyUI-Sonder-Editor/blob/92e7f04e1d381e8fb533c0740b80b3e4f1f472a3/example_workflows/sonder_minimax_h3_references_swaps.json),
[guide fallback](https://github.com/SonderSaid/ComfyUI-Sonder-Editor/blob/92e7f04e1d381e8fb533c0740b80b3e4f1f472a3/nodes/bridge_nodes.py#L352),
[mask compiler](https://github.com/SonderSaid/ComfyUI-Sonder-Editor/blob/92e7f04e1d381e8fb533c0740b80b3e4f1f472a3/nodes/spatial_mask.py),
[queue semantics](https://github.com/SonderSaid/ComfyUI-Sonder-Editor/blob/92e7f04e1d381e8fb533c0740b80b3e4f1f472a3/docs/generating.md#whats-frozen-vs-what-stays-live),
[run registry](https://github.com/SonderSaid/ComfyUI-Sonder-Editor/blob/92e7f04e1d381e8fb533c0740b80b3e4f1f472a3/server/sdk_runs.py),
[cancellation](https://github.com/SonderSaid/ComfyUI-Sonder-Editor/blob/92e7f04e1d381e8fb533c0740b80b3e4f1f472a3/server/run_control.py).

## Native design

1. Extend existing Rust contracts and jobs; do not introduce a second project authority.
   Each operation records requested revision, source/replacement IDs and hashes, exact trims,
   recipe version, model/LoRA identities, seed, mask artifacts and sound policy.
2. Implement versioned H3 generation, character swap and head swap recipes. Use actual aligned
   Add Guide conditioning, independent replacement references and validated frame geometry.
   Missing capabilities fail explicitly rather than falling back to a generic render.
3. Materialize graph/inputs before submission. For progressive passes, record explicit
   dependencies on completed prior-job outputs and resolve them once to immutable hashes.
   Corrupt queued specifications must not silently fall back to live settings.
4. Save per-frame masks as artifacts. Preserve original pixels outside the effective edit region
   through compositing where appropriate, and keep unchanged source audio outside the audio VAE.
   Segment and render sequentially with deliberate resource release.
5. Give UI and Strata the same semantic operations. Extend the compact digest with current
   revision, unfinished intervals, attempt/dependency state, stale exports and typed next actions.
   Provide bounded detail reads instead of forcing full-project reconstruction.
6. Recover by observing exact native receipts and files. A timeout is not proof a job stopped.
   Use bounded retries, distinct intentional attempts, scoped cancellation and preserved originals.
7. Separate submitted, rendered, committed, placed and exported states. Report technical checks
   separately from bounded visual inspection and properties that were not verified.

Use stock nodes where they express the required behavior. Consider a minimal pinned local
adapter only for an exact missing operation, with no project authority and explicit review of
the existing module boundary. Extra prompt-description inference uses RuntimeManager's existing
language-model gate; it must not spawn another helper language-model process.

"Continue" resumes unfinished authorized work. It does not repeat completed production, turn an
old export into the current revision, change the rendering method silently, authorize downloads
or Full Access, or answer a genuinely unresolved creative question. Retain the working durable
history, checkpoints, idempotency, interrupted-action reconciliation and explicit restart resume.

## Delivery order and verification

After the Pocket TTS PR:

1. Correct receipts, recovery matching, completion state and revision freshness.
2. Add faithful H3 guide/swap/audio behavior and SAM selection.
3. Validate producer continuation and compact native recovery actions.
4. Evaluate an optional H3 VFX recipe for object/effect changes such as dragon-to-helicopter.
   Treat this as experimental, distinct from head/character replacement.

Acceptance uses short real clips: one/multiple subjects, absent/wrong masks, corrupt guides,
preserved sound, interruption, repeated "continue", changed prompt at the same time range,
progressive pass dependencies, and an old export after a new revision. Compare actual behavior
with the pinned recipes, then run every repository-required deterministic check. No benchmark
suite, autonomous lab or background network work is part of this plan.

Model-author references: [character swap](https://huggingface.co/Alissonerdx/Minimax-H3-ComfyUI/blob/main/docs/character-swap.md),
[head swap](https://huggingface.co/Alissonerdx/Minimax-H3-ComfyUI/blob/main/docs/head-swap.md),
[experimental VFX](https://huggingface.co/Alissonerdx/Minimax-H3-ComfyUI/blob/main/docs/vfx-edit.md).
