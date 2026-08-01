# Tomorrow-ready checklist

## This PC

- Install Kestrel Local 0.4.0.
- In Settings, select the detected PrismML/LocalAI CUDA engine.
- Keep **Full GPU** selected and mixed offload disabled.
- Enable **Advanced mode**, **Restore my ready models**, and **Start Kestrel when I sign in to Windows**.
- Select Ternary Bonsai 27B and choose **Make ready tomorrow**.
- Use **Full Access** only for tasks where current-user Windows authority is intended; the live feed is observability, not a security boundary.
- Keep **Autonomous lab after model restore** enabled if you want one fresh Codex-directed evidence report per model per UTC day. Kestrel reuses the existing ChatGPT subscription sign-in and never copies its credentials.

At the next Windows sign-in, Kestrel opens and resolves the pinned model by its content signature. The cached index makes the interface immediate while a background scan refreshes model locations.

## Identical second PC

1. Put the same model files on the machine through Jan, Hugging Face, LM Studio, Ollama, LocalAI, or a custom folder.
2. Install Kestrel 0.4.0.
3. Import the exported profile in Settings.
4. Review any hardware-difference warning, then save the policy.

Exact paths and usernames may differ. Identical weights still match. The profile does not transfer weights or install GPU drivers, and Kestrel cannot prepare a machine it cannot access; if an engine or model is absent, the UI names what is missing and offers detected engines/folders instead of silently changing placement.

Lab reports are separate from the setup profile. Use **Export latest HTML + JSON + Markdown** on the first machine and **Import** on the second to compare evidence without transferring model weights, prompts from private chats, or credentials.
