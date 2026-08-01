# Kestrel Local 0.2.0 developer preview

Built and smoke-tested on Windows x64 on 17 July 2026.

## Added

- bounded multi-model residency with immediate switching among ready runtimes;
- least-recently-used eviction when the configured resident limit is reached;
- read-only, best-effort RAM warming with visible progress and a memory safety margin;
- explicit cold / RAM-warm / resident / active state labels;
- Workspace local-agent mode with constrained file tools;
- separately unlocked Full Access mode with typed program arguments, process inspection, and Explorer opening;
- live agent reasoning, tool argument, result, error, cancellation, and completion events;
- single-agent concurrency guard, bounded steps/output/timeouts, and cooperative Stop;
- settings for resident/warm limits, workspace roots, and Full Access unlock;
- competitor and Windows opportunity audit.

## Verification

- TypeScript and Vite production build: passed.
- Rust unit tests: passed.
- Rust Clippy with warnings denied: passed.
- Browser visual/DOM QA for chat, residency, agent, and settings screens: passed.
- Packaged executable launch smoke test: passed.
- NSIS and MSI packaging: passed.

## Artifacts

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| `Kestrel Local_0.2.0_x64-setup.exe` | 2,277,398 | `AB84A88FA021DE1AA764633CA50A9ED9DCF20B88B67169414BA6A080AD30D457` |
| `Kestrel Local_0.2.0_x64_en-US.msi` | 3,153,920 | `A2EC7B9A73F3618CCB4A8E94DDB971384BB3CC51555AAE66086855FA9EA2F77F` |

## Not a public release yet

These artifacts are unsigned developer previews and do not bundle a pinned `llama.cpp` engine. Full Access is observability, not sandboxing, and its Stop control does not yet kill a currently running program process tree immediately. See `ROADMAP.md` and `SECURITY.md` before distribution.
