//! Stock ComfyUI (`v0.42.1+`) MiniMax H3 workflow builder.
//!
//! Compiles typed Kestrel H3 window plans into pure stock ComfyUI node graphs without any custom
//! third-party workflow nodes (except Kestrel's optional bundled `ModelPreviewOverrideKJ` live
//! preview node `"90"` when installed).

use super::live_preview::{preview_node, PREVIEW_NODE_ID};
use kestrel_app_core::{H3LoraSpec, H3TaskMode, H3WindowCompileResult};
use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub struct H3StagedGraphRef {
    /// Relative path inside `<comfy_root>/input/` (forward slashes).
    pub relative_file: String,
    pub use_embedded_audio: bool,
}

#[derive(Debug, Clone)]
pub struct H3StagedGraphGuide {
    /// `"image"`, `"video"`, or `"audio"`.
    pub kind: String,
    pub relative_file: String,
    pub frame_idx: i32,
    pub use_image: bool,
    pub use_audio: bool,
}

#[derive(Debug, Clone)]
pub struct H3InstalledModelNames {
    pub unet_ref2va: String,
    pub unet_fl2va: String,
    pub clip_qwen3vl: String,
    pub video_vae: String,
    pub audio_vae: String,
}

impl Default for H3InstalledModelNames {
    fn default() -> Self {
        Self {
            unet_ref2va: "minimax_h3_ref2va_pruned_int8_convrot.safetensors".into(),
            unet_fl2va: "minimax_h3_fl2va_pruned_int8_convrot.safetensors".into(),
            clip_qwen3vl: "qwen3vl_32b_minimax_h3_nvfp4_awq.safetensors".into(),
            video_vae: "minimax_h3_video_vae_fp16.safetensors".into(),
            audio_vae: "minimax_h3_audio_vae_fp32.safetensors".into(),
        }
    }
}

pub struct H3ComfyGraphBuild<'a> {
    pub compile: &'a H3WindowCompileResult,
    pub models: &'a H3InstalledModelNames,
    pub task_mode: H3TaskMode,
    pub sampler_name: &'a str,
    pub scheduler: &'a str,
    pub cfg: f32,
    pub shift_video: f32,
    pub shift_audio: f32,
    pub seed: u64,
    pub ref_image_size: &'a str,
    pub output_prefix: &'a str,
    pub picture_refs: &'a [H3StagedGraphRef],
    pub video_refs: &'a [H3StagedGraphRef],
    pub standalone_audio_refs: &'a [H3StagedGraphRef],
    pub guides: &'a [H3StagedGraphGuide],
    /// Relative path in `<comfy_root>/input/` for window source video (`[render_start..render_end)` + padding).
    pub source_video_file: Option<&'a str>,
    /// Relative path in `<comfy_root>/input/` for window source stereo 48kHz WAV audio.
    pub source_audio_file: Option<&'a str>,
    /// Relative path in `<comfy_root>/input/` for `[width x height]` spatial mask PNG (`white = generate`).
    pub spatial_mask_file: Option<&'a str>,
    pub preview_available: bool,
}

struct NodeCounter(u32);

impl NodeCounter {
    fn next(&mut self) -> String {
        let id = self.0.to_string();
        self.0 += 1;
        id
    }
}

