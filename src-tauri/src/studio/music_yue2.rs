//! Native YuE2 adapter. The common music service still owns takes, cancellation and preservation.
use super::StudioError;
use kestrel_app_core::MusicSettings;
use serde_json::{json, Value};
use std::path::Path;

pub const INT8: &str = "yue2_3b_int8_convrot.safetensors";
pub const BF16: &str = "yue2_3b_bf16.safetensors";

pub fn resolve(root: &Path, variant: &str) -> Result<String, StudioError> {
    let name = match variant {
        "auto" | "int8" => INT8,
        "fp16" | "bf16" => BF16,
        _ => {
            return Err(StudioError::Invalid(
                "YuE2 variant must be auto, int8, or bf16.".into(),
            ))
        }
    };
    let path = root.join("models/checkpoints").join(name);
    let expected = if name == INT8 {
        Some(3_960_938_800)
    } else {
        None
    };
    let size = path
        .metadata()
        .map_err(|_| {
            StudioError::Render(format!(
                "{} is missing. Open Setup and install YuE2 music production.",
                path.display()
            ))
        })?
        .len();
    if expected.is_some_and(|expected| size != expected) || size == 0 {
        return Err(StudioError::Render(
            "The YuE2 checkpoint is incomplete. Resume its verified download in Setup.".into(),
        ));
    }
    Ok(name.into())
}

pub fn graph(
    settings: &MusicSettings,
    caption: &str,
    lyrics: &str,
    seed: u64,
    model: &str,
    prefix: &str,
) -> Value {
    let mut graph = json!({
        "1":{"class_type":"CheckpointLoaderSimple","inputs":{"ckpt_name":model}},
        "4":{"class_type":"YuE2GenerateMusic","inputs":{
            "clip":["1",1],"style":caption,"lyrics":lyrics,"abc":"","seed":seed,
            "mode":if settings.planning_mode == "melody" { "melody" } else { "full" },
            "max_duration":settings.max_duration_seconds,"temperature":1.0,"top_p":0.95,
            "top_k":settings.top_k,"repetition_penalty":1.2,"cfg_scale":settings.cfg_scale
        }},
        "5":{"class_type":"ConditioningZeroOut","inputs":{"conditioning":["4",0]}},
        "6":{"class_type":"EmptyYuE2LatentAudio","inputs":{"seconds":["4",1],"batch_size":1}},
        "7":{"class_type":"KSampler","inputs":{
            "model":["1",0],"positive":["4",0],"negative":["5",0],"latent_image":["6",0],
            "seed":seed,"steps":settings.steps,"cfg":1.0,"sampler_name":"dpm_2","scheduler":"sgm_uniform","denoise":1.0
        }},
        "8":{"class_type":"VAEDecodeAudio","inputs":{"samples":["7",0],"vae":["1",2]}},
        "10":{"class_type":"SaveAudioAdvanced","inputs":{"audio":["8",0],"filename_prefix":prefix,"format":"flac"}}
    });
    if settings.tiled_decode {
        graph["8"] = json!({"class_type":"VAEDecodeAudioTiled","inputs":{"samples":["7",0],"vae":["1",2],"tile_size":1920,"overlap":128}});
    }
    if settings.planning_mode != "off" {
        let score = if settings.abc_score.trim().is_empty() {
            graph["9"] = json!({"class_type":"YuE2GenerateABC","inputs":{
                "clip":["1",1],"style":caption,"lyrics":lyrics,"seed":seed,"mode":settings.planning_mode,
                "max_abc_tokens":8192,"temperature":0.7,"top_p":0.9,"top_k":30,"repetition_penalty":1.005,"penalty_window":100
            }});
            json!(["9", 0])
        } else {
            json!(settings.abc_score)
        };
        graph["4"]["inputs"]["abc"] = score.clone();
        graph["11"] = json!({"class_type":"PreviewAny","inputs":{"source":score}});
    }
    graph
}

pub fn completed_score(history: &Value, settings: &MusicSettings) -> Result<String, StudioError> {
    if settings.planning_mode == "off" {
        return Ok(String::new());
    }
    let score = history.pointer("/outputs/11/text/0").and_then(Value::as_str)
        .ok_or_else(|| StudioError::Render("YuE2 finished audio without its composition receipt. Preserve the render and inspect ComfyUI history before retrying.".into()))?;
    if score.trim().is_empty() || score.len() > 256 * 1024 {
        return Err(StudioError::Render(
            "YuE2 returned an empty or oversized ABC score.".into(),
        ));
    }
    Ok(score.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn direct_and_model_authored_scores_share_the_audio_path_and_preserve_receipts() {
        let settings = MusicSettings::default();
        let planned = graph(&settings, "style", "words", 42, INT8, "test");
        assert_eq!(planned["4"]["inputs"]["abc"], json!(["9", 0]));
        assert_eq!(planned["6"]["inputs"]["seconds"], json!(["4", 1]));
        let direct = MusicSettings {
            abc_score: "X:1\nK:C\nCDEF|".into(),
            ..settings.clone()
        };
        let authored = graph(&direct, "style", "words", 42, INT8, "test");
        assert!(authored.get("9").is_none());
        assert_eq!(authored["4"]["inputs"]["abc"], direct.abc_score);
        assert_eq!(authored["7"], planned["7"]);
        assert!(completed_score(&json!({}), &settings).is_err());
        assert_eq!(
            completed_score(
                &json!({"outputs":{"11":{"text":["X:1\nK:C\nCDEF|"]}}}),
                &settings
            )
            .unwrap(),
            direct.abc_score
        );
    }
    #[test]
    fn old_settings_keep_minimax_and_new_projects_default_to_yue2() {
        let mut legacy = serde_json::to_value(MusicSettings::default()).unwrap();
        for key in ["backend", "planningMode", "abcScore"] {
            legacy.as_object_mut().unwrap().remove(key);
        }
        let restored: MusicSettings = serde_json::from_value(legacy).unwrap();
        assert_eq!(
            restored.backend,
            kestrel_app_core::music::MusicBackend::MinimaxMusic3
        );
        assert_eq!(
            MusicSettings::default().backend,
            kestrel_app_core::music::MusicBackend::Yue2
        );
    }
}
