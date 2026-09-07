//! Order-sensitive native model input. UI composition previews use the generated shape.
use serde::Serialize;
use ts_rs::TS;

#[derive(Serialize, TS)]
#[ts(export)]
pub struct OrderedCaption<'a> {
    pub high_level_description: &'a str,
    pub style_description: OrderedStyle<'a>,
    pub compositional_deconstruction: OrderedComposition<'a>,
}

#[derive(Serialize, TS)]
#[ts(export)]
#[serde(untagged)]
pub enum OrderedStyle<'a> {
    Photo(OrderedPhotoStyle<'a>),
    Art(OrderedArtStyle<'a>),
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct OrderedPhotoStyle<'a> {
    pub aesthetics: &'a str,
    pub lighting: &'a str,
    pub photo: &'a str,
    pub medium: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub color_palette: Option<&'a [String]>,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct OrderedArtStyle<'a> {
    pub aesthetics: &'a str,
    pub lighting: &'a str,
    pub medium: &'a str,
    pub art_style: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub color_palette: Option<&'a [String]>,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct OrderedComposition<'a> {
    pub background: &'a str,
    pub elements: Vec<OrderedElement<'a>>,
}

#[derive(Serialize, TS)]
#[ts(export)]
#[serde(untagged)]
pub enum OrderedElement<'a> {
    Object(OrderedObject<'a>),
    Text(OrderedText<'a>),
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct OrderedObject<'a> {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub bbox: [u16; 4],
    pub desc: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub color_palette: Option<&'a [String]>,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct OrderedText<'a> {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub bbox: [u16; 4],
    pub text: &'a str,
    pub desc: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub color_palette: Option<&'a [String]>,
}