/// Builds a complete stock ComfyUI prompt graph for MiniMax H3.
pub fn build_h3_comfy_graph(req: H3ComfyGraphBuild<'_>) -> Value {
    let compile = req.compile;
    let width = compile.resolved_width;
    let height = compile.resolved_height;
    let length = compile.geometry.padded_frames;
    let use_tiled_vae = width >= 512 && height >= 512;

    let uses_ref_conditioning = compile.resolved_model.starts_with("ref2va_cond+")
        || compile.resolved_model.contains("ref2va")
        || !req.picture_refs.is_empty()
        || !req.video_refs.is_empty()
        || !req.standalone_audio_refs.is_empty();

    let unet_name = if compile.resolved_model.contains("fl2va") {
        &req.models.unet_fl2va
    } else {
        &req.models.unet_ref2va
    };

    let ref_image_size = if req.ref_image_size == "match" {
        "match"
    } else {
        "max"
    };

    let mut graph = json!({
        "1": {
            "class_type": "UNETLoader",
            "inputs": {
                "unet_name": unet_name,
                "weight_dtype": "default"
            }
        },
        "2": {
            "class_type": "CLIPLoader",
            "inputs": {
                "clip_name": req.models.clip_qwen3vl,
                "type": "minimax",
                "device": "default"
            }
        },
        "3": {
            "class_type": "VAELoader",
            "inputs": {
                "vae_name": req.models.video_vae
            }
        },
        "4": {
            "class_type": "VAELoader",
            "inputs": {
                "vae_name": req.models.audio_vae
            }
        }
    });

    // Conditioning node ("5")
    if uses_ref_conditioning {
        graph["5"] = json!({
            "class_type": "MiniMaxH3ReferenceToVideo",
            "inputs": {
                "clip": ["2", 0],
                "vae": ["3", 0],
                "audio_vae": ["4", 0],
                "prompt": compile.compiled_prompt,
                "width": width,
                "height": height,
                "length": length,
                "ref_image_size": ref_image_size
            }
        });
    } else {
        graph["5"] = json!({
            "class_type": "MiniMaxH3ImageToVideo",
            "inputs": {
                "clip": ["2", 0],
                "vae": ["3", 0],
                "prompt": compile.compiled_prompt,
                "width": width,
                "height": height,
                "length": length
            }
        });
    }

    let mut ids = NodeCounter(100);

    // 1. Model chain: UNETLoader ("1") -> optional LoraLoaderModelOnly -> MiniMaxH3SigmaShift -> optional Preview ("90")
    let mut model_node = "1".to_string();
    for lora in &compile.resolved_loras {
        if !lora.enabled || lora.file_name.trim().is_empty() {
            continue;
        }
        let lora_id = ids.next();
        graph[&lora_id] = build_lora_node(&model_node, lora);
        model_node = lora_id;
    }

    let sigma_id = ids.next();
    graph[&sigma_id] = json!({
        "class_type": "MiniMaxH3SigmaShift",
        "inputs": {
            "model": [model_node, 0],
            "shift_video": req.shift_video.clamp(0.1, 100.0),
            "shift_audio": req.shift_audio.clamp(0.1, 100.0)
        }
    });
    model_node = sigma_id;

    if req.preview_available && width >= 256 && height >= 256 {
        graph[PREVIEW_NODE_ID] = preview_node(&model_node, 12);
        model_node = PREVIEW_NODE_ID.to_string();
    }

    // 2. Bind staged reference media onto node "5" (`MiniMaxH3ReferenceToVideo`)
    if uses_ref_conditioning {
        for (idx, pic) in req.picture_refs.iter().enumerate() {
            let load_id = ids.next();
            graph[&load_id] = json!({
                "class_type": "LoadImage",
                "inputs": { "image": pic.relative_file }
            });
            graph["5"]["inputs"][format!("ref_images.ref_image_{idx}")] = json!([load_id, 0]);
        }
        for (idx, vid) in req.video_refs.iter().enumerate() {
            let load_id = ids.next();
            let comp_id = ids.next();
            graph[&load_id] = json!({
                "class_type": "LoadVideo",
                "inputs": { "file": vid.relative_file }
            });
            graph[&comp_id] = json!({
                "class_type": "GetVideoComponents",
                "inputs": { "video": [load_id, 0] }
            });
            graph["5"]["inputs"][format!("ref_videos.ref_video_{idx}")] = json!([comp_id, 0]);
            if vid.use_embedded_audio {
                graph["5"]["inputs"][format!("ref_video_audios.ref_video_audio_{idx}")] =
                    json!([comp_id, 1]);
            }
        }
        for (idx, aud) in req.standalone_audio_refs.iter().enumerate() {
            let load_id = ids.next();
            graph[&load_id] = json!({
                "class_type": "LoadAudio",
                "inputs": { "audio": aud.relative_file }
            });
            graph["5"]["inputs"][format!("ref_audios.ref_audio_{idx}")] = json!([load_id, 0]);
        }
    }

    // 3. Build joint AV latent (`["5", 1]` by default, or encoded source video/audio + noise masks)
    let mut latent_ref: Value = json!(["5", 1]);
    if compile.uses_source_av_latent
        && (req.source_video_file.is_some() || req.source_audio_file.is_some())
    {
        let encoded_video = req.source_video_file.map(|video_file| {
            let load_vid = ids.next();
            let comp_vid = ids.next();
            let enc_vid = ids.next();
            graph[&load_vid] = json!({
                "class_type": "LoadVideo",
                "inputs": { "file": video_file }
            });
            graph[&comp_vid] = json!({
                "class_type": "GetVideoComponents",
                "inputs": { "video": [load_vid, 0] }
            });
            if use_tiled_vae {
                graph[&enc_vid] = json!({
                    "class_type": "VAEEncodeTiled",
                    "inputs": {
                        "pixels": [comp_vid, 0],
                        "vae": ["3", 0],
                        "tile_size": 1024,
                        "overlap": 256,
                        "temporal_size": 248,
                        "temporal_overlap": 32
                    }
                });
            } else {
                graph[&enc_vid] = json!({
                    "class_type": "VAEEncode",
                    "inputs": {
                        "pixels": [comp_vid, 0],
                        "vae": ["3", 0]
                    }
                });
            }

            if compile.uses_video_noise_mask {
                let mask_id = build_video_latent_mask(
                    &mut graph,
                    &mut ids,
                    width,
                    height,
                    compile.geometry.video_latent_tokens,
                    if req.task_mode == H3TaskMode::AudioOverExistingVideo {
                        0
                    } else {
                        compile.geometry.video_mask_token_lo
                    },
                    if req.task_mode == H3TaskMode::AudioOverExistingVideo {
                        0
                    } else {
                        compile.geometry.video_mask_token_hi
                    },
                    req.spatial_mask_file,
                );
                let set_mask = ids.next();
                graph[&set_mask] = json!({
                    "class_type": "SetLatentNoiseMask",
                    "inputs": {
                        "samples": [enc_vid, 0],
                        "mask": [mask_id, 0]
                    }
                });
                set_mask
            } else {
                enc_vid
            }
        });

        let encoded_audio = req.source_audio_file.map(|audio_file| {
            let load_aud = ids.next();
            let enc_aud = ids.next();
            graph[&load_aud] = json!({
                "class_type": "LoadAudio",
                "inputs": { "audio": audio_file }
            });
            graph[&enc_aud] = json!({
                "class_type": "VAEEncodeAudio",
                "inputs": {
                    "audio": [load_aud, 0],
                    "vae": ["4", 0]
                }
            });

            if compile.uses_audio_noise_mask {
                let freeze_all_audio = req.task_mode == H3TaskMode::KeepExistingAudio
                    || (matches!(
                        req.task_mode,
                        H3TaskMode::CharacterSwap | H3TaskMode::HeadSwap
                    ) && compile.ordinals.audios.is_empty());
                let mask_id = build_audio_latent_mask(
                    &mut graph,
                    &mut ids,
                    compile.geometry.audio_latent_tokens,
                    if freeze_all_audio {
                        0
                    } else {
                        compile.geometry.audio_mask_token_lo
                    },
                    if freeze_all_audio {
                        0
                    } else {
                        compile.geometry.audio_mask_token_hi
                    },
                );
                let set_mask = ids.next();
                graph[&set_mask] = json!({
                    "class_type": "SetLatentNoiseMask",
                    "inputs": {
                        "samples": [enc_aud, 0],
                        "mask": [mask_id, 0]
                    }
                });
                set_mask
            } else {
                enc_aud
            }
        });

        match (encoded_video, encoded_audio) {
            (Some(v_id), Some(a_id)) => {
                // Fit VAEEncodeAudio length to ["5", 1] via LTXVConcatAVLatent + LTXVSeparateAVLatent,
                // then combine with masked source video latent.
                let fit_av = ids.next();
                let sep_av = ids.next();
                let concat_av = ids.next();
                graph[&fit_av] = json!({
                    "class_type": "LTXVConcatAVLatent",
                    "inputs": {
                        "video_latent": ["5", 1],
                        "audio_latent": [a_id, 0]
                    }
                });
                graph[&sep_av] = json!({
                    "class_type": "LTXVSeparateAVLatent",
                    "inputs": { "av_latent": [fit_av, 0] }
                });
                graph[&concat_av] = json!({
                    "class_type": "LTXVConcatAVLatent",
                    "inputs": {
                        "video_latent": [v_id, 0],
                        "audio_latent": [sep_av, 1]
                    }
                });
                latent_ref = json!([concat_av, 0]);
            }
            (Some(v_id), None) => {
                let sep_empty = ids.next();
                let concat_av = ids.next();
                graph[&sep_empty] = json!({
                    "class_type": "LTXVSeparateAVLatent",
                    "inputs": { "av_latent": ["5", 1] }
                });
                graph[&concat_av] = json!({
                    "class_type": "LTXVConcatAVLatent",
                    "inputs": {
                        "video_latent": [v_id, 0],
                        "audio_latent": [sep_empty, 1]
                    }
                });
                latent_ref = json!([concat_av, 0]);
            }
            (None, Some(a_id)) => {
                // `["5", 1]` is already nested: LTXVConcatAVLatent keeps empty video + fits source audio!
                let concat_av = ids.next();
                graph[&concat_av] = json!({
                    "class_type": "LTXVConcatAVLatent",
                    "inputs": {
                        "video_latent": ["5", 1],
                        "audio_latent": [a_id, 0]
                    }
                });
                latent_ref = json!([concat_av, 0]);
            }
            (None, None) => {}
        }
    }

    // 4. Chain `MiniMaxH3AddGuide` nodes onto positive conditioning ("5")
    let mut cond_node = "5".to_string();
    for guide in req.guides {
        if !guide.use_image && !guide.use_audio {
            continue;
        }
        let guide_node = ids.next();
        let mut inputs = json!({
            "positive": [cond_node, 0],
            "latent": latent_ref,
            "frame_idx": guide.frame_idx
        });
        match guide.kind.as_str() {
            "image" => {
                let load_img = ids.next();
                graph[&load_img] = json!({
                    "class_type": "LoadImage",
                    "inputs": { "image": guide.relative_file }
                });
                inputs["vae"] = json!(["3", 0]);
                inputs["image"] = json!([load_img, 0]);
            }
            "video" => {
                let load_vid = ids.next();
                let comp_vid = ids.next();
                graph[&load_vid] = json!({
                    "class_type": "LoadVideo",
                    "inputs": { "file": guide.relative_file }
                });
                graph[&comp_vid] = json!({
                    "class_type": "GetVideoComponents",
                    "inputs": { "video": [load_vid, 0] }
                });
                if guide.use_image {
                    inputs["vae"] = json!(["3", 0]);
                    inputs["image"] = json!([comp_vid, 0]);
                }
                if guide.use_audio {
                    inputs["audio_vae"] = json!(["4", 0]);
                    inputs["audio"] = json!([comp_vid, 1]);
                }
            }
            "audio" => {
                let load_aud = ids.next();
                graph[&load_aud] = json!({
                    "class_type": "LoadAudio",
                    "inputs": { "audio": guide.relative_file }
                });
                inputs["audio_vae"] = json!(["4", 0]);
                inputs["audio"] = json!([load_aud, 0]);
            }
            _ => continue,
        }
        graph[&guide_node] = json!({
            "class_type": "MiniMaxH3AddGuide",
            "inputs": inputs
        });
        cond_node = guide_node;
    }

    // 5. Sampling (`RandomNoise` -> `BasicScheduler` -> `KSamplerSelect` -> `BasicGuider`/`CFGGuider` -> `SamplerCustomAdvanced`)
    graph["6"] = json!({
        "class_type": "RandomNoise",
        "inputs": { "noise_seed": req.seed }
    });
    graph["7"] = json!({
        "class_type": "BasicScheduler",
        "inputs": {
            "model": [model_node, 0],
            "scheduler": req.scheduler,
            "steps": compile.resolved_steps,
            "denoise": compile.resolved_denoise
        }
    });
    graph["8"] = json!({
        "class_type": "KSamplerSelect",
        "inputs": { "sampler_name": req.sampler_name }
    });

    if req.cfg > 1.001 {
        let neg_id = ids.next();
        graph[&neg_id] = json!({
            "class_type": "CLIPTextEncode",
            "inputs": {
                "clip": ["2", 0],
                "text": compile.negative_prompt
            }
        });
        graph["9"] = json!({
            "class_type": "CFGGuider",
            "inputs": {
                "model": [model_node, 0],
                "positive": [cond_node, 0],
                "negative": [neg_id, 0],
                "cfg": req.cfg
            }
        });
    } else {
        graph["9"] = json!({
            "class_type": "BasicGuider",
            "inputs": {
                "model": [model_node, 0],
                "conditioning": [cond_node, 0]
            }
        });
    }

    graph["10"] = json!({
        "class_type": "SamplerCustomAdvanced",
        "inputs": {
            "noise": ["6", 0],
            "guider": ["9", 0],
            "sampler": ["8", 0],
            "sigmas": ["7", 0],
            "latent_image": latent_ref
        }
    });

    // 6. Decode Video + Audio -> CreateVideo (24 fps) -> SaveVideo
    if use_tiled_vae {
        graph["11"] = json!({
            "class_type": "VAEDecodeTiled",
            "inputs": {
                "samples": ["10", 0],
                "vae": ["3", 0],
                "tile_size": 1024,
                "overlap": 256,
                "temporal_size": 128,
                "temporal_overlap": 32
            }
        });
    } else {
        graph["11"] = json!({
            "class_type": "VAEDecode",
            "inputs": {
                "samples": ["10", 0],
                "vae": ["3", 0]
            }
        });
    }
    graph["12"] = json!({
        "class_type": "VAEDecodeAudio",
        "inputs": {
            "samples": ["10", 0],
            "vae": ["4", 0]
        }
    });
    graph["13"] = json!({
        "class_type": "CreateVideo",
        "inputs": {
            "images": ["11", 0],
            "audio": ["12", 0],
            "fps": 24.0
        }
    });
    graph["14"] = json!({
        "class_type": "SaveVideo",
        "inputs": {
            "video": ["13", 0],
            "filename_prefix": req.output_prefix,
            "format": "mp4",
            "codec": "h264"
        }
    });

    graph
}

