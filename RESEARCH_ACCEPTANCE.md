# Kestrel Local 0.7 acceptance — 1 August 2026

## Machine and local assets

- Windows, AMD Ryzen 9 7950X, 64 GiB RAM, NVIDIA GeForce RTX 5070 12 GiB.
- `Ternary-Bonsai-27B-Q2_0.gguf`, server alias `bonsai-27b`.
- Installed Bonsai context 98,304; maximum response allowance 32,768; Q4 KV.
- Kiwix 3.8.1 serving `wikipedia_en_all_maxi_2024-01.zim` with external access blocked.
- Archive cutoff 12 January 2024; 6,863,660 articles.

## Automated and live results

| Layer | Result |
| --- | --- |
| Rust unit/integration | 22 passed; 5 live tests ignored by default |
| Frontend behavior | 4 passed |
| Rust lint | `clippy --all-targets -D warnings` passed |
| TypeScript + production web build | passed |
| Git diff hygiene | passed |
| Existing Bonsai attachment | passed; attached mode, no child PID, 0.01 s |
| Installed PowerShell/BOM settings apply | passed; recovery backup preserved and Bonsai health `ok` |
| Real Kiwix search/read | passed, 0.15 s |
| Real focused expansion | citation-valid indexed standalone HTML, 108.80 s |
| Real Solo expedition | high-capacity shared-lane report, 316.36 s |

The focused live acceptance seeds prior research and requires the result to become a linked child edition, inspect fresh local Wikipedia evidence, reject unknown citations, enter FTS, and produce standalone HTML. The expedition acceptance records the 98,304 context / 32,768 output profile and verifies multi-lane shared memory without extra model processes.

## Storage recovery acceptance

- Publication is built in a hidden staging directory and atomically renamed into its final report folder.
- Saving a duplicate report ID fails and leaves the original HTML byte-for-byte unchanged.
- Deleting SQLite/WAL/SHM and reopening the store reconstructs the report and FTS result from report files.
- Missing derived HTML, evidence, or provenance files are regenerated from the immutable report JSON.
- Interrupted settings replacement retains a recovery copy.

## Visual and navigation acceptance

The Vite preview was inspected directly in the in-app browser:

- Research reader at desktop width: library, report hierarchy, source rail, short answer, evidence, provenance, and standalone action remained legible.
- Historical dark control plane: model drawer, attached runtime, local chat, single-lease statement, exact model identity, context, and live VRAM were visible without displacing research.
- Developer workspace: Codex/auth/Git/offline status, project boundary, fixed diagnostics, explicit repair confirmation, and reviewable output were clear.
- Responsive control layout at 720 × 800 and 500 × 800: no horizontal overflow; header becomes icon navigation; model library and chat remain reachable.
- Browser console reported no warning or error during navigation.

## Boundary retained

No packet-capture assertion was performed in this pass. Implementation controls remain fixed loopback research URLs, a WebView CSP without external `connect-src`, Kiwix external blocking, no remote research provider, and no developer/Codex execution while strict research is active.

Kestrel is ready for sustained offline use on this tested installation. Artifact hashing, a visible integrity action, signed installer validation, offline-VM validation, and additional local corpora remain worthwhile improvements.
