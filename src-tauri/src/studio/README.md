# Studio maintainer guide

Kestrel Studio is producer-owned — whether driven by a human in the desktop UI or by the local
Producer LLM through shared 1-to-1 `app_control` native capabilities. Rust stores the creative
record and performs every durable, prompt-compiling, graph-constructing, and media-affecting
operation; React renders an editable view of that record.

## Non-negotiable boundaries

- Studio remains useful without the public network. Model HTTP and ComfyUI traffic are loopback
  traffic only (`127.0.0.1`); there is no remote fallback. Optional Turbo LoRA installation uses
  an explicit allowlisted Hugging Face commit (`Comfy-Org/MiniMax-H3`) with SHA-256 verification
  and never runs automatically.
- `RuntimeManager` owns the only language-model process and its semaphore owns the only inference
  slot. Prompt drafting, music lyric drafting, research, ordinary chat, and Computer Tasks all use
  that gate.
- H3, MOSS-Image Base, Ideogram, and Music 3 / YuE2 rendering start only after language-model
  inference releases its lease and Kestrel unloads the model runtime from the GPU.
- `project.json` (`schemaVersion = 9`), reference library objects, render receipts, raw H3 masters,
  placed timeline clips, and export receipts are durable user data. Interrupted jobs are surfaced
  as `interrupted` on restart and are never silently resumed or discarded.
- Model output is never executed. Parse bounded typed data, validate IDs and sizes, use fixed
  command argument arrays without a shell, and persist the native result.

## Module ownership

| Module | Owns | Must not own |
| --- | --- | --- |
| `studio.rs` | Content-addressed reference store (`_references/`), ComfyUI process lifecycle, recoverable `project.json` (`schemaVersion = 9`) | React view state or arbitrary shell execution |
| `h3_geometry.rs` | `17k + 5` frame alignment, H3 video VAE `[1, 4, 4, 4, 4]` token snapping, `40 Hz` audio mask snapping, aspect-preserving pass canvas scaling, `authored_v1` take placement | Filesystem mutation or network I/O |
| `h3_references.rs` | `@Handle` normalization, window overlap + tightest-span specificity overrides, `9/3/3` H3 reference caps, `<Subject>`/`<Picture>`/`<Video>`/`<Audio>` slot numbering, `auto_stage_reference` | ComfyUI graph construction or FFmpeg execution |
| `h3_prompt.rs` | 6-channel `H3FullReference`, 3-channel `H3Base`, and `CharacterSwap` / `HeadSwap` compilation, `<d>[Language] ...</d>` vocal events, 3-pass Turbo LoRA auto-binding | Filesystem mutation or ComfyUI submission |
| `h3_graph.rs` | Pure stock ComfyUI (`v0.42.1+`) MiniMax H3 workflow JSON (`MiniMaxH3ReferenceToVideo`, `MiniMaxH3ImageToVideo`, `MiniMaxH3SigmaShift`, `MiniMaxH3AddGuide`, tiled VAE, AV noise masks, optional `taeh3` live preview) | Custom third-party Python workflow nodes or non-H3 video models |
| `h3_media.rs` | Cropped/trimmed reference staging, spatial mask PNG generation, multi-lane window source video/WAV rendering, ridge-regularized 3×4 affine color-drift cancellation, take quality telemetry + 3 contact-sheet PNGs | Destructive edits to raw masters |
| `h3_digest.rs` | Deterministic `< 4 KB` `H3ProductionDigest` snapshot (coverage spans, timeline gaps, entity counts, recent telemetry, concrete `nextActions`) for 64k-context Producer LLM resilience | Mutating project state |
| `h3_engine.rs` | Shared H3 project/scene/entity/timeline commands, 3-pass quality ladder expansion (`0.2MP` -> `1MP` -> `1080p`), ComfyUI queue execution, non-destructive lane placement, legacy `v8` in-memory migration | Auto-resuming interrupted queues |
| `export.rs` | Bounded `30s`-chunk multi-lane scene exports (`publish`, `archive`, `lossless`, `review`, `audio_flac`) with BT.709 signaling and measured linear peak-headroom audio scaling (`-1 dBFS`) | Source master mutation |
| `prompt_draft.rs` | Tool-free story/image/reference/music drafting from producer context | Applying proposals or rendering |
| `model_stream.rs` | OpenAI-compatible SSE framing and explicit reasoning-channel extraction | Feature prompts, schemas, or persistence |
| `image_assets.rs` | Listing and recovery of legacy stills | Generating new images |
| `image_studio.rs` | Recoverable image projects, structured composition, native MOSS/Ideogram graphs, immutable PNG takes | LLM process ownership or public-network fallback |
| `live_preview.rs` | Bounded process-local `taeh3` preview state and visible preview events | Durable final-render truth |
| `music.rs` | Recoverable song projects, producer arrangements, native Music 3 / YuE2 graphs, immutable takes and lyric revisions | LLM process ownership |
| `music_lyrics_model.rs` | Bounded audio-listening and translation suggestions through a caller-owned lease | Durable lyric mutation or runtime ownership |
| `music_midi.rs` | Bounded MIDI parsing/writing and recoverable piano-roll revisions | Source mutation or arbitrary path selection |

