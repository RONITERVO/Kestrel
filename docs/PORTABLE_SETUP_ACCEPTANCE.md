# Portable setup and storage acceptance

Status, 2026-10-21: the producer accepted the image model and requested this folder/storage follow-up
before merging PR 35. The installed components were reused, configured, and exercised through native
Kestrel operations. Bonsai was intentionally not reinstalled, as requested.

## Behavior fixed

- Setup remembers individual model source selections across restarts and profile export/import.
  A folder scan can be repeated for libraries spread across drives. Existing files are verified by
  the normal installer before use. Same-volume reuse uses hard links; cross-volume reuse copies
  verified files into the runtime's expected model directories. Sources are preserved.
- Movie installation honors the selected ComfyUI folder, as image, music, and speech already do.
  Movie and music renders use the current Setup folder. Previous take receipts remain immutable.
- Every media service checks the answering ComfyUI installation before submitting work. A server
  from a different or unverifiable folder produces a stop-and-retry instruction.
- Setup exposes independent MuScriptor executable/checkpoint paths, recognizes the existing local
  installation, and supplies defaults to music projects whose MIDI settings are empty.
- The long existing-model list paginates without overlapping its note. Save stays in the page footer.
- Fresh portable speech installation includes pinned `diffusers==0.38.0`; the readiness probe imports
  the decoder dependencies and the readiness receipt records the package set. A live test exposed
  the missing dependency in the previous installer.
- The music launcher handles an absent optional SageAttention package without terminating under
  Windows PowerShell's stop-on-error setting. Standard attention works without another package.

## Configured local installation

| Component | Location / result |
| --- | --- |
| Shared ComfyUI | `D:/AI/ComfyUI_windows_portable/ComfyUI`, v0.42.1 |
| H3, Music 3, MOSS, speech | Verified existing weights linked into that runtime |
| Strata | `C:/Strata`; existing model files remain on C: |
| Wikipedia | `E:/OfflineInternet/wikipedia_en_all_maxi_2024-01.zim` |
| Kiwix | Existing `D:/LocalAI/OfflineWikipedia/tools/kiwix-tools-3.8.1/kiwix-serve.exe` |
| MuScriptor | Existing `D:/AI/MuScriptor/.venv/Scripts/muscriptor.exe` and local large checkpoint |
| FFmpeg / FFprobe | Existing WinGet FFmpeg 7.1.1 installation on C: |
| Local model library | Registered Gemma, Qwen3.8-27B, and Qwen3.8-Flash-Next preserved |
| Included Bonsai | Absent intentionally; no download performed |

The real Setup snapshot reports eight of nine components ready. The remaining included-model card
is expected because Bonsai was deliberately deleted. Settings recovery copies and the exact setup
plan/snapshot are retained locally under `.artifacts/folder-acceptance/`.

## Live verification

These are opt-in developer acceptance tests, not background benchmarks or application features.
They use the same Rust operations as the desktop. GPU work ran serially on the producer's RTX 5070.

| Check | Outcome |
| --- | --- |
| Existing-file Setup installation | Real installer reused and verified weights; saved settings reloaded; eight components ready |
| Chatterbox → Whisper | Narration, word alignment, durable sidecar, and cached alignment passed; 68.8 s |
| H3 producer workflow | Created story, accepted it, saved/approved one five-second scene, rendered and exported MP4; 163.5 s |
| Music 3 cold start | Kestrel started its dedicated renderer and preserved a lossless FLAC plus graph/hash; 87.0 s |
| MuScriptor | Empty project MIDI settings inherited external paths; real transcription produced source MIDI, editable MIDI, document and receipt; audio unchanged; 33.2 s |
| Wikipedia | Actual archive search and article read passed after E: was connected |
| Existing castle image | A test copy rendered with the permanent runtime; original manifest and all earlier take records/PNG hashes unchanged; 54.7 s |
| Setup layout | Real React preview in headless Edge, 26 model rows, 1920×1080 / 1675×905 / 1366×768; 22 pages inspected with no row overlap or clipping |

The first music run exposed the optional-import launcher bug; after repair, a separate cold-start
run passed without manual assistance. The first speech run exposed the missing decoder dependency;
the repaired installation passed the round trip. These failures were not counted as successes.

Regression checks: 262 native tests, 209 core tests, 222 UI tests; Clippy with warnings denied,
architecture/generated-binding/TypeScript checks, UI build, release executable build and whitespace
checks. Headless layout uses preview data; it does not claim native file-dialog or physical-pointer
coverage. Image quality and bounding-box comparisons remain documented separately in
[MOSS image acceptance](MOSS_IMAGE_ACCEPTANCE.md).

## Storage audit and cleanup

The D: metadata scan covered accessible files of at least 100 MiB, excluding protected Windows
directories. AI folders were additionally checked for smaller retired weights. Registered model
paths, active graph dependencies, running processes and hard-link identities were checked before
cleanup. Identical large comparison weights were SHA-256 verified.

Removed at the producer's request:

- 14.12 GiB of redundant MOSS downloads.
- 32.20 GiB across the two retired Ideogram model sets.
- 1.23 GiB of unused Wan/ACE/Flux 1 decoders and retired Supertonic ONNX weights.
- 4.15 GiB of exclusively allocated files in the temporary comparison runtime. Its three MOSS
  weight links were removed only after verifying they referred to the permanent files.
- 0.31 GiB from the last duplicate Flux 2 decoder, replaced with a hash-verified hard link so both
  installations retain their existing file path.

Total removed file content: approximately **52.0 GiB**, excluding retained hard-linked weights.
Filesystem free-space changes also include allocation rounding and concurrent build activity.
The final disk check reported **129.2 GiB free on D:**.
The comparison runtime's input, output and workflow directories were moved into
`.artifacts/moss-acceptance/results/runtime-archive/`; the original comparison gallery and receipts
remain available. No user project, image, audio, video or receipt was deleted.

The older `D:/AI/ComfyUI/.venv` still occupies 4.82 GiB as a separate installation; its active model
weights share storage with the permanent runtime. Its runtime was retained because the cleanup
request explicitly authorized unused models, not uninstalling an existing external application.
MuScriptor's separate 2.81 GiB environment is required for its installed executable. The old
Supertonic environment is only 0.15 GiB; its model weights were removed. The scan also found a
SkyrimNet voice model owned by a different application, which was preserved.

Machine-specific audit inventories, exact deletion manifests and checksums remain under the ignored
`.artifacts/folder-acceptance/` directory. The desktop shortcut rebuilds and launches the current
branch. PR merging remains the producer's action.
