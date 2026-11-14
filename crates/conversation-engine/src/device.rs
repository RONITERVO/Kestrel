//! Native duplex audio. AEC consumes the actual output callback samples (including underruns).
use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde_json::{Value, json};
use sonora::{
    AudioProcessing, Config, StreamConfig,
    config::{EchoCanceller, HighPassFilter, NoiseSuppression},
};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

struct Buffer {
    samples: VecDeque<f32>,
    active: Option<Arc<AtomicBool>>,
}
#[derive(Clone)]
pub struct Playback {
    buffer: Arc<Mutex<Buffer>>,
    alive: Arc<AtomicBool>,
    pub session_id: String,
}
impl Playback {
    pub fn begin(&self, cancel: Arc<AtomicBool>) -> Result<()> {
        if !self.alive.load(Ordering::Acquire) {
            bail!("Audio device closed");
        }
        self.buffer.lock().unwrap().active = Some(cancel);
        Ok(())
    }
    pub fn push(&self, pcm: &[f32], cancel: &AtomicBool) -> Result<()> {
        for part in pcm.chunks(240) {
            loop {
                if cancel.load(Ordering::Acquire) {
                    return Ok(());
                }
                if !self.alive.load(Ordering::Acquire) {
                    bail!("Audio device closed during playback");
                }
                {
                    let mut b = self.buffer.lock().unwrap();
                    if b.samples.len() + part.len() <= 24_000 {
                        b.samples.extend(part);
                        break;
                    }
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        Ok(())
    }
    pub fn drain(&self, cancel: &AtomicBool) -> Result<()> {
        while !cancel.load(Ordering::Acquire) {
            if !self.alive.load(Ordering::Acquire) {
                bail!("Audio device closed during playback");
            }
            if self.buffer.lock().unwrap().samples.is_empty() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        self.interrupt();
        Ok(())
    }
    fn interrupt(&self) {
        let mut b = self.buffer.lock().unwrap();
        b.samples.clear();
        if let Some(flag) = b.active.take() {
            flag.store(true, Ordering::Release);
        }
    }
}
impl Drop for AudioLifetime {
    fn drop(&mut self) {
        self.0.alive.store(false, Ordering::Release);
        self.0.interrupt();
    }
}
struct AudioLifetime(Playback);

enum Frame {
    Capture([f32; 160]),
    Render([f32; 160]),
}
/// Streaming resampler: phase is carried across callbacks and device packet boundaries.
struct Resampler {
    previous: f32,
    position: f64,
    step: f64,
    initialized: bool,
    frame: [f32; 160],
    used: usize,
}
impl Resampler {
    fn new(rate: u32) -> Self {
        Self {
            previous: 0.0,
            position: 0.0,
            step: rate as f64 / 16000.0,
            initialized: false,
            frame: [0.0; 160],
            used: 0,
        }
    }
    fn push(&mut self, sample: f32, mut frame: impl FnMut([f32; 160])) {
        if !self.initialized {
            self.previous = sample;
            self.initialized = true;
            return;
        }
        while self.position < 1.0 {
            self.frame[self.used] = self.previous + (sample - self.previous) * self.position as f32;
            self.used += 1;
            if self.used == 160 {
                frame(self.frame);
                self.used = 0;
            }
            self.position += self.step;
        }
        self.position -= 1.0;
        self.previous = sample;
    }
}
struct Gate {
    prefix: VecDeque<f32>,
    audio: Vec<f32>,
    voice_ms: u32,
    quiet_ms: u32,
    speaking: bool,
    threshold: f32,
    silence: u32,
    minimum: u32,
}
impl Gate {
    fn new(threshold: f32, silence: u32, minimum: u32) -> Self {
        Self {
            prefix: VecDeque::new(),
            audio: Vec::new(),
            voice_ms: 0,
            quiet_ms: 0,
            speaking: false,
            threshold,
            silence,
            minimum,
        }
    }
    fn push(&mut self, frame: &[f32; 160]) -> (bool, Option<Vec<f32>>) {
        let speech = 10.0 * (frame.iter().map(|v| v * v).sum::<f32>() / 160.0 + 1e-10).log10()
            >= self.threshold;
        if self.audio.is_empty() {
            self.prefix.extend(frame);
            while self.prefix.len() > 3200 {
                self.prefix.pop_front();
            }
            if !speech {
                return (false, None);
            }
            self.audio.extend(self.prefix.drain(..));
        } else {
            self.audio.extend(frame);
        }
        let mut started = false;
        if speech {
            self.voice_ms += 10;
            self.quiet_ms = 0;
            if !self.speaking && self.voice_ms >= self.minimum {
                self.speaking = true;
                started = true;
            }
        } else {
            self.quiet_ms += 10;
            if !self.speaking {
                self.voice_ms = 0;
            }
        }
        if self.quiet_ms >= self.silence || self.audio.len() >= 480_000 {
            let result = if self.speaking {
                Some(std::mem::take(&mut self.audio))
            } else {
                self.audio.clear();
                None
            };
            self.voice_ms = 0;
            self.quiet_ms = 0;
            self.speaking = false;
            return (started, result);
        }
        (started, None)
    }
}

fn capture<T: cpal::SizedSample + cpal::Sample>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    tx: mpsc::SyncSender<Frame>,
    failed: Arc<AtomicBool>,
) -> Result<cpal::Stream>
where
    f32: cpal::FromSample<T>,
{
    let channels = config.channels as usize;
    let mut resample = Resampler::new(config.sample_rate.0);
    let error = failed.clone();
    Ok(device.build_input_stream(
        config,
        move |data: &[T], _| {
            for frame in data.chunks_exact(channels) {
                let mono =
                    frame.iter().map(|s| s.to_sample::<f32>()).sum::<f32>() / channels as f32;
                resample.push(mono, |frame| {
                    if tx.try_send(Frame::Capture(frame)).is_err() {
                        failed.store(true, Ordering::Release);
                    }
                });
            }
        },
        move |_| error.store(true, Ordering::Release),
        None,
    )?)
}
fn render<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    tx: mpsc::SyncSender<Frame>,
    failed: Arc<AtomicBool>,
    playback: Playback,
) -> Result<cpal::Stream>
where
    T: cpal::SizedSample + cpal::Sample + cpal::FromSample<f32>,
{
    let channels = config.channels as usize;
    let mut reference = Resampler::new(config.sample_rate.0);
    let error = failed.clone();
    let step = 24000.0 / config.sample_rate.0 as f64;
    let mut position = 0.0;
    Ok(device.build_output_stream(
        config,
        move |data: &mut [T], _| {
            let mut guard = playback.buffer.try_lock().ok();
            for frame in data.chunks_exact_mut(channels) {
                let sample = if let Some(b) = guard.as_mut() {
                    if b.active.as_ref().is_some_and(|c| c.load(Ordering::Acquire)) {
                        b.samples.clear();
                    }
                    let a = b.samples.front().copied().unwrap_or(0.0);
                    let z = b.samples.get(1).copied().unwrap_or(a);
                    let sample = a + (z - a) * position as f32;
                    position += step;
                    while position >= 1.0 {
                        b.samples.pop_front();
                        position -= 1.0;
                    }
                    sample
                } else {
                    0.0
                };
                for out in frame {
                    *out = T::from_sample(sample);
                }
                reference.push(sample, |frame| {
                    if tx.try_send(Frame::Render(frame)).is_err() {
                        failed.store(true, Ordering::Release);
                    }
                });
            }
        },
        move |_| error.store(true, Ordering::Release),
        None,
    )?)
}

pub(crate) fn listen(
    request: &Value,
    cancel: &Arc<AtomicBool>,
    slot: &Arc<Mutex<Option<Playback>>>,
    mut emit: impl FnMut(&str, Value) -> Result<()>,
) -> Result<()> {
    let threshold = request["threshold"].as_f64().unwrap_or(-42.0) as f32;
    let silence = request["silenceMs"].as_f64().unwrap_or(600.0) as u32;
    let minimum = request["minimumMs"].as_f64().unwrap_or(200.0) as u32;
    if !threshold.is_finite()
        || !(-80.0..=0.0).contains(&threshold)
        || !(100..=10000).contains(&silence)
        || !(20..=5000).contains(&minimum)
    {
        bail!("Invalid native voice detection settings");
    }
    let host = cpal::default_host();
    let input=host.default_input_device().context("No microphone available. Select a Windows input device and allow desktop microphone access.")?;
    let output = host
        .default_output_device()
        .context("No speaker available. Select a Windows output device.")?;
    let input_config = input.default_input_config()?;
    let output_config = output.default_output_config()?;
    for config in [&input_config, &output_config] {
        if !(16000..=192000).contains(&config.sample_rate().0)
            || config.channels() == 0
            || config.channels() > 8
        {
            bail!("Unsupported audio device format. Select a 16–192 kHz microphone and speaker.");
        }
    }
    let playback = Playback {
        buffer: Arc::new(Mutex::new(Buffer {
            samples: VecDeque::with_capacity(24_000),
            active: None,
        })),
        alive: Arc::new(AtomicBool::new(true)),
        session_id: request["id"].as_str().unwrap_or_default().into(),
    };
    let _lifetime = AudioLifetime(playback.clone());
    let failed = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::sync_channel(40);
    let capture_stream = match input_config.sample_format() {
        cpal::SampleFormat::F32 => {
            capture::<f32>(&input, &input_config.config(), tx.clone(), failed.clone())?
        }
        cpal::SampleFormat::I16 => {
            capture::<i16>(&input, &input_config.config(), tx.clone(), failed.clone())?
        }
        cpal::SampleFormat::U16 => {
            capture::<u16>(&input, &input_config.config(), tx.clone(), failed.clone())?
        }
        _ => bail!("Unsupported microphone sample format"),
    };
    let render_stream = match output_config.sample_format() {
        cpal::SampleFormat::F32 => render::<f32>(
            &output,
            &output_config.config(),
            tx.clone(),
            failed.clone(),
            playback.clone(),
        )?,
        cpal::SampleFormat::I16 => render::<i16>(
            &output,
            &output_config.config(),
            tx.clone(),
            failed.clone(),
            playback.clone(),
        )?,
        cpal::SampleFormat::U16 => render::<u16>(
            &output,
            &output_config.config(),
            tx,
            failed.clone(),
            playback.clone(),
        )?,
        _ => bail!("Unsupported speaker sample format"),
    };
    let mut apm = processor();
    let mut gate = Gate::new(threshold, silence, minimum);
    let mut clean = [0.0; 160];
    render_stream.play()?;
    capture_stream.play()?;
    *slot.lock().unwrap() = Some(playback.clone());
    emit("listening", json!({}))?;
    while !cancel.load(Ordering::Acquire) {
        if failed.load(Ordering::Acquire) {
            bail!(
                "Audio device disconnected or could not keep up. Check Windows sound settings and retry the microphone."
            );
        }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Frame::Render(frame)) => {
                apm.process_render_f32(&[&frame], &mut [&mut clean])
                    .map_err(|e| anyhow::anyhow!("Echo reference failed: {e:?}"))?;
            }
            Ok(Frame::Capture(frame)) => {
                apm.process_capture_f32(&[&frame], &mut [&mut clean])
                    .map_err(|e| anyhow::anyhow!("Microphone processing failed: {e:?}"))?;
                let (started, utterance) = gate.push(&clean);
                if started {
                    playback.interrupt();
                    emit("speech_start", json!({}))?;
                }
                if let Some(pcm) = utterance {
                    let bytes: Vec<u8> = pcm
                        .iter()
                        .flat_map(|s| ((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes())
                        .collect();
                    if let Err(error) = emit("utterance", json!({"pcm":STANDARD.encode(bytes)})) {
                        emit("capture_error", json!({"detail":error.to_string()}))?;
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => bail!("Audio input stopped"),
        }
    }
    Ok(())
}

fn processor() -> AudioProcessing {
    let stream = StreamConfig::new(16000, 1);
    AudioProcessing::builder()
        .config(Config {
            echo_canceller: Some(EchoCanceller::default()),
            noise_suppression: Some(NoiseSuppression::default()),
            high_pass_filter: Some(HighPassFilter::default()),
            ..Config::default()
        })
        .capture_config(stream)
        .render_config(stream)
        .build()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn echo_reference_suppresses_delayed_speaker_audio_and_keeps_near_speech() {
        let mut apm = processor();
        let mut delay = VecDeque::from(vec![0.0; 640]);
        let mut seed = 123_u32;
        let mut input_energy = 0.0_f64;
        let mut output_energy = 0.0_f64;
        let mut near_energy = 0.0_f64;
        for index in 0..1000 {
            let mut render = [0.0; 160];
            let mut capture = [0.0; 160];
            for sample in 0..160 {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                render[sample] = ((seed >> 8) as f32 / 16777216.0 - 0.5) * 0.3;
                delay.push_back(render[sample]);
                capture[sample] = delay.pop_front().unwrap() * 0.5;
                if index >= 800 {
                    capture[sample] += ((index * 160 + sample) as f32 * 0.071).sin() * 0.2;
                }
            }
            let mut clean = [0.0; 160];
            apm.process_render_f32(&[&render], &mut [&mut clean])
                .unwrap();
            apm.process_capture_f32(&[&capture], &mut [&mut clean])
                .unwrap();
            if (500..800).contains(&index) {
                input_energy += capture.iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
                output_energy += clean.iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
            }
            if index >= 900 {
                near_energy += clean.iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
            }
        }
        assert!(
            output_energy < input_energy * 0.1,
            "echo energy ratio {}",
            output_energy / input_energy
        );
        assert!(
            near_energy / 16000.0 > 0.001,
            "near speech must survive double-talk"
        );
    }
    #[test]
    fn endpoint_retains_prefix_and_commits_once_then_recovers() {
        let mut gate = Gate::new(-42.0, 600, 200);
        let mut starts = 0;
        let mut utterances = Vec::new();
        for value in std::iter::repeat_n(0.0, 30)
            .chain(std::iter::repeat_n(0.1, 40))
            .chain(std::iter::repeat_n(0.0, 90))
            .chain(std::iter::repeat_n(0.1, 30))
            .chain(std::iter::repeat_n(0.0, 60))
        {
            let (start, utterance) = gate.push(&[value; 160]);
            starts += usize::from(start);
            if let Some(pcm) = utterance {
                utterances.push(pcm);
            }
        }
        assert_eq!(starts, 2);
        assert_eq!(utterances.len(), 2);
        assert!(utterances[0][..160].iter().all(|s| *s == 0.0));
    }
    #[test]
    fn short_noise_is_ignored_and_a_mid_sentence_pause_is_retained() {
        let mut gate = Gate::new(-42.0, 600, 200);
        for level in std::iter::repeat_n(0.0, 100)
            .chain(std::iter::repeat_n(0.1, 2))
            .chain(std::iter::repeat_n(0.0, 65))
        {
            let (started, utterance) = gate.push(&[level; 160]);
            assert!(!started);
            assert!(utterance.is_none());
        }
        for level in std::iter::repeat_n(0.1, 40)
            .chain(std::iter::repeat_n(0.0, 30))
            .chain(std::iter::repeat_n(0.1, 40))
        {
            assert!(gate.push(&[level; 160]).1.is_none());
        }
        let mut count = 0;
        for _ in 0..60 {
            if let (_, Some(audio)) = gate.push(&[0.0; 160]) {
                assert!(audio.len() >= 170 * 160);
                count += 1;
            }
        }
        assert_eq!(count, 1);
    }
    #[test]
    fn native_audio_never_exceeds_thirty_seconds() {
        let mut gate = Gate::new(-42.0, 600, 200);
        let mut outputs = 0;
        for _ in 0..6000 {
            if let (_, Some(pcm)) = gate.push(&[0.1; 160]) {
                assert!(pcm.len() <= 480000);
                outputs += 1;
            }
        }
        assert_eq!(outputs, 2);
    }
    #[test]
    fn resampling_keeps_fractional_phase_across_packets() {
        for rate in [16000, 44100, 48000, 96000] {
            let mut r = Resampler::new(rate);
            let mut frames = 0;
            for _ in 0..=rate {
                r.push(0.25, |frame| {
                    assert!(frame.iter().all(|s| (*s - 0.25).abs() < 1e-6));
                    frames += 1;
                });
            }
            assert_eq!(frames, 100);
        }
    }
}
