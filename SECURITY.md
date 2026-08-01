# Offline research security model

## Trust boundary

Kestrel Research handles queries, prior research, model outputs, source excerpts, and report files as private local data. Its only HTTP clients target two fixed loopback services:

- Bonsai: `http://127.0.0.1:8080/v1/chat/completions`
- Kiwix: `http://127.0.0.1:8085`

There is no configurable remote provider in this feature. The React WebView cannot contact either service directly; typed Tauri commands cross into Rust, and Rust owns requests, validation, storage, and progress events.

## Enforced now

- Tauri CSP: no external `connect-src`, remote scripts, remote images, CDN assets, or web fonts.
- Kiwix's installed start script uses `--blockexternal`.
- Wikipedia references are canonicalized only after validating a local relative path or `http://127.0.0.1:8085` / `localhost:8085` URL.
- HTTPS, other hosts, other ports, path traversal, backslashes, queries, fragments, control characters, and excessively long references are rejected.
- The model has two read-only logical tools. It has no shell, program execution, arbitrary file path, browser, MCP, upload, delete, or network tool.
- Search results cannot be cited. Evidence IDs are created only when `read_source` opens a result.
- Model-supplied citation IDs are intersected with the native evidence ledger before publication.
- Prior report identity, parent linkage, edition number, output paths, and IDs are native decisions, not model-controlled paths.
- All user/model text is HTML-escaped in standalone pages. Embedded JSON escapes closing script sequences.
- Report directories use native slugging plus a generated ID and are created only beneath Kestrel's research root.
- SQLite uses prepared statements and a bounded busy timeout.
- Cancellation stops at the active local request/tool boundary and never edits a published edition.

## Model/API assumptions

The exact installed telemetry proxy at port 8080 does not require an API key. This is acceptable only because it binds to loopback and is a user-managed local service. A future Kestrel-owned runtime should use a random per-launch key that is not exposed to the WebView.

Model output is untrusted. Strict schemas help shape it, but native code still validates citations, paths, source identity, sizes, and storage. Factual accuracy remains bounded by the model and Wikipedia snapshot.

## Storage and OS synchronization

The library defaults to `C:\Users\<you>\Kestrel Research`, not Documents, because Documents may be redirected into OneDrive. Kestrel does not configure encryption at rest. Anyone or any process with the current Windows user's file access can read the reports.

Opening standalone HTML launches the OS file association. The page contains no remote resources or active application commands, but it does embed the full report JSON for future local tooling.

## Remaining security work

- Observe a complete live research run under Windows packet capture/firewall logging and retain the evidence.
- Add catalog rebuild and integrity-check commands, including hashes for every immutable report file.
- Add optional local encryption for report bundles and clear UX around its effect on local-model access.
- Add configurable local endpoints only with the same loopback validation and explicit archive/model identity.
- Fuzz Kiwix HTML parsing, structured model responses, and report HTML generation.
- Add a signed installer, SBOM, third-party notices, and reproducible build provenance.

## Reporting

Do not attach private queries, report bundles, usernames, local paths, source excerpts, or model telemetry to a public issue without explicit review and redaction.
