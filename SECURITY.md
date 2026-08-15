# Security and offline boundary

Research queries, excerpts, model output, telemetry, and reports are private local data. Strict research has only two HTTP authorities: Bonsai at `127.0.0.1:8080` and Kiwix at `127.0.0.1:8085`.

The Control room has one separate producer-triggered network exception for public Hugging Face GGUF inspection and download. It accepts only `https://huggingface.co` repository/file URLs, rejects embedded credentials, bounds metadata, follows redirects only to approved Hugging Face/Xet CDN hosts, records publisher LFS SHA-256 when available, and never starts or resumes automatically. The process-wide work gate prevents this exception from overlapping research. A scoped Windows execution-state request prevents system sleep only while an approved transfer is active and is released on stop or completion.

Enforced controls:

- WebView CSP blocks external connections, scripts, images, fonts, and CDNs.
- Kiwix starts with external access blocked.
- Kiwix references reject non-loopback hosts, other ports, traversal, query strings, fragments, backslashes, controls, and excessive length.
- The research model has no shell, arbitrary path, browser, MCP, upload, delete, or remote provider tool.
- Search hits cannot be cited until opened; native Rust validates citation identity before publication.
- Model text cannot choose report paths, IDs, SQL, processes, or network targets.
- Managed model servers bind to loopback and use a random per-launch API key that is redacted from UI launch proof.
- One inference lease prevents duplicate Kestrel loads and simultaneous generations.
- Model downloads use recoverable ledgers and partial files; size, byte range, source identity, SHA-256, and GGUF metadata are checked before catalog activation.
- Reports are HTML-escaped, directory-atomically published, immutable by ID, and recoverable without SQLite.

Codex is a separate developer-only exception, never a research dependency. It requires an explicit confirmed action, a validated Kestrel Git root, installed/authenticated Codex CLI, an ephemeral workspace-write sandbox, and a fixed post-edit verification suite. It cannot run during strict research, uses no shell-composed command, and never commits. Native diagnostics remain available offline.

The default library avoids Documents because it may be cloud-synchronized. Kestrel does not provide encryption at rest; Windows account/file permissions remain the storage boundary. Standalone HTML has no remote assets or application command bridge.

Remaining assurance work includes firewall observation of a complete run, artifact hashing, fuzzing archive/model parsers, a signed installer, SBOM, and clean offline-VM validation. Never attach private reports, queries, paths, telemetry, or excerpts to a public issue without review.
