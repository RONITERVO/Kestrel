# Offline research roadmap

Kestrel Research is not described as finished or the best possible workflow. Improvements should preserve its small harness, offline boundary, source traceability, and open file formats.

## P0 — durability before long-term dependence

- [x] Bonsai-native OpenAI tool calling with a fixed two-tool prefix.
- [x] Real Kiwix search/read against the installed 102.3 GiB archive.
- [x] Evidence-only citations and archive cutoff disclosure.
- [x] Immutable edition lineage and related-report discovery.
- [x] SQLite FTS5 plus rebuildable JSONL and standalone report bundles.
- [x] Visible progress, elapsed time, activity, and bounded cancellation.
- [x] Self-contained accessible/printable HTML.
- [x] Live first-edition and expansion acceptance tests.
- [x] Opt-in single-context expedition with concurrent archive lanes and shared candidate memory.
- [x] Live VRAM/runtime visibility and uncapped advanced tuning with explicit restart.
- [ ] Add a one-click catalog integrity check and SQLite rebuild from report folders.
- [ ] Hash every report artifact in provenance and verify on open.
- [ ] Add crash recovery for a report interrupted between file publication and catalog insertion.
- [ ] Add Windows packet-capture evidence for zero non-loopback requests.
- [ ] Build/sign an installer and validate on a clean offline Windows VM.

## P1 — research usefulness

- [ ] Add local papers/books/user-document adapters behind the same two-tool contract.
- [ ] Add report collections and tags based on observed real usage.
- [ ] Show parent/child edition history and a readable “what changed” comparison.
- [ ] Add source-level notes, quote pinning, and user corrections without mutating model evidence.
- [ ] Add library backup/export and restore verification.
- [ ] Add per-report confidence/controversy visualization derived from explicit evidence, not model self-scoring alone.
- [ ] Add multilingual Kiwix book selection with explicit snapshot identity.

## P2 — broader local models

- [ ] Add small, explicit model adapters with tested prompt/tool contracts rather than a universal hidden heuristic.
- [ ] Measure quality, time-to-first-tool, total completion time, and publication success by model/profile.
- [ ] Persist anonymized local timing counters only when the user enables them.

## Acceptance targets

- Zero non-loopback requests in an offline research run.
- 100% of citations refer to sources opened in the same edition.
- A related rerun produces a child edition and leaves the parent byte-for-byte unchanged.
- Catalog rebuild produces the same report count and IDs from files alone.
- 10,000-report library search p95 below 200 ms on the supported baseline.
- Interrupted/cancelled research leaves no discoverable partial edition.
- Standalone HTML remains readable with JavaScript disabled and Kestrel uninstalled.
