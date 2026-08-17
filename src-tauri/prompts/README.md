# Kestrel prompt packs

`default.json` is the single source of truth for Kestrel-authored natural-language instructions sent to local models. It is deliberately a prompt-only, versioned document: it contains no Rust, TypeScript, paths, credentials, model weights, projects, conversations, or producer content.

The native `prompt_catalog` module owns stable prompt IDs, completeness and size validation, placeholder rendering, recoverable persistence, and the embedded build defaults. The System → Prompt pack editor lets advanced users validate, activate, reset, import, and export the same format without opening the source tree. A custom pack is stored as `prompt-pack.json` in the Kestrel Research library; exported packs go to its `prompt-packs` folder.

This boundary covers app-authored system messages, orchestration and retry instructions, collaborator instructions, reviewer/lint wording, fixed model suffixes, and qualification prompts. Producer-authored fields, durable chat history, generated tool results, media metadata, structured schemas, and dynamically compiled image/music payloads remain typed runtime data. Those values are visible in their owning workspaces and receipts; duplicating them into a global prompt file would create stale or unsafe second copies.

Changing a prompt never changes native authority. Tool availability, filesystem policy, loopback-only research, citation checks, schemas, render graphs, and project validation remain enforced in Rust.
