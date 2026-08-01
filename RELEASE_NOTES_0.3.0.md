# Kestrel Local 0.3.0 — Tomorrow Ready

This personal-preview release turns the 0.2 runtime into a setup that can remember a working local model environment across restarts and matching PCs.

## Added

- Cached zero-setup discovery for GGUF files in Jan, Hugging Face, LM Studio, Ollama, LocalAI, and common model folders.
- Automatic compatible `llama-server` detection, including Jan-managed backends and LocalAI installations.
- Path-independent model signatures and portable JSON profile export/import.
- “Make ready tomorrow,” automatic model restore, and optional Windows sign-in launch.
- Advanced mode with no Kestrel-imposed positive context/output cap.
- Configurable uncapped agent output budget.
- Live system/app/runtime RAM and GPU VRAM map, model/projector sizes, context, launch path, and redacted arguments.
- Ternary Bonsai 27B profile: strict all-GPU placement, automatic projector association, Q4 KV cache, flash attention, GPU KV/ops, no RAM prompt cache, and 32K context when global context is automatic.

## Validated on the development machine

- Windows 11, Ryzen 9 7950X, 64 GiB RAM, NVIDIA RTX 5070 12 GiB.
- PrismML `llama-server` loaded `Ternary-Bonsai-27B-Q2_0.gguf` plus its Q8 projector in about four seconds at an 8K acceptance-test context with strict GPU placement and no mixed offload.
- The model emitted a schema-valid `write_file` agent call, received the visible tool result, and completed the task at roughly 61 generated tokens/second.
- GPU used memory rose by about 7.1 GiB during the test and was reclaimed after shutdown.
- TypeScript check/build, Rust tests, and clippy with warnings denied pass.

The 8K test context protected the active development session from GPU-memory pressure; Kestrel's remembered Bonsai profile uses the previously validated 32K default. Public claims still require clean-VM, multi-vendor, accessibility, signing, update, security, and repeatable benchmark gates.
