//! Streaming audio primitives shared by Windows capture and playback.
use std::collections::VecDeque;
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct PauseTimeline {
    paused_at: Option<Duration>,
    total_paused: Duration,
    last_resume: Duration,
}
impl PauseTimeline {
    pub fn set_paused(&mut self, paused: bool, now: Duration) {
        match (paused, self.paused_at) {
            (true, None) => self.paused_at = Some(now),
            (false, Some(at)) => {
                self.total_paused += now.saturating_sub(at);
                self.paused_at = None;
                self.last_resume = now;
            }
            _ => {}
        }
    }
    pub fn elapsed(&self, now: Duration) -> Duration {
        self.paused_at
            .unwrap_or(now)
            .saturating_sub(self.total_paused)
    }
}
pub struct AudioClock {
    started: Instant,
    timeline: Mutex<PauseTimeline>,
}
impl AudioClock {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            timeline: Mutex::new(PauseTimeline::default()),
        }
    }
    pub fn elapsed(&self) -> Duration {
        self.timeline
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .elapsed(self.started.elapsed())
    }
    pub fn set_paused(&self, paused: bool) {
        self.timeline
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .set_paused(paused, self.started.elapsed());
    }
    pub fn accepts(&self, latency: Duration) -> bool {
        self.started.elapsed().saturating_sub(latency)
            >= self
                .timeline
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .last_resume
    }
}

pub struct Resampler {
    step: f64,
    position: f64,
    pending: VecDeque<f32>,
}
impl Resampler {
    pub fn new(input_rate: u32, output_rate: u32) -> Self {
        assert!(input_rate > 0 && output_rate > 0);
        Self {
            step: input_rate as f64 / output_rate as f64,
            position: 0.0,
            pending: VecDeque::new(),
        }
    }
    pub fn push(&mut self, input: impl IntoIterator<Item = f32>) -> Vec<f32> {
        self.pending.extend(input);
        let mut result = vec![];
        while self.position + 1.0 < self.pending.len() as f64 {
            let i = self.position.floor() as usize;
            let f = self.position.fract() as f32;
            result.push(self.pending[i] * (1.0 - f) + self.pending[i + 1] * f);
            self.position += self.step;
        }
        let consumed = (self.position.floor() as usize).min(self.pending.len().saturating_sub(1));
        self.pending.drain(..consumed);
        self.position -= consumed as f64;
        result
    }
}
pub struct Chunk {
    pub source: usize,
    pub start: u64,
    pub samples: Vec<f32>,
}
pub struct Mixer {
    pub cursor: u64,
    sources: Vec<VecDeque<Chunk>>,
}
impl Mixer {
    pub fn new(sources: usize) -> Self {
        assert!(sources > 0);
        Self {
            cursor: 0,
            sources: (0..sources).map(|_| VecDeque::new()).collect(),
        }
    }
    pub fn push(&mut self, chunk: Chunk) {
        if let Some(source) = self.sources.get_mut(chunk.source) {
            source.push_back(chunk);
        }
    }
    pub fn take(&mut self, frames: usize) -> Vec<i16> {
        let end = self.cursor + frames as u64;
        let mut mixed = vec![0f32; frames];
        for source in &mut self.sources {
            while source
                .front()
                .is_some_and(|c| c.start + c.samples.len() as u64 <= self.cursor)
            {
                source.pop_front();
            }
            for chunk in source.iter() {
                if chunk.start >= end {
                    break;
                }
                let from = chunk.start.max(self.cursor);
                let to = (chunk.start + chunk.samples.len() as u64).min(end);
                for position in from..to {
                    mixed[(position - self.cursor) as usize] +=
                        chunk.samples[(position - chunk.start) as usize];
                }
            }
        }
        let gain = 1.0 / self.sources.len() as f32;
        self.cursor = end;
        mixed
            .into_iter()
            .map(|x| (x * gain).clamp(-1.0, 1.0).mul_add(i16::MAX as f32, 0.0) as i16)
            .collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resampling_is_independent_of_callback_boundaries() {
        let input: Vec<_> = (0..4800).map(|i| (i as f32 * 0.1).sin()).collect();
        let expected = Resampler::new(48000, 16000).push(input.clone());
        let mut streaming = Resampler::new(48000, 16000);
        let mut actual = vec![];
        for chunk in input.chunks(137) {
            actual.extend(streaming.push(chunk.iter().copied()));
        }
        assert_eq!(actual, expected);
        assert_eq!(actual.len(), 1600);
    }
    #[test]
    fn mixing_preserves_alignment_silence_and_both_sources() {
        let mut mix = Mixer::new(2);
        mix.push(Chunk {
            source: 0,
            start: 2,
            samples: vec![0.8, 0.8, 0.8],
        });
        mix.push(Chunk {
            source: 1,
            start: 3,
            samples: vec![0.4, 0.4],
        });
        let out = mix.take(4);
        assert_eq!(out[0], 0);
        assert_eq!(out[1], 0);
        assert!((out[2] as i32 - 13106).abs() < 2);
        assert!((out[3] as i32 - 19660).abs() < 2);
        assert!((mix.take(1)[0] as i32 - 19660).abs() < 2);
        assert_eq!(mix.take(2), vec![0, 0]);
    }
    #[test]
    fn pause_resume_keeps_one_timeline_and_excludes_all_paused_time() {
        let mut clock = PauseTimeline::default();
        clock.set_paused(true, Duration::from_secs(5));
        assert_eq!(
            clock.elapsed(Duration::from_secs(50)),
            Duration::from_secs(5)
        );
        clock.set_paused(true, Duration::from_secs(10));
        clock.set_paused(false, Duration::from_secs(50));
        assert_eq!(
            clock.elapsed(Duration::from_secs(55)),
            Duration::from_secs(10)
        );
        clock.set_paused(true, Duration::from_secs(56));
        assert_eq!(
            clock.elapsed(Duration::from_secs(70)),
            Duration::from_secs(11)
        );
        clock.set_paused(false, Duration::from_secs(76));
        assert_eq!(
            clock.elapsed(Duration::from_secs(80)),
            Duration::from_secs(15)
        );
        clock.set_paused(false, Duration::from_secs(81));
        assert_eq!(
            clock.elapsed(Duration::from_secs(81)),
            Duration::from_secs(16)
        );
    }
}
