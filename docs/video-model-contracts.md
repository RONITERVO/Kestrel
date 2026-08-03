# Offline video model contracts

Last audited against upstream primary sources: 2026-08-03.

This file is the maintenance ledger for Kestrel's built-in ComfyUI video presets. Runtime behavior must not silently follow changing online defaults. A newly planned project snapshots the contract version, preset ID, prompt style, prompt length limit, sampler, scheduler, and model shift into `project.json`. That snapshot is accepted only when its preset ID matches the project preset. Older projects without a valid snapshot use the built-in compatibility defaults.

## Prompt contract

Every final positive prompt is natural English descriptive prose for one continuous shot. It must identify the subject and visible details, environment and spatial relationships, visual style, shot scale, lighting and color, one readable action using direct verbs, and one coherent camera movement. It must not contain storyboard labels, generator commands, cuts, dialogue, captions, editing instructions, audience metadata, or negative-prompt terms.

The local planner writes a 68-82 word chapter seed. Native Rust adds a bounded action phase and camera move, producing an 80-100 word final clip prompt. It also repairs short planner output deterministically and caps the model input at 1,800 characters. The continuity bible remains planning data; it is not appended wholesale to every text-encoder call. Each clip prompt instead restates the essential visual identity and scene. A blank negative-prompt override selects the audited preset baseline; user text replaces that baseline completely.

## Audited inference profiles

| Preset | Native output | Sampling | Prompt style | Memory policy |
| --- | --- | --- | --- | --- |
| Wan 2.1 T2V 1.3B GPU only | 832x480 landscape, 33 frames at 16 fps, 2 seconds | 50 steps, CFG 6, UniPC/simple, shift 8 | `wan-expanded-english-v1` | Offload forbidden |
| Wan VACE 1.3B Reference Studio | 832x480 landscape, 81 frames at 16 fps, 5 seconds | 50 steps, CFG 5, UniPC/simple, shift 16 | `wan-vace-descriptive-english-v1` | Predictable stage-boundary movement |
| Kandinsky 5 Lite Distilled | 768x512 landscape, 121 frames at 24 fps, 5 seconds | 16 steps, CFG 1, Euler ancestral/beta, shift 5 | `kandinsky-expanded-english-v1` | Predictable stage-boundary movement |
| Kandinsky 5 Lite SFT | 768x512 landscape, 121 frames at 24 fps, 5 seconds | 100 steps, CFG 5, Euler ancestral/beta, shift 5 | `kandinsky-expanded-english-v1` | Predictable stage-boundary movement |
| Wan 2.2 TI2V 5B | 1280x704 landscape, 121 frames at 24 fps, 5 seconds | 20 steps, CFG 5, UniPC/simple, shift 8 | `wan-expanded-english-v1` | Forced low-VRAM asynchronous offload |

Portrait swaps the native landscape dimensions. Square is 624x624 for Wan 1.3B and Kandinsky, 640x640 for VACE, and 960x960 for Wan 2.2.

## Deliberate local choices

- Wan 2.1 T2V 1.3B keeps the official quality-oriented 50-step, CFG 6, shift 8 profile but uses 33 frames instead of the common 81-frame example. This is the explicit two-second GPU-only speed preset; 33 still satisfies Wan's `4n+1` frame rule.
- Wan VACE follows the upstream inference defaults: 50 steps, CFG 5, shift 16, 81 frames.
- Kandinsky Distilled follows the official 16-step, CFG 1 profile. SFT uses the upstream repository's 100-NFE quality target rather than the faster 50-step ComfyUI example because this preset is explicitly quality-first.
- Wan 2.2 follows the official ComfyUI native template used by this backend: 20 steps, CFG 5, shift 8, 121 frames, and 1280x704. The original command-line repository commonly defaults to 50 steps and shift 5; mixing those defaults into the ComfyUI graph would make Kestrel's reviewed timing contract inaccurate.

## Primary-source ledger

- Wan 2.1 model guidance and examples: <https://github.com/Wan-Video/Wan2.1/blob/main/README.md>
- Wan 2.1 inference defaults: <https://github.com/Wan-Video/Wan2.1/blob/main/generate.py>
- Wan 2.1 local prompt-expander contract: <https://github.com/Wan-Video/Wan2.1/blob/main/wan/utils/prompt_extend.py>
- VACE prompt and inference guidance: <https://github.com/ali-vilab/VACE/blob/main/UserGuide.md>
- Kandinsky 5 upstream implementation and examples: <https://github.com/kandinskylab/kandinsky-5>
- Kandinsky 5 ComfyUI guidance: <https://github.com/kandinskylab/kandinsky-5/tree/main/comfyui>
- Kandinsky 5 Diffusers parameters: <https://huggingface.co/docs/diffusers/api/pipelines/kandinsky5_video>
- Wan 2.2 TI2V 5B model card: <https://huggingface.co/Wan-AI/Wan2.2-TI2V-5B>
- ComfyUI Wan 2.2 guide: <https://docs.comfy.org/tutorials/video/wan/wan2_2>
- ComfyUI Wan 2.2 5B workflow JSON: <https://raw.githubusercontent.com/Comfy-Org/workflow_templates/refs/heads/main/templates/video_wan2_2_5B_ti2v.json>

## Future maintenance

Do not change a live contract merely because upstream changes a default. Audit the exact checkpoint and ComfyUI node graph, add a new contract version, update the source ledger and regression tests, and decide explicitly whether existing projects retain their snapshot or migrate. New prompt styles should receive a new stable style ID. Offline generation must remain deterministic from the durable project record without requiring any source above to be reachable.
