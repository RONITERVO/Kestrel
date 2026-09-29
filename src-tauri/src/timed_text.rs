//! Word-timed text for exports. Narration and songs become lines of words with start and end
//! times, saved beside their audio as enhanced LRC (karaoke-capable music players), WebVTT with
//! word timestamps (browsers and video players), JSON (anyone's own player), and a self-contained
//! HTML page that plays the audio and lights each word as it is heard.

use crate::models::{MusicLyricsDocument, SpeechTiming};
use crate::speech_text::spoken_words;
use serde::Serialize;
use std::path::{Path, PathBuf};

const PLAYER_TEMPLATE: &str = include_str!("../templates/word-player.html");
/// A line longer than this many words breaks at its next comma, colon, or semicolon.
const LONG_LINE_WORDS: usize = 18;
/// A line never runs past this many words, so captions stay readable.
const LONGEST_LINE_WORDS: usize = 26;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TimedWord {
    pub text: String,
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TimedLine {
    pub start: f64,
    pub end: f64,
    pub text: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub translation: String,
    pub words: Vec<TimedWord>,
}

/// The look of the exported player, after the lyric visualizer themes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PlayerTheme {
    /// Warm paper, following the viewer's dark mode.
    Paper,
    /// A night field with glowing words.
    Night,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimedText {
    pub title: String,
    pub language: String,
    pub theme: PlayerTheme,
    /// Some word times were spread evenly because no one listened to that audio.
    pub estimated: bool,
    pub lines: Vec<TimedLine>,
}

/// Place the words of `shown` on what was heard. The shown text may write numbers as digits
/// ("23,500") while the voice said them in words; both are compared as spoken words, so each
/// shown word takes the times of the heard words it stands for. Words nobody heard share the time
/// between their neighbours by length. Without heard words every word is spread over `duration`,
/// and the result says the times are estimated.
pub fn align_words(shown: &str, heard: &[SpeechTiming], duration: f64) -> (Vec<TimedWord>, bool) {
    let shown = shown.split_whitespace().collect::<Vec<_>>();
    if shown.is_empty() {
        return (Vec::new(), false);
    }
    let mut expected = Vec::new();
    let mut expected_word = Vec::new();
    for index in 0..shown.len() {
        for token in shown_word_tokens(&shown, index) {
            expected.push(token);
            expected_word.push(index);
        }
    }
    let mut heard_tokens = Vec::new();
    let mut heard_timing = Vec::new();
    for (index, timing) in heard.iter().enumerate() {
        for token in spoken_words(&timing.value) {
            heard_tokens.push(token);
            heard_timing.push(index);
        }
    }

    let mut starts = vec![None::<f64>; shown.len()];
    let mut ends = vec![None::<f64>; shown.len()];
    let (rows, columns) = (expected.len(), heard_tokens.len());
    let width = columns + 1;
    let mut table = vec![0u32; (rows + 1) * width];
    for row in 1..=rows {
        for column in 1..=columns {
            table[row * width + column] = if expected[row - 1] == heard_tokens[column - 1] {
                table[(row - 1) * width + column - 1] + 1
            } else {
                table[(row - 1) * width + column].max(table[row * width + column - 1])
            };
        }
    }
    let (mut row, mut column) = (rows, columns);
    while row > 0 && column > 0 {
        if expected[row - 1] == heard_tokens[column - 1]
            && table[row * width + column] == table[(row - 1) * width + column - 1] + 1
        {
            let word = expected_word[row - 1];
            let timing = &heard[heard_timing[column - 1]];
            starts[word] = Some(starts[word].map_or(timing.start, |start| start.min(timing.start)));
            ends[word] = Some(ends[word].map_or(timing.end, |end| end.max(timing.end)));
            row -= 1;
            column -= 1;
        } else if table[(row - 1) * width + column] >= table[row * width + column - 1] {
            row -= 1;
        } else {
            column -= 1;
        }
    }

    let estimated = starts.iter().all(Option::is_none);
    let (floor, ceiling) = if estimated && !heard.is_empty() {
        (
            heard[0].start,
            heard[heard.len() - 1].end.max(heard[0].start),
        )
    } else {
        (0.0, duration.max(0.0))
    };
    let mut words = Vec::with_capacity(shown.len());
    let mut index = 0;
    let mut cursor = floor;
    while index < shown.len() {
        if let (Some(start), Some(end)) = (starts[index], ends[index]) {
            let start = start.max(cursor).min(ceiling.max(cursor));
            let end = end.max(start);
            words.push(TimedWord {
                text: shown[index].to_string(),
                start,
                end,
            });
            cursor = start;
            index += 1;
            continue;
        }
        // A run of words nobody heard shares the time until the next heard word by length.
        let run_end = (index..shown.len())
            .find(|position| starts[*position].is_some())
            .unwrap_or(shown.len());
        let from = words
            .last()
            .map_or(floor, |word: &TimedWord| word.end.max(cursor));
        let to = starts
            .get(run_end)
            .copied()
            .flatten()
            .unwrap_or(ceiling)
            .max(from);
        let weights = shown[index..run_end]
            .iter()
            .map(|word| word.chars().count().max(1) as f64)
            .collect::<Vec<_>>();
        let total = weights.iter().sum::<f64>();
        let mut at = from;
        for (offset, weight) in weights.iter().enumerate() {
            let length = (to - from) * weight / total;
            words.push(TimedWord {
                text: shown[index + offset].to_string(),
                start: at,
                end: at + length,
            });
            at += length;
        }
        cursor = at;
        index = run_end;
    }
    (words, estimated)
}

/// How one shown word is spoken. A unit after a number is said in that number's context
/// ("23,500 kg." -> "kilograms"), so it matches what the voice said.
fn shown_word_tokens(shown: &[&str], index: usize) -> Vec<String> {
    let word = shown[index];
    match index.checked_sub(1).map(|previous| shown[previous]) {
        Some(previous) if previous.ends_with(|character: char| character.is_ascii_digit()) => {
            let before = spoken_words(previous).len();
            let together = spoken_words(&format!("{previous} {word}"));
            together
                .get(before..)
                .map(<[String]>::to_vec)
                .unwrap_or_default()
        }
        _ => spoken_words(word),
    }
}

fn ends_sentence(word: &str) -> bool {
    word.trim_end_matches(['"', '\'', ')', ']', '”', '’'])
        .ends_with(['.', '!', '?', '…'])
}

/// Group words into readable lines: one sentence per line, and long sentences broken at a
/// comma, colon, or semicolon.
pub fn sentence_lines(words: Vec<TimedWord>) -> Vec<TimedLine> {
    let mut lines = Vec::new();
    let mut current: Vec<TimedWord> = Vec::new();
    let flush = |current: &mut Vec<TimedWord>, lines: &mut Vec<TimedLine>| {
        if current.is_empty() {
            return;
        }
        let words = std::mem::take(current);
        lines.push(TimedLine {
            start: words[0].start,
            end: words[words.len() - 1].end,
            text: words
                .iter()
                .map(|word| word.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            translation: String::new(),
            words,
        });
    };
    for word in words {
        let breaks = ends_sentence(&word.text)
            || (current.len() + 1 >= LONG_LINE_WORDS && word.text.ends_with([',', ':', ';']))
            || current.len() + 1 >= LONGEST_LINE_WORDS;
        current.push(word);
        if breaks {
            flush(&mut current, &mut lines);
        }
    }
    flush(&mut current, &mut lines);
    lines
}

/// A song's saved lyrics as timed text: each lyric cue is a line with its words' times, and its
/// translation when the visualizer shows one. The player takes the visualizer's look.
pub fn song_timed_text(title: &str, document: &MusicLyricsDocument) -> TimedText {
    let mut segments = document
        .segments
        .iter()
        .filter(|segment| !segment.primary.trim().is_empty())
        .collect::<Vec<_>>();
    segments.sort_by(|left, right| left.start.total_cmp(&right.start));
    let lines = segments
        .into_iter()
        .map(|segment| TimedLine {
            start: segment.start,
            end: segment.end.max(segment.start),
            text: clean(&segment.primary),
            translation: if document.show_translation {
                clean(&segment.translation)
            } else {
                String::new()
            },
            words: segment
                .words
                .iter()
                .filter(|word| !word.value.trim().is_empty())
                .map(|word| TimedWord {
                    text: word.value.trim().to_string(),
                    start: word.start,
                    end: word.end.max(word.start),
                })
                .collect(),
        })
        .collect();
    TimedText {
        title: title.to_string(),
        language: if document.language == "auto" {
            String::new()
        } else {
            document.language.clone()
        },
        theme: if document.theme == "signal-bloom" {
            PlayerTheme::Night
        } else {
            PlayerTheme::Paper
        },
        estimated: false,
        lines,
    }
}

fn clean(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn lrc_time(seconds: f64) -> String {
    let hundredths = (seconds.max(0.0) * 100.0).round() as u64;
    format!(
        "{:02}:{:02}.{:02}",
        hundredths / 6_000,
        (hundredths / 100) % 60,
        hundredths % 100
    )
}

fn vtt_time(seconds: f64) -> String {
    let millis = (seconds.max(0.0) * 1_000.0).round() as u64;
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        millis / 3_600_000,
        (millis / 60_000) % 60,
        (millis / 1_000) % 60,
        millis % 1_000
    )
}

/// Enhanced LRC: each line starts at its time and each word carries its own, so karaoke-capable
/// players light words as they are sung or spoken. A translation follows at the same time.
pub fn to_lrc(text: &TimedText) -> String {
    let safe = |value: &str| {
        clean(value)
            .replace(['[', '<'], "(")
            .replace([']', '>'], ")")
    };
    let mut lrc = format!("[ti:{}]\n[re:Kestrel]\n", safe(&text.title));
    for line in &text.lines {
        lrc.push_str(&format!("[{}]", lrc_time(line.start)));
        if line.words.is_empty() {
            lrc.push_str(&safe(&line.text));
        } else {
            let words = line
                .words
                .iter()
                .map(|word| format!("<{}>{}", lrc_time(word.start), safe(&word.text)))
                .collect::<Vec<_>>()
                .join(" ");
            lrc.push_str(&format!("{words} <{}>", lrc_time(line.end)));
        }
        lrc.push('\n');
        if !line.translation.trim().is_empty() {
            lrc.push_str(&format!(
                "[{}]{}\n",
                lrc_time(line.start),
                safe(&line.translation)
            ));
        }
    }
    lrc
}

/// WebVTT with a timestamp before each word after the first, so players can style words as past
/// or still to come. A translation is the cue's second line.
pub fn to_webvtt(text: &TimedText) -> String {
    let escape = |value: &str| {
        clean(value)
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let mut vtt = String::from("WEBVTT\n\n");
    for (index, line) in text.lines.iter().enumerate() {
        let start = line.start;
        let end = line.end.max(start + 0.01);
        vtt.push_str(&format!(
            "{}\n{} --> {}\n",
            index + 1,
            vtt_time(start),
            vtt_time(end)
        ));
        if line.words.is_empty() {
            vtt.push_str(&escape(&line.text));
        } else {
            let mut last = start;
            let mut parts = Vec::new();
            for word in &line.words {
                // Inline timestamps must fall strictly inside the cue and keep increasing.
                if word.start > last + 0.0005 && word.start < end {
                    parts.push(format!("<{}>{}", vtt_time(word.start), escape(&word.text)));
                    last = word.start;
                } else {
                    parts.push(escape(&word.text));
                }
            }
            vtt.push_str(&parts.join(" "));
        }
        vtt.push('\n');
        if !line.translation.trim().is_empty() {
            vtt.push_str(&escape(&line.translation));
            vtt.push('\n');
        }
        vtt.push('\n');
    }
    vtt
}

/// The timed text as JSON, naming the audio file it belongs to.
pub fn to_json(text: &TimedText, audio_file: &str) -> Result<String, serde_json::Error> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Document<'a> {
        format: &'static str,
        version: u32,
        audio: &'a str,
        #[serde(flatten)]
        text: &'a TimedText,
    }
    serde_json::to_string_pretty(&Document {
        format: "kestrel-timed-text",
        version: 1,
        audio: audio_file,
        text,
    })
}

/// A single HTML file that carries the audio and its timed words and plays them anywhere a
/// browser runs, lighting each word as it is heard. User text is embedded as JSON with every `<`
/// escaped, so no lyric or reply can close the script that holds it.
pub fn to_player_html(
    text: &TimedText,
    audio: &[u8],
    mime: &str,
) -> Result<String, serde_json::Error> {
    use base64::Engine as _;
    let data = serde_json::to_string(text)?.replace('<', "\\u003c");
    let title = html_escape::encode_text(&clean(&text.title)).into_owned();
    let theme = match text.theme {
        PlayerTheme::Paper => "paper",
        PlayerTheme::Night => "night",
    };
    let page = PLAYER_TEMPLATE
        .replace("{{TITLE}}", &title)
        .replace("{{THEME}}", theme)
        .replace("{{MIME}}", mime)
        .replace("{{DATA}}", &data);
    // The audio is inserted last so no other replacement scans it.
    Ok(page.replace(
        "{{AUDIO}}",
        &base64::engine::general_purpose::STANDARD.encode(audio),
    ))
}

/// Write the LRC, WebVTT, JSON, and player files beside `audio`, each through a temporary file
/// so a failure never leaves a half-written companion. Returns the files written.
pub fn write_companions(text: &TimedText, audio: &Path) -> Result<Vec<PathBuf>, String> {
    let audio_name = audio
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("audio.m4a")
        .to_string();
    let bytes = std::fs::read(audio).map_err(|error| {
        format!("Could not read the exported audio for its player page: {error}")
    })?;
    let json = to_json(text, &audio_name).map_err(|error| error.to_string())?;
    let player = to_player_html(text, &bytes, "audio/mp4").map_err(|error| error.to_string())?;
    let mut written = Vec::new();
    for (extension, contents) in [
        ("lrc", to_lrc(text)),
        ("vtt", to_webvtt(text)),
        ("json", json),
        ("html", player),
    ] {
        let target = audio.with_extension(extension);
        let partial = audio.with_extension(format!("{extension}.part"));
        std::fs::write(&partial, contents.as_bytes())
            .and_then(|()| std::fs::rename(&partial, &target))
            .map_err(|error| {
                let _ = std::fs::remove_file(&partial);
                format!("Could not save {}: {error}", target.display())
            })?;
        written.push(target);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heard(words: &[(&str, f64, f64)]) -> Vec<SpeechTiming> {
        words
            .iter()
            .map(|(value, start, end)| SpeechTiming {
                value: (*value).into(),
                start: *start,
                end: *end,
            })
            .collect()
    }

    #[test]
    fn shown_digits_take_the_times_of_the_words_that_were_heard() {
        let (words, estimated) = align_words(
            "It was 23,500 kg.",
            &heard(&[
                ("It", 0.0, 0.2),
                ("was", 0.2, 0.4),
                ("twenty-three", 0.4, 1.0),
                ("thousand", 1.0, 1.4),
                ("five", 1.4, 1.6),
                ("hundred", 1.6, 1.9),
                ("kilograms.", 1.9, 2.5),
            ]),
            3.0,
        );
        assert!(!estimated);
        let texts = words
            .iter()
            .map(|word| word.text.as_str())
            .collect::<Vec<_>>();
        assert_eq!(texts, ["It", "was", "23,500", "kg."]);
        assert_eq!((words[2].start, words[2].end), (0.4, 1.9));
        assert_eq!((words[3].start, words[3].end), (1.9, 2.5));
    }

    #[test]
    fn unheard_words_share_the_gap_and_unheard_audio_is_estimated() {
        let (words, estimated) = align_words(
            "Code block on screen. Then more.",
            &heard(&[("Then", 2.0, 2.3), ("more.", 2.3, 2.8)]),
            3.0,
        );
        assert!(!estimated);
        assert_eq!(words[0].start, 0.0);
        assert!((words[3].end - 2.0).abs() < 1e-9, "{words:?}");
        assert!(words.windows(2).all(|pair| pair[1].start >= pair[0].start));

        let (spread, estimated) = align_words("One two three.", &[], 3.0);
        assert!(estimated);
        assert_eq!(spread[0].start, 0.0);
        assert!((spread[2].end - 3.0).abs() < 1e-9);
    }

    fn sample() -> TimedText {
        let (words, _) = align_words(
            "The tide rose. It was <high> & cold.",
            &heard(&[
                ("The", 1.0, 1.2),
                ("tide", 1.2, 1.5),
                ("rose.", 1.5, 2.0),
                ("It", 2.5, 2.6),
                ("was", 2.6, 2.8),
                ("high", 2.8, 3.1),
                ("and", 3.1, 3.2),
                ("cold.", 3.2, 3.6),
            ]),
            4.0,
        );
        let mut lines = sentence_lines(words);
        lines[0].translation = "Vuorovesi nousi.".into();
        TimedText {
            title: "The [Tide]".into(),
            language: "en".into(),
            theme: PlayerTheme::Night,
            estimated: false,
            lines,
        }
    }

    #[test]
    fn sentences_become_lines() {
        let text = sample();
        assert_eq!(text.lines.len(), 2);
        assert_eq!(text.lines[0].text, "The tide rose.");
        assert_eq!((text.lines[0].start, text.lines[0].end), (1.0, 2.0));
        assert_eq!(text.lines[1].text, "It was <high> & cold.");
    }

    #[test]
    fn lrc_times_every_word_and_keeps_markup_out() {
        let lrc = to_lrc(&sample());
        assert!(lrc.starts_with("[ti:The (Tide)]\n"));
        assert!(lrc.contains("[00:01.00]<00:01.00>The <00:01.20>tide <00:01.50>rose. <00:02.00>\n"));
        assert!(lrc.contains("[00:01.00]Vuorovesi nousi.\n"));
        assert!(lrc.contains("<00:02.80>(high)"));
        assert_eq!(lrc_time(3_725.456), "62:05.46");
    }

    #[test]
    fn webvtt_times_words_inside_each_cue_and_escapes_text() {
        let vtt = to_webvtt(&sample());
        assert!(vtt.starts_with("WEBVTT\n\n1\n00:00:01.000 --> 00:00:02.000\nThe <00:00:01.200>tide <00:00:01.500>rose.\nVuorovesi nousi.\n\n"));
        assert!(vtt.contains("&lt;high&gt;"));
        assert!(vtt.contains("&amp;"));
        assert_eq!(vtt_time(3_725.456_7), "01:02:05.457");
    }

    #[test]
    fn the_player_carries_audio_and_text_without_letting_text_close_its_script() {
        let mut text = sample();
        text.lines[1].text = "</script><script>alert(1)</script>".into();
        let page = to_player_html(&text, b"audio-bytes", "audio/mp4").unwrap();
        assert!(!page.contains("</script><script>alert"));
        assert!(page.contains("\\u003c/script>"));
        assert!(page.contains("YXVkaW8tYnl0ZXM="));
        assert!(page.contains("data-theme=\"night\""));
        assert!(page.contains("<title>The [Tide]</title>"));
        assert!(!page.contains("{{"));
        let json = to_json(&text, "The Tide.m4a").unwrap();
        assert!(json.contains("\"format\": \"kestrel-timed-text\""));
        assert!(json.contains("\"audio\": \"The Tide.m4a\""));
    }

    #[test]
    fn a_songs_cues_become_lines_with_its_words_translation_and_look() {
        let word = |value: &str, start: f64, end: f64| crate::models::MusicLyricWord {
            value: value.into(),
            start,
            end,
        };
        let segment = |id: &str, start: f64, end: f64, primary: &str, words| {
            crate::models::MusicLyricSegment {
                id: id.into(),
                start,
                end,
                primary: primary.into(),
                translation: format!("{primary} (fi)"),
                words,
            }
        };
        let document = MusicLyricsDocument {
            schema_version: 1,
            take_id: "take".into(),
            source_sha256: String::new(),
            revision: 3,
            language: "en".into(),
            source: "local-sync".into(),
            transcript: String::new(),
            theme: "signal-bloom".into(),
            show_translation: true,
            translation_language: "fi".into(),
            translation_model_id: String::new(),
            created_at: String::new(),
            updated_at: String::new(),
            segments: vec![
                segment(
                    "b",
                    5.0,
                    7.0,
                    "Second line",
                    vec![word("Second", 5.0, 5.8), word("line", 5.8, 7.0)],
                ),
                segment("gap", 3.0, 4.0, "  ", Vec::new()),
                segment("a", 1.0, 3.0, "First line", Vec::new()),
            ],
        };
        let text = song_timed_text("Song", &document);
        assert_eq!(text.theme, PlayerTheme::Night);
        assert_eq!(text.language, "en");
        assert_eq!(text.lines.len(), 2);
        assert_eq!(text.lines[0].text, "First line");
        assert!(text.lines[0].words.is_empty());
        assert_eq!(text.lines[1].translation, "Second line (fi)");
        assert_eq!(text.lines[1].words[1].start, 5.8);
        // A cue without word times is still one timed line in every format.
        assert!(to_lrc(&text).contains(
            "[00:01.00]First line
"
        ));
        assert!(to_webvtt(&text).contains(
            "00:00:01.000 --> 00:00:03.000
First line
"
        ));
        let hidden = song_timed_text(
            "Song",
            &MusicLyricsDocument {
                show_translation: false,
                theme: "sketchbook".into(),
                ..document
            },
        );
        assert!(hidden.lines.iter().all(|line| line.translation.is_empty()));
        assert_eq!(hidden.theme, PlayerTheme::Paper);
    }

    #[test]
    fn companions_are_written_beside_the_audio() {
        let directory = tempfile::tempdir().unwrap();
        let audio = directory.path().join("Story.m4a");
        std::fs::write(&audio, b"audio").unwrap();
        let written = write_companions(&sample(), &audio).unwrap();
        let names = written
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            ["Story.lrc", "Story.vtt", "Story.json", "Story.html"]
        );
        assert!(written.iter().all(|path| path.is_file()));
        assert!(std::fs::read_dir(directory.path())
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".part")));
    }
}
