use crate::audio::Resampler;
use anyhow::{Context, Result, bail};
use cpal::{
    FromSample, SampleFormat, SizedSample,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

pub struct Player {
    stop: Arc<AtomicBool>,
    pub finished: Arc<AtomicBool>,
    errors: mpsc::Receiver<String>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Player {
    pub fn start(path: PathBuf) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let (sender, errors) = mpsc::channel();
        let (a, b) = (stop.clone(), finished.clone());
        let worker = thread::spawn(move || {
            if let Err(e) = play(path, a) {
                let _ = sender.send(e.to_string());
            }
            b.store(true, Ordering::SeqCst);
        });
        Self {
            stop,
            finished,
            errors,
            worker: Some(worker),
        }
    }
    pub fn error(&self) -> Option<String> {
        self.errors.try_recv().ok()
    }
}
impl Drop for Player {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}
fn output<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    receiver: mpsc::Receiver<Vec<f32>>,
    stop: Arc<AtomicBool>,
) -> Result<cpal::Stream> {
    let channels = config.channels as usize;
    let mut current = vec![];
    let mut offset = 0;
    Ok(device.build_output_stream(
        config,
        move |data: &mut [T], _| {
            for frame in data.chunks_mut(channels) {
                if offset >= current.len() {
                    current = receiver.try_recv().unwrap_or_default();
                    offset = 0;
                }
                let sample = current.get(offset).copied().unwrap_or(0.0);
                offset += 1;
                for channel in frame {
                    *channel = T::from_sample(sample);
                }
            }
        },
        move |_| stop.store(true, Ordering::SeqCst),
        None,
    )?)
}
fn play(path: PathBuf, stop: Arc<AtomicBool>) -> Result<()> {
    let host = cpal::default_host();
    let device = host.default_output_device().context("No playback device")?;
    let supported = device.default_output_config()?;
    let config = supported.config();
    let (sender, receiver) = mpsc::sync_channel(8);
    let stream = match supported.sample_format() {
        SampleFormat::F32 => output::<f32>(&device, &config, receiver, stop.clone()),
        SampleFormat::F64 => output::<f64>(&device, &config, receiver, stop.clone()),
        SampleFormat::I16 => output::<i16>(&device, &config, receiver, stop.clone()),
        SampleFormat::I32 => output::<i32>(&device, &config, receiver, stop.clone()),
        format => bail!("Unsupported playback device format: {format}"),
    }?;
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    anyhow::ensure!(
        spec.channels == 1 && spec.bits_per_sample == 16,
        "Playback audio must be mono PCM16"
    );
    let mut resampler = Resampler::new(spec.sample_rate, config.sample_rate.0);
    let mut samples = reader.samples::<i16>();
    stream.play()?;
    while !stop.load(Ordering::SeqCst) {
        let chunk: Vec<_> = samples
            .by_ref()
            .take(1600)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if chunk.is_empty() {
            break;
        }
        let output = resampler.push(chunk.into_iter().map(|s| s as f32 / i16::MAX as f32));
        let mut pending = output;
        loop {
            if stop.load(Ordering::SeqCst) {
                return Ok(());
            }
            match sender.try_send(pending) {
                Ok(()) => break,
                Err(mpsc::TrySendError::Full(value)) => {
                    pending = value;
                    thread::sleep(Duration::from_millis(10));
                }
                Err(mpsc::TrySendError::Disconnected(_)) => return Ok(()),
            }
        }
    }
    // At most eight 100 ms chunks remain. Stop requests interrupt this drain.
    for _ in 0..90 {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    drop(stream);
    Ok(())
}
