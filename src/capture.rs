//! Native Windows WASAPI capture. No browser, video stream, or HTTP connection.
use crate::{
    audio::{AudioClock, Chunk, Mixer, Resampler},
    overlay::Overlay,
};
use anyhow::{Context, Result, bail};
use cpal::{
    FromSample, SampleFormat, SizedSample,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

pub const RATE: u32 = 16000;
pub struct CaptureResult {
    pub warning: Option<String>,
    pub chunks: u64,
}
pub struct Capture {
    pub stop: Arc<AtomicBool>,
    pub active: Arc<AtomicBool>,
    pub level: Arc<AtomicU32>,
    pub clock: Arc<AudioClock>,
    pub paused: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Result<CaptureResult>>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Capture {
    pub fn start(
        path: PathBuf,
        system: bool,
        mic: bool,
        overlay: Overlay,
        wake: impl Fn() + Send + 'static,
    ) -> Self {
        overlay.pause.store(false, Ordering::SeqCst);
        let stop = Arc::new(AtomicBool::new(false));
        let active = Arc::new(AtomicBool::new(false));
        let level = Arc::new(AtomicU32::new(0));
        let (sender, receiver) = mpsc::channel();
        let clock = Arc::new(AudioClock::new());
        let clock_worker = clock.clone();
        let paused = overlay.pause.clone();
        let (a, b, c) = (stop.clone(), active.clone(), level.clone());
        let worker = thread::spawn(move || {
            let result = record(
                path,
                system,
                mic,
                overlay.clone(),
                Control {
                    stop: a,
                    active: b.clone(),
                    level: c,
                    clock: clock_worker,
                },
            );
            b.store(false, Ordering::SeqCst);
            overlay.hide();
            let _ = sender.send(result);
            wake();
        });
        Self {
            stop,
            active,
            level,
            clock,
            paused,
            receiver,
            worker: Some(worker),
        }
    }
    pub fn poll(&self) -> Option<Result<CaptureResult>> {
        self.receiver.try_recv().ok()
    }
    pub fn finish(&mut self) -> Option<Result<CaptureResult>> {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.poll()
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
#[derive(Clone)]
struct StreamContext {
    source: usize,
    clock: Arc<AudioClock>,
    paused: Arc<AtomicBool>,
    sender: mpsc::SyncSender<Chunk>,
    dropped: Arc<AtomicU64>,
    errors: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
}
fn stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    context: StreamContext,
) -> Result<cpal::Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let StreamContext {
        source,
        clock,
        paused,
        sender,
        dropped,
        errors,
        stop,
    } = context;
    let channels = config.channels as usize;
    let mut resampler = Resampler::new(config.sample_rate.0, RATE);
    let mut next = 0u64;
    Ok(device.build_input_stream(
        config,
        move |data: &[T], info| {
            let stamp = info.timestamp();
            if paused.load(Ordering::SeqCst)
                || !clock.accepts(
                    stamp
                        .callback
                        .duration_since(&stamp.capture)
                        .unwrap_or_default(),
                )
            {
                return;
            }
            let mono = data.chunks_exact(channels).map(|frame| {
                frame.iter().map(|s| s.to_sample::<f32>()).sum::<f32>() / channels as f32
            });
            let samples = resampler.push(mono);
            let elapsed = (clock.elapsed().as_secs_f64() * RATE as f64) as u64;
            // Re-anchor after silence (WASAPI loopback may stop delivering silent buffers).
            let wall = elapsed.saturating_sub(samples.len() as u64);
            let at = if wall.abs_diff(next) > RATE as u64 / 10 {
                wall.max(next)
            } else {
                next
            };
            next = at + samples.len() as u64;
            if sender
                .try_send(Chunk {
                    source,
                    start: at,
                    samples,
                })
                .is_err()
            {
                dropped.fetch_add(1, Ordering::Relaxed);
            }
        },
        move |e| {
            if let Ok(mut error) = errors.lock() {
                *error = Some(format!("Audio device stopped: {e}"));
            }
            stop.store(true, Ordering::SeqCst);
        },
        None,
    )?)
}
fn input(device: &cpal::Device, output: bool, context: StreamContext) -> Result<cpal::Stream> {
    let supported = if output {
        device.default_output_config()?
    } else {
        device.default_input_config()?
    };
    let config = supported.config();
    // CPAL's WASAPI backend explicitly supports build_input_stream on output
    // devices and enables AUDCLNT_STREAMFLAGS_LOOPBACK for this case.
    match supported.sample_format() {
        SampleFormat::F32 => stream::<f32>(device, &config, context),
        SampleFormat::F64 => stream::<f64>(device, &config, context),
        SampleFormat::I16 => stream::<i16>(device, &config, context),
        SampleFormat::I32 => stream::<i32>(device, &config, context),
        SampleFormat::U16 => stream::<u16>(device, &config, context),
        format => bail!(
            "Unsupported Windows audio format: {format}. Change the device format in Windows Sound settings."
        ),
    }
}
struct Control {
    stop: Arc<AtomicBool>,
    active: Arc<AtomicBool>,
    level: Arc<AtomicU32>,
    clock: Arc<AudioClock>,
}
fn record(
    path: PathBuf,
    system: bool,
    mic: bool,
    overlay: Overlay,
    control: Control,
) -> Result<CaptureResult> {
    let Control {
        stop,
        active,
        level,
        clock,
    } = control;
    anyhow::ensure!(system || mic, "Choose meeting audio, microphone, or both");
    let host = cpal::default_host();
    let mut devices = vec![];
    if system {
        devices.push((host.default_output_device().context("No Windows default playback device. Select your meeting's speakers/headphones in Windows Sound settings.")?,true));
    }
    if mic {
        devices.push((
            host.default_input_device()
                .context("No microphone. Check Windows microphone access settings.")?,
            false,
        ));
    }
    let (sender, receiver) = mpsc::sync_channel(256);
    let dropped = Arc::new(AtomicU64::new(0));
    let errors = Arc::new(Mutex::new(None));
    let mut streams = vec![];
    for (index, (device, output)) in devices.iter().enumerate() {
        streams.push(input(
            device,
            *output,
            StreamContext {
                source: index,
                clock: clock.clone(),
                paused: overlay.pause.clone(),
                sender: sender.clone(),
                dropped: dropped.clone(),
                errors: errors.clone(),
                stop: stop.clone(),
            },
        )?);
    }
    let mut writer = hound::WavWriter::create(
        &path,
        hound::WavSpec {
            channels: 1,
            sample_rate: RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )?;
    for stream in &streams {
        stream.play()?;
    }
    anyhow::ensure!(overlay.start(), "Another recording is already active");
    active.store(true, Ordering::SeqCst);
    let mut mixer = Mixer::new(devices.len());
    let mut voice = crate::audio::VoiceDetector::new();
    let mut segmenter = crate::segments::Segmenter::new(
        path.parent().context("No session folder")?.join("chunks"),
        0,
    )?;
    let mut last_flush = Instant::now();
    let mut limit = false;
    let mut is_paused = false;
    loop {
        while let Ok(chunk) = receiver.try_recv() {
            mixer.push(chunk);
        }
        let requested = overlay.pause.load(Ordering::SeqCst);
        if requested != is_paused {
            clock.set_paused(requested);
            for stream in &streams {
                let result = if requested {
                    stream.pause().map_err(|e| e.to_string())
                } else {
                    stream.play().map_err(|e| e.to_string())
                };
                if let Err(error) = result {
                    if let Ok(mut e) = errors.lock() {
                        *e = Some(format!("Cannot pause/resume audio device: {error}"));
                    }
                    stop.store(true, Ordering::SeqCst);
                }
            }
            is_paused = requested;
        }
        overlay
            .elapsed_ms
            .store(clock.elapsed().as_millis() as u64, Ordering::Relaxed);
        let ending = stop.load(Ordering::SeqCst) || overlay.stop.load(Ordering::SeqCst);
        let wall = (clock.elapsed().as_secs_f64() * RATE as f64) as u64;
        let target = if ending || is_paused {
            wall
        } else {
            wall.saturating_sub(RATE as u64 / 5)
        }
        .min(1_000_000_000);
        while mixer.cursor < target {
            let frames = (target - mixer.cursor).min(1600) as usize;
            let samples = mixer.take(frames);
            let peak = samples
                .iter()
                .map(|s| (*s as f32 / i16::MAX as f32).abs())
                .fold(0.0, f32::max);
            level.store(peak.to_bits(), Ordering::Relaxed);
            overlay.voice.store(voice.push(&samples), Ordering::Relaxed);
            for sample in &samples {
                writer.write_sample(*sample)?;
            }
            segmenter.push(&samples)?;
        }
        if is_paused {
            level.store(0, Ordering::Relaxed);
            voice.pause();
            overlay.voice.store(false, Ordering::Relaxed);
        }
        if last_flush.elapsed() > Duration::from_secs(1) {
            writer.flush()?;
            last_flush = Instant::now();
        }
        // Keep WAV below 2 GiB, including headers, even on long unattended recordings.
        if mixer.cursor >= 1_000_000_000 {
            limit = true;
            stop.store(true, Ordering::SeqCst);
        }
        if ending {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    drop(streams);
    writer.finalize()?;
    let chunks = segmenter.finish()?;
    let mut warnings = vec![];
    if let Some(error) = errors.lock().ok().and_then(|e| e.clone()) {
        warnings.push(error);
    }
    if dropped.load(Ordering::Relaxed) > 0 {
        warnings
            .push("Some audio buffers were dropped because the computer could not keep up.".into());
    }
    if limit {
        warnings.push("Recording stopped at the WAV file-size limit.".into());
    }
    Ok(CaptureResult {
        chunks,
        warning: (!warnings.is_empty()).then(|| warnings.join(" ")),
    })
}