fn build_lora_node(model_node: &str, lora: &H3LoraSpec) -> Value {
    json!({
        "class_type": "LoraLoaderModelOnly",
        "inputs": {
            "model": [model_node, 0],
            "lora_name": lora.file_name,
            "strength_model": lora.strength_model
        }
    })
}

/// Builds a `[T_video, H, W]` mask tensor (`0.0` = frozen context, `1.0` = generated span)
/// using stock `EmptyImage` / `LoadImage` + `RepeatImageBatch` + `ImageBatch` + `ImageToMask`.
#[allow(clippy::too_many_arguments)]
fn build_video_latent_mask(
    graph: &mut Value,
    ids: &mut NodeCounter,
    width: u32,
    height: u32,
    total_tokens: u32,
    lo: u32,
    hi: u32,
    spatial_mask_file: Option<&str>,
) -> String {
    let total = total_tokens.max(1);
    let lo = lo.min(total);
    let hi = hi.clamp(lo, total);

    let mut segments: Vec<String> = Vec::new();
    if lo > 0 {
        let pre_id = ids.next();
        graph[&pre_id] = json!({
            "class_type": "EmptyImage",
            "inputs": {
                "width": width,
                "height": height,
                "batch_size": lo,
                "color": 0
            }
        });
        segments.push(pre_id);
    }

    if hi > lo {
        let active_len = hi - lo;
        if let Some(mask_png) = spatial_mask_file {
            let load_id = ids.next();
            graph[&load_id] = json!({
                "class_type": "LoadImage",
                "inputs": { "image": mask_png }
            });
            if active_len > 1 {
                let rep_id = ids.next();
                graph[&rep_id] = json!({
                    "class_type": "RepeatImageBatch",
                    "inputs": {
                        "image": [load_id, 0],
                        "amount": active_len
                    }
                });
                segments.push(rep_id);
            } else {
                segments.push(load_id);
            }
        } else {
            let mid_id = ids.next();
            graph[&mid_id] = json!({
                "class_type": "EmptyImage",
                "inputs": {
                    "width": width,
                    "height": height,
                    "batch_size": active_len,
                    "color": 0xFFFFFF
                }
            });
            segments.push(mid_id);
        }
    }

    if hi < total {
        let post_id = ids.next();
        graph[&post_id] = json!({
            "class_type": "EmptyImage",
            "inputs": {
                "width": width,
                "height": height,
                "batch_size": total - hi,
                "color": 0
            }
        });
        segments.push(post_id);
    }

    let mut combined = segments.remove(0);
    for next_seg in segments {
        let batch_id = ids.next();
        graph[&batch_id] = json!({
            "class_type": "ImageBatch",
            "inputs": {
                "image1": [combined, 0],
                "image2": [next_seg, 0]
            }
        });
        combined = batch_id;
    }

    let mask_id = ids.next();
    graph[&mask_id] = json!({
        "class_type": "ImageToMask",
        "inputs": {
            "image": [combined, 0],
            "channel": "red"
        }
    });
    mask_id
}

