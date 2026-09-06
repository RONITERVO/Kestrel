# Offline Windows releases

Source releases on GitHub may be published with the verified checks and any remaining acceptance
limits stated in their release notes. They do not claim a signed Windows installer or a clean-machine
acceptance run. The signed commercial installer process below is a separate distribution gate.

Run deterministic verification first, then create a current-user NSIS installer containing the
full WebView2 offline installer:

```powershell
npm run package:offline
```

The command creates a timestamped `release/<version>-<timestamp>/` directory containing a portable
executable, offline installer, JSON manifest, Authenticode status, sizes, and SHA-256 checksums.
No startup entry is created and the installer blocks version downgrades.

For a public build, use an installed code-signing certificate and require validation:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/package-offline.ps1 `
  -SigningCertificateThumbprint <thumbprint> `
  -TimestampUrl <RFC-3161-url> -RequireSignature
```

The script fails if signing or signature verification fails. Keep certificates outside the
repository. Validate the resulting installer on a clean Windows VM, using networking only for the
documented setup downloads before the offline acceptance checks, before publishing it.

An unsigned artifact is a development build, never a saleable producer release. Before publishing,
record a clean-machine acceptance run on hardware with at least 12 GiB NVIDIA VRAM:

1. Install the signed current-user NSIS package without Node.js, Rust, Git, Python, FFmpeg, ComfyUI,
   or WebView2 already present.
2. With networking available, choose a drive in Setup and complete **Set up essentials** and **Set up
   production suite** using only visible buttons. Interrupt and resume at least one large download.
3. Disable the public network, reboot, and then verify chat, one local research report, an H3 image,
   an H3 clip, a Music 3 take, Chatterbox narration, Whisper dictation, and an FFmpeg export offline.
4. Confirm **Release AI memory** returns GPU usage to the desktop baseline and that no service binds
   outside loopback.
5. Preserve the installer manifest, SHA-256 list, acceptance log, and exact release commit together.

MuScriptor is not part of this commercial acceptance matrix. Its gated CC-BY-NC checkpoint remains
a producer-supplied extension and Setup must not silently download, bundle, or grant rights to it.
Ideogram 4 is also excluded from the commercial acceptance matrix because its published model
agreement is non-commercial. For an internal non-commercial acceptance run, install it separately
through its explicit Setup acknowledgement, disconnect the public network, reboot, generate one
full-resolution Image Studio PNG, and preserve the take receipt with the acceptance record. Do not
bundle the weights or represent Kestrel's MIT license as granting Ideogram model or output rights.

## Producer Studio acceptance

Before the public release, verify a short idea → complete story → accepted story → distinct scene
queue → producer references → H3 renders → export. New movies must select the faster 768 × 448
preset. Verify that frame choices and native references explain their incompatibility without
clearing saved selections, and that video audio cannot be left enabled without video motion.

Stop a queue after at least one completed scene, reopen, and explicitly resume. Confirm saved scenes
are not repeated. Test a multi-hour requested count, paginated scene editing, and an export spanning
multiple FFmpeg groups. Preserve custom timeline order, repeated items, trims, and the export title
through scene saves. A model response stopped by its output limit must remain partial text.

The deterministic suite covers queue checkpoint/restart and reference boundaries. Run the explicit
local media acceptance test as well:

```powershell
cargo test --manifest-path src-tauri\Cargo.toml live_long_timeline_export_joins_groups_and_preserves_sources -- --ignored --nocapture
```

After packaging checks, evaluate the selected local model through the actual story/scene jobs:

```powershell
$env:KESTREL_ACCEPTANCE_LIBRARY = 'D:\Kestrel acceptance\producer'
$env:KESTREL_LIVE_MODEL_ID = '<installed catalog model ID>'
cargo test --manifest-path src-tauri\Cargo.toml live_local_producer_story_and_distinct_scene_queue -- --ignored --nocapture
```

This explicit test reads the existing model catalog and engine settings, uses `RuntimeManager`,
and saves a real story and three distinct scene cards in a separate acceptance library. It has no
public-network fallback. Inspect the actual prose and H3 results; passing structured validation
alone does not establish creative quality or a clean-machine release.

Use the printed acceptance movie ID to render those saved scenes and export the cut. Keep the
separate acceptance library selected; this command never chooses a user production implicitly.

```powershell
$env:KESTREL_ACCEPTANCE_MOVIE_ID = '<acceptance movie ID printed above>'
$env:KESTREL_LIVE_COMFY_ROOT = '<installed ComfyUI folder>'
cargo test --manifest-path src-tauri\Cargo.toml live_producer_h3_render_and_export -- --ignored --nocapture
```
