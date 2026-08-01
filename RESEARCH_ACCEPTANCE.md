# Offline Research Acceptance — 1 August 2026

## Tested machine and assets

- Windows, AMD Ryzen 9 7950X, 64 GiB RAM, NVIDIA GeForce RTX 5070 12 GiB.
- `Ternary-Bonsai-27B-Q2_0.gguf`, alias `bonsai-27b`.
- PrismML `llama-server`, native context reported as 262,144; effective installed context 98,304.
- Kiwix 3.8.1 serving `wikipedia_en_all_maxi_2024-01.zim` with external access blocked.
- Archive date 12 January 2024, 6,863,660 articles, MAXI media variant.

## Automated results

| Layer | Check | Result |
| --- | --- | --- |
| Frontend | Reader and new-research behavior | 2 passed |
| Native unit | Storage, FTS, parent matching, HTML escaping, citations, Kiwix parsing/path safety | 8 passed |
| Build | TypeScript + Vite production assets | passed |
| Live Kiwix | Search and read the Antikythera mechanism from the real 102.3 GiB ZIM | passed, 0.11 s |
| Live pipeline | Empty-library research through real Bonsai/Kiwix to indexed HTML bundle | passed, 128.11 s |
| Live expansion | Seeded prior report → edition 2, correct parent, prior-research evidence, 2+ fresh Wikipedia sources, FTS, HTML | passed, 241.60 s |

## Visual acceptance

- Desktop reader inspected at 1265 × 711: library, report hierarchy, short answer, source rail, and evidence context were legible without clipping.
- New-research modal inspected with a filled question: focused/thorough choice, offline assurance, and enabled-state behavior were clear.
- Live progress drawer inspected during the real model run: stage, current activity, six-step sequence, spinner, and safe stop were visible.
- Responsive pass at 720 × 800: no horizontal page overflow; sidebar collapsed behind the menu; report hierarchy remained readable.
- Native Tauri first-use pass found and fixed an indefinite empty-library skeleton. The app now presents a clear first-research state.

## Failures found and fixed during live acceptance

1. Bonsai copied the Kiwix book ID as `2024-001` instead of `2024-01`. The original strict boundary rejected it. Kestrel now canonicalizes only the local book segment while continuing to reject remote hosts, non-8085 ports, traversal, queries, fragments, control characters, and backslashes. A regression test covers all cases.
2. A 4,300-token focused synthesis ended in valid but truncated JSON because reasoning and visible structured output shared the allowance. Field bounds, larger evidence-appropriate allowances, and one compact no-thinking retry now make publication reliable. The following live run passed.
3. Windows Documents resolved into OneDrive on the tested machine. The default research root moved to the unsynced local home directory to preserve the offline/privacy expectation.
4. An empty catalog displayed a permanent loading skeleton. It now displays an actionable first-use explanation.

## Offline-readiness boundary

No packet-capture assertion was performed in this pass. The implementation-level controls are fixed loopback URLs, a WebView CSP without external `connect-src`, Kiwix `--blockexternal`, no remote dependencies at runtime, and no remote tool/provider code path. A future release acceptance should add a Windows Firewall or packet-capture observation while running the ignored live test.

The experience is ready for sustained local use on the tested setup, with the improvement opportunities listed in README retained explicitly. It is not described as the last or best possible research workflow.
