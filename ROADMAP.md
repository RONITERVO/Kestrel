# Offline research roadmap

Kestrel should keep improving while preserving its small model-specific harness, offline boundary, source traceability, open formats, and single-runtime design.

## Durability

- [x] Directory-atomic immutable report publication.
- [x] Automatic SQLite/JSONL reconstruction from report files.
- [x] Recoverable research, control, and Bonsai settings writes.
- [x] One runtime owner and inference lease across chat/research.
- [x] Durable offline video plans with explicit memory profiles, bounded retries, verified clip hashes, and manual restart recovery.
- [x] Optional repository-scoped Codex repair with fixed verification.
- [ ] Hash and verify every immutable artifact.
- [ ] Add a visible library integrity/rebuild command.
- [ ] Add backup/export plus restore verification.
- [ ] Validate a signed installer on a clean offline Windows VM.

## Research usefulness

- [ ] Add local paper, book, and user-document adapters behind the evidence contract.
- [ ] Add collections/tags based on observed library use.
- [ ] Add parent/child history and a readable “what changed” comparison.
- [ ] Add source notes and user corrections without mutating model evidence.
- [ ] Add explicit confidence/controversy views derived from evidence.
- [ ] Add multilingual archive selection with snapshot identity.

## Acceptance targets

- Zero non-loopback requests during strict research.
- Every citation refers to a source opened in that edition.
- Related reruns create child editions and leave parents byte-for-byte unchanged.
- Catalog recovery produces the same report IDs from files alone.
- 10,000-report search p95 remains below 200 ms on the supported baseline.
- Interrupted/cancelled work leaves no visible partial edition.
- Video generation never starts until Kestrel can prove the selected offload policy, and interrupted batches never auto-resume.
- Standalone HTML remains readable with JavaScript disabled and Kestrel uninstalled.