## MiniMax H3 workflow & 3-pass quality ladder

1. **Entities & Reference Library**: Create `@Handle` entities (`Character`, `Location`, `Prop`,
   `Outfit`, `Voice`, `Sound`, `Other`) backed by local media or 1-click imports from Image Studio,
   Speech Studio (`.opus`), and Music Studio.
2. **Multi-Scene Timelines**: Use `Standard` (`1920×1088`), `Audition` (`960×544` voice/turnaround
   tests), and `Soundtrack` (`64×32` minimal-video music/ambience generation).
3. **Reference Lanes (`R1..Rn`)**: Stretch `@Handle` items over `[start..end]`. Tighter spans
   override wider spans for the same `@Handle` without deleting underlying lanes.
4. **Prompt Sections & Guides**: Write action in `Global Prompt` + `[Shot k]` sections using
   `@Handle` names and `<d>[Language] ...</d>` vocal events; pin keyframes or pose videos on the
   Guides track (`MiniMaxH3AddGuide`).
5. **3-Pass Quality Ladder**:
   - **Pass 1 (`0.2MP` Draft, ~`608×352`)**: `er_sde` + `sgm_uniform`, `10` steps, Ref2V Turbo LoRA
     `@ 0.8` (or `20` steps without LoRA), `denoise = 1.0`. Locks motion, acting, and dialogue sync.
   - **Pass 2 (`1MP` Detail, ~`1344×768`)**: `12` steps, Ref2V Turbo LoRA `@ 0.5`, `denoise = 0.75`
     over Pass 1's placed master (`KeepExistingAudio` optional when locking Pass 1's voice performance).
   - **Pass 3 (`1080p` Upscale, `1920×1088`)**: `4` steps, Ref2V Turbo LoRA `@ 0.5`, `denoise = 0.40`
     for crisp facial identity, fabric texture, and lighting fidelity.
6. **Inspect & Deliver**: Call `inspect_movie_h3_take` for FFprobe/peak/RMS telemetry + 3 extracted
   contact-sheet PNGs, compare A/B takes on `V1..Vn`, mix dialogue/SFX/score on `A1..An`, and
   render an immutable BT.709 / `-1 dBFS` deliverable via `render_movie_edit`.

Music lyric alignment and visual-renderer ownership remain documented in
[`MUSIC_LYRIC_ALIGNMENT_CONTINGENCY.md`](MUSIC_LYRIC_ALIGNMENT_CONTINGENCY.md) and
[`MUSIC_LYRIC_VISUALS.md`](MUSIC_LYRIC_VISUALS.md).
