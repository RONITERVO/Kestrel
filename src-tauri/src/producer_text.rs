//! Producer authoring adapters. Direct authors and local-model collaborators use the same saves.
use kestrel_app_core::{ImageElement, ImageProject, MusicProject, MusicSection};
use serde_json::Value;

pub fn music(mut project: MusicProject, field: &str, text: &str) -> Result<MusicProject, String> {
    if text.len() > 64 * 1024 {
        return Err("Music text exceeds 64 KiB.".into());
    }
    match field {
        "caption" => project.caption = text.into(),
        "lyrics" => project.sections = lyrics(&project.sections, text)?,
        _ => return Err("Music field must be caption or lyrics.".into()),
    }
    Ok(project)
}

fn lyrics(sections: &[MusicSection], text: &str) -> Result<Vec<MusicSection>, String> {
    let tags = [
        "Intro",
        "Verse",
        "Pre-Chorus",
        "Chorus",
        "Post-Chorus",
        "Bridge",
        "Instrumental",
        "Solo",
        "Break",
        "Outro",
    ];
    let mut blocks: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            let normalized = line[1..line.len() - 1]
                .to_ascii_lowercase()
                .replace([' ', '-', '_'], "");
            let tag = tags.iter().find(|tag| tag.to_ascii_lowercase().replace('-', "") == normalized).ok_or_else(|| format!("Unknown lyric section: {line}. Use Intro, Verse, Chorus, Pre-Chorus, Post-Chorus, Bridge, Instrumental, Solo, Break or Outro."))?;
            blocks.push(((*tag).into(), String::new()));
        } else if let Some((_, lyrics)) = blocks.last_mut() {
            lyrics.push_str(line);
            lyrics.push('\n');
        }
    }
    let mut result = sections.to_vec();
    if blocks.is_empty() {
        let first = result
            .first_mut()
            .ok_or("Add a song section before applying untagged lyrics.")?;
        first.lyrics = text.trim().into();
        return Ok(result);
    }
    let mut seen = std::collections::HashMap::new();
    for (tag, text) in blocks {
        let occurrence = seen.entry(tag.clone()).or_insert(0usize);
        let matching = sections
            .iter()
            .enumerate()
            .filter(|(_, s)| s.tag == tag)
            .nth(*occurrence)
            .map(|(index, _)| index);
        if let Some(index) = matching {
            result[index].lyrics = text.trim().into();
        } else {
            result.push(MusicSection {
                id: uuid::Uuid::new_v4().to_string(),
                name: format!("{tag} {}", *occurrence + 1),
                bars: if matches!(tag.as_str(), "Intro" | "Outro") {
                    4
                } else {
                    8
                },
                tag,
                lyrics: text.trim().into(),
                direction: String::new(),
            });
        }
        *occurrence += 1;
    }
    Ok(result)
}

pub fn image(mut project: ImageProject, text: &str) -> Result<ImageProject, String> {
    if text.len() > 256 * 1024 {
        return Err("Image proposal exceeds 256 KiB.".into());
    }
    let value: Value = crate::structured_output::parse(text)
        .ok_or("Image proposal must contain one valid JSON object.")?;
    let required = |value: &Value, key: &str| -> Result<String, String> {
        value
            .get(key)
            .and_then(Value::as_str)
            .filter(|v| !v.trim().is_empty())
            .map(String::from)
            .ok_or_else(|| format!("Image proposal needs {key}."))
    };
    let style = value
        .get("style_description")
        .ok_or("Image proposal needs style_description.")?;
    let composition = value
        .get("compositional_deconstruction")
        .ok_or("Image proposal needs compositional_deconstruction.")?;
    let photo = style
        .get("photo")
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty());
    let art = style
        .get("art_style")
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty());
    if photo.is_some() == art.is_some() {
        return Err("Image style needs exactly one of photo or art_style.".into());
    }
    project.high_level_description = required(&value, "high_level_description")?;
    project.style.aesthetics = required(style, "aesthetics")?;
    project.style.lighting = required(style, "lighting")?;
    project.style.medium = required(style, "medium")?;
    if let Some(photo) = photo {
        project.style.mode = "photo".into();
        project.style.photo = photo.into();
    }
    if let Some(art) = art {
        project.style.mode = "art".into();
        project.style.art_style = art.into();
    }
    project.style.color_palette = palette(style, 16)?;
    project.background = required(composition, "background")?;
    let elements = composition
        .get("elements")
        .and_then(Value::as_array)
        .ok_or("Image proposal needs an elements array.")?;
    if elements.len() > 64 {
        return Err("Image proposal exceeds 64 elements.".into());
    }
    project.elements = elements
        .iter()
        .map(|element| {
            let kind = required(element, "type")?;
            if kind != "text" && kind != "obj" {
                return Err("Image elements must be obj or text.".into());
            }
            let bbox: [u16; 4] =
                serde_json::from_value(element.get("bbox").cloned().unwrap_or(Value::Null))
                    .map_err(|_| "Each bbox needs four integers in top,left,bottom,right order.")?;
            if bbox.iter().any(|v| *v > 1000) || bbox[0] >= bbox[2] || bbox[1] >= bbox[3] {
                return Err("Bbox coordinates must be ordered within 0–1000.".into());
            }
            Ok(ImageElement {
                id: uuid::Uuid::new_v4().to_string(),
                text: if kind == "text" {
                    required(element, "text")?
                } else {
                    String::new()
                },
                kind,
                bbox,
                description: required(element, "desc")?,
                color_palette: palette(element, 5)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    project.settings.prompt_mode = kestrel_app_core::image::ImagePromptMode::Composition;
    Ok(project)
}
fn palette(value: &Value, maximum: usize) -> Result<Vec<String>, String> {
    let values: Vec<String> = serde_json::from_value(
        value
            .get("color_palette")
            .cloned()
            .unwrap_or_else(|| serde_json::json!([])),
    )
    .map_err(|_| "A palette must be an array of #RRGGBB colors.")?;
    if values.len() > maximum
        || values.iter().any(|v| {
            v.len() != 7
                || !v.starts_with('#')
                || !v.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
        })
    {
        return Err(format!("Palette needs at most {maximum} #RRGGBB colors."));
    }
    Ok(values.into_iter().map(|v| v.to_ascii_uppercase()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_sections_preserve_ids_and_unsupplied_material() {
        let sections = vec![
            MusicSection {
                id: "a".into(),
                tag: "Verse".into(),
                name: "One".into(),
                bars: 8,
                lyrics: "old".into(),
                direction: "soft".into(),
            },
            MusicSection {
                id: "b".into(),
                tag: "Verse".into(),
                name: "Two".into(),
                bars: 8,
                lyrics: "old2".into(),
                direction: "loud".into(),
            },
        ];
        let merged = lyrics(&sections, "[Verse]\nfirst\n[Verse]\nsecond\n[Chorus]\nhook").unwrap();
        assert_eq!(merged[0].id, "a");
        assert_eq!(merged[1].direction, "loud");
        assert_eq!(merged[1].lyrics, "second");
        assert_eq!(merged[2].tag, "Chorus");
        assert!(lyrics(&sections, "[Invented]\ntext").is_err());
    }
}
