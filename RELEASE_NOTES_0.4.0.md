# Kestrel Local 0.4.0 — Autonomous Model Lab

This personal-lab release adds a Codex-subscription evaluation department to the transparent local-model runtime introduced in 0.3.

## Added

- Reuse of the existing ChatGPT/Codex sign-in through the stable `codex exec` interface. Kestrel runs ephemeral, read-only, strict-schema director and judge sessions and never reads the credential store.
- Fresh, nonce-bound synthetic suites across quantitative reasoning, structured data, evidence discipline, professional writing, multilingual work, coding, instruction adherence, long context, and agent/tool use.
- Optional current-frontier research for the director, with source URLs and explicit anti-copy/contamination instructions.
- Four host-generated deterministic probes in every fresh suite, plus versioned human-contributed cases.
- Immutable suite hashes and professional JSON, Markdown, and self-contained HTML reports.
- Blinded Codex rubric judging, local objective scorers, and separately preserved human score/note overlays.
- Cross-machine report export/import and a report library keyed by model, machine fingerprint, and suite hash.
- Full run evidence: raw visible output, separated reasoning, tool calls, errors, prompt/completion counts, latency, throughput, model signature, engine SHA-256, exact launch flags, hardware, telemetry, sampler, and seed policy.
- A template-aware thinking-model harness. Visible-answer tokens and reasoning tokens are budgeted separately with llama.cpp's per-request reasoning control, and the selected harness is recorded in the report.
- A durable live pipeline feed that survives navigation, plus immediate termination of the exact active Codex child when Stop is requested.
- Optional automatic cycle after a remembered model is restored, limited to one report per model per UTC day.

## Validation

- Native Windows run confirmed the newer Codex CLI, ChatGPT subscription authentication, live-research director, local Bonsai execution, blinded judge, and report writer in one cycle.
- A first diagnostic run correctly revealed that an unrestricted thinking channel could consume the entire answer budget. The repaired harness was verified against the active Bonsai/llama.cpp runtime before release packaging.
- `npm run check`, `npm run build`, `cargo test`, and `cargo clippy --all-targets -- -D warnings` pass.

## Still not a public beta

Code signing, signed updates, pinned engine provenance, SBOM/third-party notices, clean-VM installation, multi-vendor GPU and CPU-only coverage, accessibility/localization, exact-suite replay, statistical baseline matrices, and security fuzzing remain release gates.
