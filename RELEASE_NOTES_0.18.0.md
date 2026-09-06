# Kestrel Local 0.18.0

Movie Studio starts with a short idea and lets the local model do the creative writing. The producer
keeps control of references, frames, audio, and render approval. Native code manages long queues,
because reliable movie production should not depend on a small model coordinating tools or writing
an entire batch correctly in one response.

## Producer workflow

- Complete Markdown story revisions with direct editing and tool-free collaboration.
- A distinct written scene for every clip. Request 1–4096 scenes with one action; native code writes
  and checkpoints one at a time. Stop and explicitly resume without repeating saved scenes.
- The faster 768 × 448 video preset is the default; 1344 × 768 is an explicit higher-detail choice.
- Clear frame/reference compatibility controls that preserve the producer's saved media choices.
- Paginated scene cards, masters, and timeline index, with visible-window timeline rendering.
- Bounded FFmpeg exports that avoid Windows command-line limits for long cuts. Scene saves retain
  repeated timeline items, order, trims, preserved source versions, and custom export titles.
- Native UUIDs for chat and manual scene cards, preserved partial responses, and protection against
  sending a story revision while direct edits remain unsaved. Structured scene data appears as cards.

## Verification on 7 September 2026

The required checks passed: whitespace validation, 194 Rust tests (17 explicit service tests ignored),
Clippy with warnings denied, architecture and generated-binding checks (80 contract tests),
TypeScript, 160 frontend tests, and the production frontend build.

Explicit local acceptance used an NVIDIA RTX 5070 with 12 GB VRAM:

- Qwen3.8-27B, UD-IQ2_XXS, low thinking: a short idea became a complete accepted story and three
  separately saved five-second scenes through the production `RuntimeManager` and Studio jobs.
  The final run completed in 171 seconds including startup. The prose and scene progression were
  inspected; camera guidance was consistent and the postcard scene did not demand long readable text.
- The first Qwen acceptance's three scenes rendered through local MiniMax H3 at 768 × 448 and
  20 steps, then exported as a 15-second H.264/AAC movie. The render-and-export run took 549 seconds.
  Sampled frames were inspected. Model output still requires the producer's creative review.
- A separate 17-item FFmpeg test crossed the 16-input grouping boundary, checked output duration
  and both media streams, verified source/export hashes, and checked scratch cleanup.
- A 1440-scene queue checkpoint survived restart and resumed without repeating its saved scene.
  A two-hour editor fixture checked bounded controls and access to later masters and timeline items.
- The offline NSIS packaging process produced a portable application, an installer containing
  WebView2, version checks, a manifest, and SHA-256 checksums. The installed older preview loader was
  repaired using Setup's pinned, hash-verified components and passed a real H3 decoder check.

## Distribution and acceptance scope

This is a source-code release. Unsigned Windows development artifacts remain local; this release
does not distribute a signed installer. Tests used the existing PC and installed local services,
not a clean Windows machine. A full multi-hour model-writing or H3-rendering production and the
whole application's disconnected-network installation matrix were not run. Queue capacity,
restart, editor scale, and grouped export were exercised separately; those checks do not establish
creative coherence for thousands of generated scenes. See [RELEASING.md](RELEASING.md) for the
separate signed-installer acceptance process.
