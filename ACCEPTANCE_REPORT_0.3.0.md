# Kestrel Local 0.3.0 acceptance report

Date: 2026-07-17 (Europe/Helsinki)

## Outcome

The personal-preview build is installed and prepared for the next Windows sign-in on the development machine. It is not approved as a general public beta yet; the remaining gates are tracked in `ROADMAP.md`.

## Installed state

- Installed executable: `%LOCALAPPDATA%\Kestrel Local\kestrel-local.exe`, file version 0.3.0.
- Current-user desktop and Start-menu shortcuts target the 0.3.0 executable.
- The quoted executable path is registered in `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\KestrelLocal`.
- Ternary Bonsai 27B is pinned by content signature `bbf55546b7596786ea7e19a0`.
- Engine: `D:\LocalAI\Bonsai27B\runtime\llama-server.exe`.
- Placement: strict full GPU; automatic fitting and mixed offload disabled.
- Bonsai automatic context: 32,768 tokens. Advanced mode can request any positive context/output value; physical/model limits still apply.
- Agent maximum output: 32,768 tokens.
- Automatic restore: enabled.
- Personal Full Access unlock: enabled by explicit request.
- Shareable profile Full Access unlock: disabled for safe handoff.

## Agent acceptance

The installed PrismML engine loaded `Ternary-Bonsai-27B-Q2_0.gguf` and the Q8 multimodal projector in about four seconds with strict all-GPU placement at an 8K acceptance-test context. The model emitted a schema-valid `write_file` tool call, consumed the visible tool result, and completed the task. Generation measured about 61 tokens/second. GPU used-memory delta was about 7.1 GiB and was reclaimed after shutdown.

The acceptance test used 8K to avoid GPU pressure on the active development/Codex session. The remembered Kestrel profile selects the prior validated 32K Bonsai configuration.

## Automated checks

- `npm run check`: passed.
- `npm run build`: passed.
- `cargo test`: passed (1 test, 0 failures).
- `cargo clippy --all-targets -- -D warnings`: passed.
- Production Tauri compilation and both Windows bundles: passed.
- Installed executable, engine, model, projector, profile JSON, registry value, and shortcuts: verified.
- Final state: no Kestrel or `llama-server` process left running; GPU memory returned to baseline.

## Public-release blockers

Code signing, signed updates, pinned/bundled engine provenance, SBOM/third-party notices, clean-VM install/upgrade tests, crash recovery, persistent encrypted chat/audit history, immediate process-tree cancellation, parser/SSE fuzzing, accessibility/localization, and AMD/Intel/CPU-only test coverage remain before a responsible global beta.
