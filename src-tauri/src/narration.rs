//! Narration quality and export. The optional mistake check compares what Whisper heard with the
//! words the voice was asked to say: a clip that lost the text, repeated itself, or kept talking
//! after its last word is regenerated once. An export joins passages in order and leaves out
//! anything the voice said after a passage's last word.

use crate::models::SpeechTiming;
use crate::speech_text::spoken_words;
use crate::timed_text::{align_words, sentence_lines, PlayerTheme, TimedText, TimedWord};
use std::ffi::OsString;
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

/// Compare the spoken text with Whisper's word timings. Whisper often writes numbers as digits,
/// so heard words are spelled out the same way as the text before comparing.
pub fn check_narration(spoken: &str, heard: &[SpeechTiming]) -> NarrationCheck {
    let expected = spoken_words(spoken);
    let mut heard_words = Vec::new();
    let mut heard_timing = Vec::new();
    for (index, timing) in heard.iter().enumerate() {
        for word in spoken_words(&timing.value) {
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

/// One passage clip in an export: its audio, where to stop reading it, and the words it speaks
/// with Whisper's timings when someone listened to it.
#[derive(Debug, Clone)]
pub struct ExportClip {
    pub path: PathBuf,
    /// Stop here to leave out speech the voice added after the text.
    pub outpoint: Option<f64>,
    /// The passage's words as shown.
    pub text: String,
    /// What Whisper heard, in seconds from the clip's start; empty when nobody listened.
    pub heard: Vec<SpeechTiming>,
}

/// Where an exported clip should stop: just after its last word, when the voice kept talking.
pub fn export_outpoint(spoken: &str, heard: &[SpeechTiming]) -> Option<f64> {
    let check = check_narration(spoken, heard);
    (check.tail_words > 0)
        .then_some(check.speech_end)
        .flatten()
        .map(|end| end + TAIL_PADDING_SECONDS)
}

/// A file name for the export from its title: letters, digits, spaces, and dashes only.
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

/// Narration is joined as mono samples at this rate before it is encoded.
const SAMPLE_RATE: u32 = 48_000;
/// Longest FFmpeg may take for one step of an export, even a book-length one.
const FFMPEG_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30 * 60);
const STOPPED: &str = "Export stopped. Nothing was saved.";

fn arguments(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

/// FFmpeg arguments that decode one passage to raw mono samples, cut sample-exactly where the
/// passage's words end.
pub fn decode_arguments(clip: &Path, outpoint: Option<f64>) -> Vec<OsString> {
    let mut values = arguments(&["-hide_banner", "-loglevel", "error", "-nostdin", "-i"]);
    values.push(clip.as_os_str().to_owned());
    if let Some(end) = outpoint {
        values.push("-af".into());
        values.push(format!("atrim=end={end:.3}").into());
    }
    values.extend(arguments(&[
        "-vn", "-ac", "1", "-ar", "48000", "-f", "s16le", "pipe:1",
    ]));
    values
}

/// FFmpeg arguments that encode the joined raw samples as AAC in an M4A file.
pub fn encode_arguments(samples: &Path, output: &Path) -> Vec<OsString> {
    let mut values = arguments(&[
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-y",
        "-f",
        "s16le",
        "-ar",
        "48000",
        "-ac",
        "1",
        "-i",
    ]);
    values.push(samples.as_os_str().to_owned());
    values.extend(arguments(&[
        "-c:a",
        "aac",
        "-b:a",
        "128k",
        "-f",
        "ipod",
        "-movflags",
        "+faststart",
    ]));
    values.push(output.as_os_str().to_owned());
    values
}

/// FFmpeg arguments that turn a song master into AAC in an M4A file.
pub fn transcode_arguments(source: &Path, output: &Path) -> Vec<OsString> {
    let mut values = arguments(&["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"]);
    values.push(source.as_os_str().to_owned());
    values.extend(arguments(&[
        "-vn",
        "-c:a",
        "aac",
        "-b:a",
        "192k",
        "-f",
        "ipod",
        "-movflags",
        "+faststart",
    ]));
    values.push(output.as_os_str().to_owned());
    values
}

/// Run FFmpeg with fixed arguments and return what it wrote to standard output.
async fn run_ffmpeg(
    values: Vec<OsString>,
    step: &str,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<Vec<u8>, String> {
    let mut command = tokio::process::Command::new(crate::studio::media_program("ffmpeg"));
    command
        .args(values)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let child = command.spawn().map_err(|error| {
        format!(
            "FFmpeg is needed to export audio and could not start ({error}). Install Movie finishing tools in Setup, or choose ffmpeg.exe there."
        )
    })?;
    let output = tokio::select! {
        output = tokio::time::timeout(FFMPEG_TIMEOUT, child.wait_with_output()) => output,
        _ = cancel.cancelled() => return Err(STOPPED.into()),
    };
    let output = output
        .map_err(|_| format!("FFmpeg took more than 30 minutes to {step} and was stopped."))?
        .map_err(|error| format!("FFmpeg stopped while trying to {step}: {error}"))?;
    if !output.status.success() {
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
        return Err(format!("FFmpeg could not {step}: {tail}"));
    }
    Ok(output.stdout)
}

/// Join narration passages into `destination` as AAC. Each passage is decoded to raw samples and
/// cut exactly where its words end, so the joined timeline is known to the sample: the result
/// gives each passage's start and length in seconds. The file is written beside the destination
/// first and then replaces it, so a stopped or failed export never leaves half a file there.
pub async fn write_narration_audio(
    clips: &[ExportClip],
    destination: &Path,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<Vec<(f64, f64)>, String> {
    use std::io::Write as _;
    let samples_path = destination.with_extension("m4a.pcm");
    let partial = destination.with_extension("m4a.part");
    let result = async {
        let mut samples_file = std::fs::File::create(&samples_path).map_err(|error| {
            format!(
                "Could not prepare the narration export beside {}: {error}",
                destination.display()
            )
        })?;
        let mut spans = Vec::with_capacity(clips.len());
        let mut total = 0u64;
        for clip in clips {
            if cancel.is_cancelled() {
                return Err(STOPPED.to_string());
            }
            let pcm = run_ffmpeg(
                decode_arguments(&clip.path, clip.outpoint),
                "decode a narration passage",
                cancel,
            )
            .await?;
            let samples = (pcm.len() / 2) as u64;
            samples_file
                .write_all(&pcm[..samples as usize * 2])
                .map_err(|error| format!("Could not write the joined narration: {error}"))?;
            spans.push((
                total as f64 / f64::from(SAMPLE_RATE),
                samples as f64 / f64::from(SAMPLE_RATE),
            ));
            total += samples;
        }
        samples_file
            .flush()
            .map_err(|error| format!("Could not write the joined narration: {error}"))?;
        drop(samples_file);
        run_ffmpeg(
            encode_arguments(&samples_path, &partial),
            "encode the narration",
            cancel,
        )
        .await?;
        std::fs::rename(&partial, destination).map_err(|error| {
            format!(
                "The narration was joined but could not be saved as {}: {error}",
                destination.display()
            )
        })?;
        Ok(spans)
    }
    .await;
    let _ = std::fs::remove_file(&samples_path);
    let _ = std::fs::remove_file(&partial);
    result
}

/// Every passage's words on the joined file's timeline, as readable lines.
pub fn narration_timed_text(title: &str, clips: &[ExportClip], spans: &[(f64, f64)]) -> TimedText {
    let mut lines = Vec::new();
    let mut estimated = false;
    for (clip, (start, length)) in clips.iter().zip(spans) {
        let (words, guessed) = align_words(&clip.text, &clip.heard, *length);
        estimated |= guessed;
        let placed = words
            .into_iter()
            .map(|word| TimedWord {
                start: start + word.start.clamp(0.0, *length),
                end: start + word.end.clamp(0.0, *length),
                text: word.text,
            })
            .collect();
        lines.extend(sentence_lines(placed));
    }
    TimedText {
        title: title.to_string(),
        language: String::new(),
        theme: PlayerTheme::Paper,
        estimated,
        lines,
    }
}

/// Turn a song master into AAC at `destination`, written beside it first.
pub async fn write_song_audio(
    source: &Path,
    destination: &Path,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<(), String> {
    let partial = destination.with_extension("m4a.part");
    let result = async {
        run_ffmpeg(
            transcode_arguments(source, &partial),
            "convert the song",
            cancel,
        )
        .await?;
        std::fs::rename(&partial, destination).map_err(|error| {
            format!(
                "The song was converted but could not be saved as {}: {error}",
                destination.display()
            )
        })
    }
    .await;
    let _ = std::fs::remove_file(&partial);
    result
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
    fn exports_use_fixed_arguments_safe_names_and_one_timeline() {
        assert_eq!(
            export_file_name("The Cartographer: of *Falling* Stars?"),
            "The Cartographer of Falling Stars.m4a"
        );
        assert_eq!(export_file_name("???"), "Kestrel narration.m4a");

        let decode = decode_arguments(Path::new(r"C:\it's\b.opus"), Some(12.3456));
        assert!(decode
            .windows(2)
            .any(|pair| pair[0] == "-af" && pair[1] == "atrim=end=12.346"));
        assert_eq!(decode.last().unwrap(), "pipe:1");
        assert!(!decode_arguments(Path::new("a.opus"), None)
            .iter()
            .any(|value| value == "-af"));
        let encode = encode_arguments(Path::new("joined.pcm"), Path::new("out.m4a.part"));
        assert!(encode
            .windows(2)
            .any(|pair| pair[0] == "-f" && pair[1] == "s16le"));
        assert_eq!(encode.last().unwrap(), "out.m4a.part");
        let song = transcode_arguments(Path::new("take.flac"), Path::new("song.m4a.part"));
        assert!(song
            .windows(2)
            .any(|pair| pair[0] == "-b:a" && pair[1] == "192k"));

        // The second passage's words start where the first passage's audio ends.
        let clip = |text: &str, heard: &[(&str, f64, f64)]| ExportClip {
            path: PathBuf::from("clip.opus"),
            outpoint: None,
            text: text.into(),
            heard: heard
                .iter()
                .map(|(value, start, end)| SpeechTiming {
                    value: (*value).into(),
                    start: *start,
                    end: *end,
                })
                .collect(),
        };
        let clips = [
            clip("First line.", &[("First", 0.1, 0.4), ("line.", 0.4, 0.9)]),
            clip("Second one.", &[("Second", 0.2, 0.6), ("one.", 0.6, 1.0)]),
        ];
        let text = narration_timed_text("Story", &clips, &[(0.0, 1.5), (1.5, 1.2)]);
        assert!(!text.estimated);
        assert_eq!(text.lines.len(), 2);
        assert!((text.lines[1].words[0].start - 1.7).abs() < 1e-9);
        assert!((text.lines[1].end - 2.5).abs() < 1e-9);
    }
}
