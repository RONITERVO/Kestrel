//! Narration quality and export. The optional mistake check compares what Whisper heard with the
//! words the voice was asked to say: a clip that lost the text, repeated itself, or kept talking
//! after its last word is regenerated once. An export joins passages in order and leaves out
//! anything the voice said after a passage's last word.

use crate::models::SpeechTiming;
use std::path::{Path, PathBuf};

/// Seconds kept after a passage's last word so its final syllable is not clipped.
pub const TAIL_PADDING_SECONDS: f64 = 0.35;
const MIN_MATCHED: f64 = 0.85;
const MAX_TAIL_WORDS: usize = 2;
const MAX_HEARD_RATIO: f64 = 1.35;

/// How closely a generated clip follows its text.
#[derive(Debug, Clone, PartialEq)]
pub struct NarrationCheck {
    /// Share of the passage's words that were heard, in order.
    pub matched: f64,
    /// Words heard after the passage's last word: speech the voice invented.
    pub tail_words: usize,
    /// When the passage's last heard word ends, in seconds.
    pub speech_end: Option<f64>,
    /// Heard words per word of text; well above one means the voice repeated itself.
    pub heard_ratio: f64,
    pub passed: bool,
}

impl NarrationCheck {
    /// Higher is better; used to keep the better of two takes.
    pub fn score(&self) -> f64 {
        self.matched - self.tail_words as f64 * 0.02 - (self.heard_ratio - 1.0).max(0.0)
    }
}

fn words_of(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Compare the spoken text with Whisper's word timings. Whisper often writes numbers as digits,
/// so heard words are spelled out the same way as the text before comparing.
pub fn check_narration(spoken: &str, heard: &[SpeechTiming]) -> NarrationCheck {
    let expected = words_of(spoken);
    let mut heard_words = Vec::new();
    let mut heard_timing = Vec::new();
    for (index, timing) in heard.iter().enumerate() {
        for word in words_of(&crate::speech_text::speak_numbers(&timing.value)) {
            heard_words.push(word);
            heard_timing.push(index);
        }
    }
    let rows = expected.len();
    let columns = heard_words.len();
    if rows == 0 {
        return NarrationCheck {
            matched: 1.0,
            tail_words: columns,
            speech_end: None,
            heard_ratio: 0.0,
            passed: columns <= MAX_TAIL_WORDS,
        };
    }
    // Longest common subsequence of text and heard words.
    let width = columns + 1;
    let mut table = vec![0u16; (rows + 1) * width];
    for row in 1..=rows {
        for column in 1..=columns {
            table[row * width + column] = if expected[row - 1] == heard_words[column - 1] {
                table[(row - 1) * width + column - 1] + 1
            } else {
                table[(row - 1) * width + column].max(table[row * width + column - 1])
            };
        }
    }
    let matched_words = table[rows * width + columns];
    // The passage ends at the first heard word by which every match is complete; anything later
    // is extra speech, even when it repeats earlier words.
    let end = (0..=columns)
        .find(|column| table[rows * width + column] == matched_words)
        .unwrap_or(columns);
    let speech_end = end
        .checked_sub(1)
        .and_then(|last| heard_timing.get(last))
        .map(|index| heard[*index].end);
    let matched = f64::from(matched_words) / rows as f64;
    let tail_words = columns - end;
    let heard_ratio = columns as f64 / rows as f64;
    // Very short passages give Whisper little to go on; they only fail on invented speech.
    let passed = if rows < 4 {
        tail_words <= MAX_TAIL_WORDS
    } else {
        matched >= MIN_MATCHED && tail_words <= MAX_TAIL_WORDS && heard_ratio <= MAX_HEARD_RATIO
    };
    NarrationCheck {
        matched,
        tail_words,
        speech_end,
        heard_ratio,
        passed,
    }
}

/// One passage clip in an export and where to stop reading it.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportClip {
    pub path: PathBuf,
    /// Stop here to leave out speech the voice added after the text.
    pub outpoint: Option<f64>,
}

