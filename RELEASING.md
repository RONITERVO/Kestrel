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
