//! MOSS-Image native prompt adapter. The editor keeps normalized, top-left-origin boxes.
//! Pixel-edge conversion matches ComfyUI v0.42.1's MOSSImageLayoutPrompt, including rounding
//! outwards so a narrow producer box cannot collapse at a smaller output resolution.
use crate::image::ImageProject;
use serde::Serialize;
use ts_rs::TS;

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct MossLayout {
    pub summary: MossSummary,
    pub items: Vec<MossItem>,
    pub bbox_coord_space: &'static str,
    pub bbox_format: &'static str,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct MossSummary {
    pub caption: String,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct MossItem {
    pub label: &'static str,
    pub caption: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub palette: Vec<String>,
    pub bbox: [i32; 4],
}

pub fn centered_pixel_box(bbox: [u16; 4], width: u32, height: u32) -> [i32; 4] {
    let lower =
        |edge: u16, extent: u32| (u32::from(edge) * extent / 1000) as i32 - (extent / 2) as i32;
    let upper = |edge: u16, extent: u32| {
        (u32::from(edge) * extent).div_ceil(1000) as i32 - (extent / 2) as i32
    };
    [
        lower(bbox[0], height),
        lower(bbox[1], width),
        upper(bbox[2], height),
        upper(bbox[3], width),
    ]
}

/// No element boxes means a natural-language caption, as in the upstream native node.
/// Otherwise the exact layout JSON goes directly into CLIPTextEncode, without re-enhancement.
pub fn prompt(project: &ImageProject) -> Result<(serde_json::Value, String), serde_json::Error> {
    let style = &project.style;
    let mut caption = [
        project.high_level_description.trim(),
        style.medium.trim(),
        style.aesthetics.trim(),
        style.lighting.trim(),
        if style.mode == "art" {
            style.art_style.trim()
        } else {
            style.photo.trim()
        },
        project.background.trim(),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join("\n");
    if !style.color_palette.is_empty() {
        caption.push_str(&format!(
            "\nOverall color palette: {}.",
            style.color_palette.join(", ")
        ));
    }
    if project.elements.is_empty() {
        return Ok((serde_json::Value::String(caption.clone()), caption));
    }
    let layout = MossLayout {
        summary: MossSummary { caption },
        items: project
            .elements
            .iter()
            .map(|element| MossItem {
                label: if element.kind == "text" {
                    "text"
                } else {
                    "object"
                },
                caption: element.description.trim().into(),
                text: (element.kind == "text").then(|| element.text.clone()),
                palette: element.color_palette.clone(),
                bbox: centered_pixel_box(
                    element.bbox,
                    project.settings.width,
                    project.settings.height,
                ),
            })
            .collect(),
        bbox_coord_space: "abs_centralized",
        bbox_format: "yxyx",
    };
    let text = serde_json::to_string(&layout)?;
    Ok((serde_json::from_str(&text)?, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centered_boxes_match_upstream_pixel_edges_across_aspects() {
        assert_eq!(
            centered_pixel_box([0, 0, 1000, 1000], 1536, 1024),
            [-512, -768, 512, 768]
        );
        assert_eq!(
            centered_pixel_box([125, 250, 875, 750], 1024, 1536),
            [-576, -256, 576, 256]
        );
        assert_eq!(
            centered_pixel_box([70, 80, 230, 920], 1536, 1024),
            [-441, -646, -276, 646]
        );
        assert_eq!(
            centered_pixel_box([999, 999, 1000, 1000], 256, 1536),
            [766, 127, 768, 128]
        );
        assert_eq!(
            centered_pixel_box([500, 500, 501, 501], 256, 256),
            [0, 0, 1, 1]
        );
    }
}