/// Where an exported clip should stop: just after its last word, when the voice kept talking.
pub fn export_outpoint(spoken: &str, heard: &[SpeechTiming]) -> Option<f64> {
    let check = check_narration(spoken, heard);
    (check.tail_words > 0)
        .then_some(check.speech_end)
        .flatten()
        .map(|end| end + TAIL_PADDING_SECONDS)
}

/// FFmpeg concat script for the clips, in order. Paths are written with forward slashes inside
/// single quotes, the only character FFmpeg needs escaped there.
pub fn concat_script(clips: &[ExportClip]) -> String {
    let mut script = String::from("ffconcat version 1.0\n");
    for clip in clips {
        let path = clip
            .path
            .to_string_lossy()
            .replace('\\', "/")
            .replace('\'', r"'\''");
        script.push_str(&format!("file '{path}'\n"));
        if let Some(outpoint) = clip.outpoint {
            script.push_str(&format!("outpoint {outpoint:.3}\n"));
        }
    }
    script
}

/// A file name for the export from the reply's title: letters, digits, spaces, and dashes only.
pub fn export_file_name(title: &str) -> String {
    let cleaned = title
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || character == ' ' || character == '-' {
                character
            } else {
                ' '
            }
        })
        .collect::<String>();
    let words = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let name = words.chars().take(80).collect::<String>();
    format!(
        "{}.m4a",
        if name.trim().is_empty() {
            "Kestrel narration"
        } else {
            name.trim()
        }
    )
}

/// FFmpeg arguments that join the clips into one AAC audio file for audiobook players.
pub fn export_arguments(script: &Path, output: &Path) -> Vec<std::ffi::OsString> {
    let mut arguments = [
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-y",
        "-f",
        "concat",
        "-safe",
        "0",
        "-i",
    ]
    .map(std::ffi::OsString::from)
    .to_vec();
    arguments.push(script.as_os_str().to_owned());
    arguments.extend(
        [
            "-vn",
            "-c:a",
            "aac",
            "-b:a",
            "128k",
            "-f",
            "ipod",
            "-movflags",
            "+faststart",
        ]
        .map(std::ffi::OsString::from),
    );
    arguments.push(output.as_os_str().to_owned());
    arguments
}

