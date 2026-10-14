//! Durable, bounded one-minute PCM segments, independent of UI rendering.
use anyhow::Result;
use std::path::{Path, PathBuf};
pub const RATE: u32 = 16000;
pub const MINUTE_SAMPLES: usize = RATE as usize * 60;
pub struct Segmenter {
    dir: PathBuf,
    pending: Vec<i16>,
    pub count: u64,
}
impl Segmenter {
    pub fn new(dir: PathBuf, count: u64) -> Result<Self> {
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            pending: Vec::with_capacity(MINUTE_SAMPLES),
            count,
        })
    }
    pub fn push(&mut self, mut samples: &[i16]) -> Result<()> {
        while !samples.is_empty() {
            let take = (MINUTE_SAMPLES - self.pending.len()).min(samples.len());
            self.pending.extend_from_slice(&samples[..take]);
            samples = &samples[take..];
            if self.pending.len() == MINUTE_SAMPLES {
                self.flush()?;
            }
        }
        Ok(())
    }
    pub fn finish(&mut self) -> Result<u64> {
        self.flush()?;
        Ok(self.count)
    }
    fn flush(&mut self) -> Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let index = self.count + 1;
        write(&self.dir, index, &self.pending)?;
        self.count = index;
        self.pending.clear();
        Ok(())
    }
}
pub fn path(dir: &Path, index: u64) -> PathBuf {
    dir.join(format!("minute-{index:06}.wav"))
}
pub fn write(dir: &Path, index: u64, samples: &[i16]) -> Result<()> {
    let destination = path(dir, index);
    let temporary = destination.with_extension("tmp");
    let mut writer = hound::WavWriter::create(
        &temporary,
        hound::WavSpec {
            channels: 1,
            sample_rate: RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )?;
    for sample in samples {
        writer.write_sample(*sample)?;
    }
    writer.finalize()?;
    std::fs::rename(temporary, destination)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_minutes_and_final_tail_preserve_exact_samples() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let mut split = Segmenter::new(root.clone(), 0).unwrap();
        let input: Vec<i16> = (0..MINUTE_SAMPLES * 2 + 723)
            .map(|n| (n % 32000) as i16)
            .collect();
        for chunk in input.chunks(1600) {
            split.push(chunk).unwrap();
        }
        assert_eq!(split.count, 2);
        assert_eq!(split.finish().unwrap(), 3);
        assert_eq!(split.finish().unwrap(), 3);
        let mut output = Vec::new();
        for index in 1..=3 {
            output.extend(
                hound::WavReader::open(path(&root, index))
                    .unwrap()
                    .samples::<i16>()
                    .map(Result::unwrap),
            );
        }
        assert_eq!(input, output);
        std::fs::remove_dir_all(root).unwrap();
    }
}
