# Competitive audit and release direction

Audited 17 July 2026 from the installed Jan 0.8.3 application, current public Jan and `llama.cpp` documentation, and the public PrismML Bonsai demo repository. A separately installed product named "Bonsai Control Center" was not found on this machine or in the public PrismML repository, so Bonsai-specific claims below are intentionally narrow.

## What Kestrel 0.2 uniquely makes obvious

| User question | Kestrel 0.2 answer |
| --- | --- |
| Where is my model running? | A persistent CPU/GPU placement policy. Mixed placement is rejected unless explicitly enabled. |
| Can it silently fall back? | No. `--fit off` is enforced in strict modes and a failed fit is reported as a failure. |
| Can I switch immediately? | Yes, among wholly resident runtimes up to the configured limit. The active runtime is identified separately from loaded runtimes. |
| Is “warm in RAM” the same as loaded on GPU? | No. The UI explicitly says it saves disk reads but not the RAM-to-VRAM copy. |
| What exactly did the model do as an agent? | Start, reasoning, tool call, result, completion, error, and cancellation events remain visible in one feed. |
| Does “Full Access” really mean full access? | Yes. It is disabled by default, presented in red, and described as current-user authority rather than a safety sandbox. |
| How was the engine launched? | Runtime state and raw process output are visible; the next release must add a polished copyable full-argument/provenance panel. |

## Where Jan currently leads

The installed Jan build is a mature general local-AI workstation. It already has a model hub/downloader, multiple cloud providers, assistants, attachments, projects/history, MCP servers, a local API server, proxy/privacy settings, shortcuts, backend installation and updates, engine environment variables, system/GPU monitoring, and a deep `llama.cpp` settings surface. Its current runtime settings include concurrent loaded models, fit targets, thread counts, context shift, prediction limit, batch and micro-batch sizes, GPU split/main GPU, flash attention, parallel sequences, continuous batching, memory mapping, and memory locking.

Kestrel must not copy that settings density into the primary screen. Its opportunity is to make the safe/recommended path understandable first, then place every expert control in an attributable advanced profile with a plain-language cost.

## What native llama.cpp exposes that Kestrel should surface

Current `llama-server` supports a router with model presets and a maximum loaded-model count, explicit loaded/loading/unloaded/sleeping states, idle sleep, prompt caching, slot inspection/control, embeddings, reranking, grammars, speculative decoding, multimodal inputs, and OpenAI-compatible endpoints. Jan's 0.8 release also builds its runtime management around that router.

Kestrel 0.2 uses independent managed servers for isolation and transparent residency. Before beta, benchmark this against the native router for switch latency, memory overhead, crash isolation, and per-model argument fidelity. Adopt the router only if it preserves the no-silent-offload invariant and remains inspectable.

## Windows capabilities still missing

- Job Objects for immediate model/agent process-tree termination and cleanup after crashes.
- DXGI/vendor telemetry for trustworthy adapter selection, dedicated/shared VRAM, and preflight capacity.
- ETW/performance counters for time-to-first-token, memory, disk, CPU, and GPU timelines.
- start-at-login, tray residency, notifications, and measured wake-to-ready reporting.
- Windows Graphics Capture and UI Automation for visible, permissioned GUI work.
- AppContainer/Windows Sandbox profiles between Workspace and Full Access.
- Credential Manager for optional cloud-help keys and signed secret handoff.
- signed MSIX/NSIS/MSI, SmartScreen reputation, signed updates, SBOM, and clean-VM validation.

## Product order

1. Make “Use recommended” correct: import signed/reproducible model release metadata and preserve embedded templates.
2. Make “Ready in five seconds” measured: startup tray, persisted resident profile, readiness histogram, and cold/RAM-warm/device-resident labels.
3. Make Full Access stoppable: Job Objects, confirmation policies, immutable audit, egress profiles, and recovery.
4. Add useful-work harnesses: coding workspace, document/attachment pipeline, MCP, browser/UI automation, and local OpenAI/Anthropic endpoints.
5. Add the experiment workbench: seeds, repetitions, warmups, blinded comparisons, hashes, full environment manifests, JSON/CSV export, and replay.
6. Only then match mature ecosystem conveniences: model download/catalog, split GGUF, backend manager, chats/projects, multimodal, localization, and accessibility.

## Honest verdict

Kestrel 0.2 is already more explicit than the inspected alternatives about placement, silent fallback, residency versus RAM warmth, and what Full Access means. It is not yet a better total product than Jan, and the public Bonsai evidence is insufficient for a broad superiority claim. The path to being best for this use case is not maximum toggles; it is measurable readiness, official-profile fidelity, transparent experiments, and a powerful agent that can be stopped and audited.

## Primary references

- llama.cpp server documentation: <https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md>
- Jan documentation: <https://jan.ai/docs>
- Jan desktop quickstart: <https://www.jan.ai/docs/desktop/quickstart>
- Jan 0.8 router release: <https://www.jan.ai/changelog/2026-05-22-jan-v0.8.0>
- PrismML Bonsai public demo: <https://github.com/PrismML-Eng/Bonsai-demo>