/// Longest FFmpeg may take to join one narration, even a book-length one.
const EXPORT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// Join the clips into `destination`. FFmpeg writes beside it first and the finished file then
/// replaces the destination, so a stopped or failed export never leaves half a file there.
pub async fn write_export(
    clips: &[ExportClip],
    destination: &Path,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<(), String> {
    let script = destination.with_extension("m4a.ffconcat");
    let partial = destination.with_extension("m4a.part");
    std::fs::write(&script, concat_script(clips)).map_err(|error| {
        format!(
            "Could not prepare the narration export beside {}: {error}",
            destination.display()
        )
    })?;
    let cleanup = || {
        let _ = std::fs::remove_file(&script);
        let _ = std::fs::remove_file(&partial);
    };
    let mut command = tokio::process::Command::new(crate::studio::media_program("ffmpeg"));
    command
        .args(export_arguments(&script, &partial))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            cleanup();
            return Err(format!(
                "FFmpeg is needed to export narration and could not start ({error}). Install Movie finishing tools in Setup, or choose ffmpeg.exe there."
            ));
        }
    };
    let output = tokio::select! {
        output = tokio::time::timeout(EXPORT_TIMEOUT, child.wait_with_output()) => output,
        _ = cancel.cancelled() => {
            cleanup();
            return Err("Narration export stopped. Nothing was saved.".into());
        }
    };
    let output = match output {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            cleanup();
            return Err(format!(
                "FFmpeg stopped while joining the narration: {error}"
            ));
        }
        Err(_) => {
            cleanup();
            return Err(
                "FFmpeg took more than 30 minutes to join the narration and was stopped.".into(),
            );
        }
    };
    let _ = std::fs::remove_file(&script);
    if !output.status.success() {
        cleanup();
        let errors = String::from_utf8_lossy(&output.stderr);
        let tail = errors
            .trim()
            .chars()
            .rev()
            .take(600)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<String>();
        return Err(format!("FFmpeg could not join the narration: {tail}"));
    }
    std::fs::rename(&partial, destination).map_err(|error| {
        cleanup();
        format!(
            "The narration was joined but could not be saved as {}: {error}",
            destination.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heard(words: &[&str]) -> Vec<SpeechTiming> {
        words
            .iter()
            .enumerate()
            .map(|(index, value)| SpeechTiming {
                value: (*value).into(),
                start: index as f64 * 0.4,
                end: index as f64 * 0.4 + 0.35,
            })
            .collect()
    }

    #[test]
    fn a_faithful_clip_passes_even_when_whisper_writes_digits() {
        let spoken = "E equals twenty-three thousand five hundred times zero point six zero.";
        let check = check_narration(spoken, &heard(&["E", "equals", "23,500", "times", "0.60."]));
        assert!(check.passed, "{check:?}");
        assert_eq!(check.tail_words, 0);
        assert!((check.matched - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn invented_speech_after_the_text_fails_and_is_cut_from_exports() {
        let spoken = "Kael wrote E equals one over two m v squared for one projectile.";
        let words = heard(&[
            "Kael",
            "wrote",
            "E",
            "equals",
            "one",
            "over",
            "two",
            "m",
            "v",
            "squared",
            "for",
            "one",
            "projectile.",
            "Kael",
            "wrote",
            "E",
            "equals",
            "Kemes",
        ]);
        let check = check_narration(spoken, &words);
        assert!(!check.passed, "{check:?}");
        assert_eq!(check.tail_words, 5);
        assert_eq!(check.speech_end, Some(words[12].end));
        assert_eq!(
            export_outpoint(spoken, &words),
            Some(words[12].end + TAIL_PADDING_SECONDS)
        );
        assert_eq!(export_outpoint(spoken, &words[..13]), None);
    }

    #[test]
    fn a_looping_or_garbled_clip_fails() {
        let spoken = "E sub new equals E sub old times one minus r sub one.";
        let looping = heard(&[
            "E", "new", "equals", "EOLD", "MIMAN", "MIMAN", "MIMAN", "MIMAN", "MIMAN", "MIMAN",
            "MIMAN", "MIMAN", "MIMAN", "MIMAN", "MIMAN", "MIMAN",
        ]);
        let check = check_narration(spoken, &looping);
        assert!(!check.passed, "{check:?}");
        assert!(check.matched < 0.5);
        let better = check_narration(
            spoken,
            &heard(&[
                "E", "sub", "new", "equals", "E", "sub", "old", "times", "one", "minus", "r",
                "sub", "one.",
            ]),
        );
        assert!(better.score() > check.score());
    }

    #[test]
    fn exports_are_joined_in_order_with_safe_paths_and_names() {
        let script = concat_script(&[
            ExportClip {
                path: PathBuf::from(r"C:\Kestrel Research\speech-cache\a.opus"),
                outpoint: None,
            },
            ExportClip {
                path: PathBuf::from(r"C:\it's\b.opus"),
                outpoint: Some(12.3456),
            },
        ]);
        assert_eq!(
            script,
            "ffconcat version 1.0\nfile 'C:/Kestrel Research/speech-cache/a.opus'\nfile 'C:/it'\\''s/b.opus'\noutpoint 12.346\n"
        );
        assert_eq!(
            export_file_name("The Cartographer: of *Falling* Stars?"),
            "The Cartographer of Falling Stars.m4a"
        );
        assert_eq!(export_file_name("???"), "Kestrel narration.m4a");
        let arguments = export_arguments(Path::new("list.ffconcat"), Path::new("out.m4a.part"));
        assert!(arguments
            .windows(2)
            .any(|pair| pair[0] == "-f" && pair[1] == "concat"));
        assert_eq!(arguments.last().unwrap(), "out.m4a.part");
    }
}
