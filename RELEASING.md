# Offline Windows releases

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
repository. Validate the resulting installer on a clean Windows VM with networking disabled before
publishing it.

An unsigned artifact is a development build, never a saleable producer release. Before publishing,
record a clean-machine acceptance run on hardware with at least 12 GiB NVIDIA VRAM:

1. Install the signed current-user NSIS package without Node.js, Rust, Git, Python, FFmpeg, ComfyUI,
   or WebView2 already present.
2. In Setup, choose a drive and complete **Set up essentials** and **Set up production suite** using
   only visible buttons. Interrupt and resume at least one large download.
3. Reboot, disconnect the public network, and verify chat, one local research report, an H3 image,
   an H3 clip, a Music 3 take, Chatterbox narration, Whisper dictation, and an FFmpeg export.
4. Confirm **Release AI memory** returns GPU usage to the desktop baseline and that no service binds
   outside loopback.
5. Preserve the installer manifest, SHA-256 list, acceptance log, and exact release commit together.

MuScriptor is not part of this commercial acceptance matrix. Its gated CC-BY-NC checkpoint remains
a producer-supplied extension and Setup must not silently download, bundle, or grant rights to it.
