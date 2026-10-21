// Test-only unchanged Ideogram baseline from main. Never selected by the application.
use super::*;
use kestrel_app_core::ideogram::{
    OrderedArtStyle, OrderedCaption, OrderedComposition, OrderedElement, OrderedObject,
    OrderedPhotoStyle, OrderedStyle, OrderedText,
};
const IDEOGRAM_MODEL: &str = "ideogram4_nvfp4_mixed.safetensors";
const IDEOGRAM_UNCONDITIONAL_MODEL: &str = "ideogram4_unconditional_nvfp4_mixed.safetensors";
const IDEOGRAM_TEXT_ENCODER: &str = "qwen3vl_8b_nvfp4.safetensors";
const IDEOGRAM_VAE: &str = "flux2-vae.safetensors";
pub const IDEOGRAM_LICENSE_NOTICE: &str = "Ideogram 4 is provided under the Ideogram Non-Commercial Model Agreement. Generated work may not be used in or to advertise revenue-generating products or services unless Ideogram grants separate rights.";

/// Compile the producer-owned composition into Ideogram's order-sensitive caption schema.
/// The string is durable inference truth; the Value is only a convenient UI/receipt view.
pub(super) fn structured_prompt(project: &ImageProject) -> Result<(Value, String), StudioError> {
    let elements = project
        .elements
        .iter()
        .map(|element| {
            if element.kind == "text" {
                OrderedElement::Text(OrderedText {
                    kind: "text",
                    bbox: element.bbox,
                    text: element.text.as_str(),
                    desc: element.description.trim(),
                    color_palette: (!element.color_palette.is_empty())
                        .then_some(element.color_palette.as_slice()),
                })
            } else {
                OrderedElement::Object(OrderedObject {
                    kind: "obj",
                    bbox: element.bbox,
                    desc: element.description.trim(),
                    color_palette: (!element.color_palette.is_empty())
                        .then_some(element.color_palette.as_slice()),
                })
            }
        })
        .collect();
    let style_description = if project.style.mode == "art" {
        OrderedStyle::Art(OrderedArtStyle {
            aesthetics: project.style.aesthetics.trim(),
            lighting: project.style.lighting.trim(),
            medium: project.style.medium.trim(),
            art_style: project.style.art_style.trim(),
            color_palette: (!project.style.color_palette.is_empty())
                .then_some(project.style.color_palette.as_slice()),
        })
    } else {
        OrderedStyle::Photo(OrderedPhotoStyle {
            aesthetics: project.style.aesthetics.trim(),
            lighting: project.style.lighting.trim(),
            photo: project.style.photo.trim(),
            medium: project.style.medium.trim(),
            color_palette: (!project.style.color_palette.is_empty())
                .then_some(project.style.color_palette.as_slice()),
        })
    };
    let caption = OrderedCaption {
        high_level_description: project.high_level_description.trim(),
        style_description,
        compositional_deconstruction: OrderedComposition {
            background: project.background.trim(),
            elements,
        },
    };
    let text = serde_json::to_string(&caption)?;
    let value = serde_json::from_str(&text)?;
    Ok((value, text))
}

pub(super) fn ideogram_graph(
    settings: &ImageSettings,
    seed: u64,
    prompt: &str,
    prefix: &str,
) -> Value {
    let (steps, mu, std) = preset_parameters(&settings.preset);
    json!({
        "1":{"class_type":"UNETLoader","inputs":{"unet_name":IDEOGRAM_MODEL,"weight_dtype":"default"}},
        "2":{"class_type":"UNETLoader","inputs":{"unet_name":IDEOGRAM_UNCONDITIONAL_MODEL,"weight_dtype":"default"}},
        "3":{"class_type":"CLIPLoader","inputs":{"clip_name":IDEOGRAM_TEXT_ENCODER,"type":"ideogram4","device":"default"}},
        "4":{"class_type":"VAELoader","inputs":{"vae_name":IDEOGRAM_VAE}},
        "5":{"class_type":"CLIPTextEncode","inputs":{"clip":["3",0],"text":prompt}},
        "6":{"class_type":"ConditioningZeroOut","inputs":{"conditioning":["5",0]}},
        "7":{"class_type":"CFGOverride","inputs":{"model":["1",0],"cfg":3.0,"start_percent":0.7,"end_percent":1.0}},
        "8":{"class_type":"DualModelGuider","inputs":{"model":["7",0],"model_negative":["2",0],"positive":["5",0],"negative":["6",0],"cfg":7.0}},
        "9":{"class_type":"RandomNoise","inputs":{"noise_seed":seed}},
        "10":{"class_type":"KSamplerSelect","inputs":{"sampler_name":"euler"}},
        "11":{"class_type":"Ideogram4Scheduler","inputs":{"steps":steps,"width":settings.width,"height":settings.height,"mu":mu,"std":std}},
        "12":{"class_type":"EmptyFlux2LatentImage","inputs":{"width":settings.width,"height":settings.height,"batch_size":settings.batch_size}},
        "13":{"class_type":"SamplerCustomAdvanced","inputs":{"noise":["9",0],"guider":["8",0],"sampler":["10",0],"sigmas":["11",0],"latent_image":["12",0]}},
        "14":{"class_type":"VAEDecode","inputs":{"samples":["13",0],"vae":["4",0]}},
        "15":{"class_type":"SaveImage","inputs":{"images":["14",0],"filename_prefix":prefix}}
    })
}

pub(super) fn preset_parameters(preset: &str) -> (u32, f64, f64) {
    match preset {
        "quality" => (48, 0.0, 1.5),
        "turbo" => (12, 0.5, 1.75),
        _ => (20, 0.0, 1.75),
    }
}

pub(super) async fn verify_ideogram_nodes(http: &Client) -> Result<(), StudioError> {
    let info: Value = http
        .get(format!("{COMFY_BASE}/object_info"))
        .timeout(Duration::from_secs(30))
        .send()
        .await?
        .json()
        .await?;
    for node in [
        "Ideogram4Scheduler",
        "DualModelGuider",
        "CFGOverride",
        "EmptyFlux2LatentImage",
        "SamplerCustomAdvanced",
    ] {
        if info.get(node).is_none() {
            return Err(StudioError::Render(format!(
                "the running ComfyUI does not expose {node}. Update ComfyUI to 0.33.1 or newer, restart it, then resume Image Studio in Setup"
            )));
        }
    }
    Ok(())
}

pub(super) fn verify_ideogram_assets(comfy_root: &Path) -> Result<(), StudioError> {
    for relative in [
        format!("models/diffusion_models/{IDEOGRAM_MODEL}"),
        format!("models/diffusion_models/{IDEOGRAM_UNCONDITIONAL_MODEL}"),
        format!("models/text_encoders/{IDEOGRAM_TEXT_ENCODER}"),
        format!("models/vae/{IDEOGRAM_VAE}"),
        "models/diffusion_models/IDEOGRAM-4-NON-COMMERCIAL-LICENSE.txt".into(),
    ] {
        let path = comfy_root.join(relative);
        if !path.is_file() {
            return Err(StudioError::Render(format!(
                "Ideogram 4 asset is missing: {}. Resume Image Studio in Setup.",
                path.display()
            )));
        }
    }
    Ok(())
}