/// Builds a `[1, 2, T_audio]` mask tensor (`0.0` = frozen audio, `1.0` = generated audio)
/// using stock `SolidMask` + `MaskComposite`.
fn build_audio_latent_mask(
    graph: &mut Value,
    ids: &mut NodeCounter,
    total_tokens: u32,
    lo: u32,
    hi: u32,
) -> String {
    let total = total_tokens.max(1);
    let lo = lo.min(total);
    let hi = hi.clamp(lo, total);

    if lo == 0 && hi == 0 {
        let zero_id = ids.next();
        graph[&zero_id] = json!({
            "class_type": "SolidMask",
            "inputs": {
                "value": 0.0,
                "width": total,
                "height": 2
            }
        });
        return zero_id;
    }
    if lo == 0 && hi >= total {
        let one_id = ids.next();
        graph[&one_id] = json!({
            "class_type": "SolidMask",
            "inputs": {
                "value": 1.0,
                "width": total,
                "height": 2
            }
        });
        return one_id;
    }

    let base_id = ids.next();
    let span_id = ids.next();
    let comp_id = ids.next();
    graph[&base_id] = json!({
        "class_type": "SolidMask",
        "inputs": {
            "value": 0.0,
            "width": total,
            "height": 2
        }
    });
    graph[&span_id] = json!({
        "class_type": "SolidMask",
        "inputs": {
            "value": 1.0,
            "width": (hi - lo).max(1),
            "height": 2
        }
    });
    graph[&comp_id] = json!({
        "class_type": "MaskComposite",
        "inputs": {
            "destination": [base_id, 0],
            "source": [span_id, 0],
            "x": lo,
            "y": 0,
            "operation": "add"
        }
    });
    comp_id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::studio::h3_geometry::calculate_take_placement;
    use kestrel_app_core::{
        H3CompiledPromptChannels, H3OrdinalManifest, H3PromptFormat, H3WindowConfig,
    };

    fn sample_compile(
        width: u32,
        height: u32,
        uses_source_av_latent: bool,
        uses_video_noise_mask: bool,
        uses_audio_noise_mask: bool,
    ) -> H3WindowCompileResult {
        let geom = calculate_take_placement(
            &H3WindowConfig {
                in_sec: 1.0,
                out_sec: 6.0,
                pre_context_sec: if uses_video_noise_mask { 0.5 } else { 0.0 },
                post_context_sec: 0.0,
                ..Default::default()
            },
            24,
        )
        .unwrap();
        H3WindowCompileResult {
            scene_id: "sc-1".into(),
            scene_name: "Scene 1".into(),
            resolved_width: width,
            resolved_height: height,
            resolved_model: "minimax_h3_ref2va_pruned_int8_convrot.safetensors".into(),
            resolved_steps: 10,
            resolved_denoise: if uses_source_av_latent { 0.75 } else { 1.0 },
            uses_source_av_latent,
            uses_video_noise_mask,
            uses_audio_noise_mask,
            resolved_loras: vec![H3LoraSpec {
                file_name: "minimax_h3_ref2v_turbo_4step_v0.1_comfyui_bf16.safetensors".into(),
                strength_model: 0.8,
                enabled: true,
            }],
            prompt_format: H3PromptFormat::H3FullReference,
            compiled_prompt: "Subject Definitions:\n- <Subject 1>: ...".into(),
            negative_prompt: String::new(),
            channels: H3CompiledPromptChannels::default(),
            ordinals: H3OrdinalManifest::default(),
            verdicts: Vec::new(),
            guides: Vec::new(),
            geometry: geom,
            advisories: Vec::new(),
        }
    }

    #[test]
    fn builds_stock_ref2va_graph_with_turbo_lora_sigma_shift_guides_and_masks() {
        let compile = sample_compile(1344, 768, true, true, true);
        let models = H3InstalledModelNames::default();
        let pics = vec![H3StagedGraphRef {
            relative_file: "kestrel/p1/portrait.png".into(),
            use_embedded_audio: false,
        }];
        let guides = vec![H3StagedGraphGuide {
            kind: "image".into(),
            relative_file: "kestrel/p1/guide0.png".into(),
            frame_idx: 0,
            use_image: true,
            use_audio: false,
        }];
        let graph = build_h3_comfy_graph(H3ComfyGraphBuild {
            compile: &compile,
            models: &models,
            task_mode: H3TaskMode::KeepExistingAudio,
            sampler_name: "er_sde",
            scheduler: "sgm_uniform",
            cfg: 1.0,
            shift_video: 12.0,
            shift_audio: 6.0,
            seed: 42,
            ref_image_size: "max",
            output_prefix: "kestrel_h3/job1",
            picture_refs: &pics,
            video_refs: &[],
            standalone_audio_refs: &[],
            guides: &guides,
            source_video_file: Some("kestrel/p1/src.mp4"),
            source_audio_file: Some("kestrel/p1/src.wav"),
            spatial_mask_file: None,
            preview_available: true,
        });

        assert_eq!(graph["5"]["class_type"], "MiniMaxH3ReferenceToVideo");
        assert_eq!(graph["11"]["class_type"], "VAEDecodeTiled");
        assert_eq!(graph[PREVIEW_NODE_ID]["class_type"], "ModelPreviewOverrideKJ");

        // Verify MiniMaxH3SigmaShift, LoraLoaderModelOnly, SetLatentNoiseMask, LTXVConcatAVLatent, MiniMaxH3AddGuide exist
        let classes: Vec<&str> = graph
            .as_object()
            .unwrap()
            .values()
            .filter_map(|v| v.get("class_type").and_then(Value::as_str))
            .collect();
        assert!(classes.contains(&"LoraLoaderModelOnly"));
        assert!(classes.contains(&"MiniMaxH3SigmaShift"));
        assert!(classes.contains(&"SetLatentNoiseMask"));
        assert!(classes.contains(&"LTXVConcatAVLatent"));
        assert!(classes.contains(&"LTXVSeparateAVLatent"));
        assert!(classes.contains(&"MiniMaxH3AddGuide"));
    }

    #[test]
    fn soundtrack_64x32_pass_uses_untiled_vae_decode() {
        let mut compile = sample_compile(64, 32, false, false, false);
        compile.resolved_model = "minimax_h3_fl2va_pruned_int8_convrot.safetensors".into();
        let models = H3InstalledModelNames::default();
        let graph = build_h3_comfy_graph(H3ComfyGraphBuild {
            compile: &compile,
            models: &models,
            task_mode: H3TaskMode::SoundtrackPass,
            sampler_name: "er_sde",
            scheduler: "sgm_uniform",
            cfg: 1.0,
            shift_video: 12.0,
            shift_audio: 6.0,
            seed: 7,
            ref_image_size: "match",
            output_prefix: "kestrel_h3/score",
            picture_refs: &[],
            video_refs: &[],
            standalone_audio_refs: &[],
            guides: &[],
            source_video_file: None,
            source_audio_file: None,
            spatial_mask_file: None,
            preview_available: true,
        });
        assert_eq!(graph["5"]["class_type"], "MiniMaxH3ImageToVideo");
        assert_eq!(graph["11"]["class_type"], "VAEDecode");
        assert!(graph.get(PREVIEW_NODE_ID).is_none());
    }
}
