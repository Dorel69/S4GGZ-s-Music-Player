//! Playback-driven light metering. The audio thread does fixed-size arithmetic
//! and publishes at 100 Hz; the UI never decodes files or locks the audio thread.
use rodio::{source::SeekError, ChannelCount, SampleRate, Source};
use std::{
    sync::{
        atomic::{AtomicU32, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AudioReactive {
    pub energy: f32,
    pub beat: f32,
    pub beat_count: u64,
    pub tempo_bpm: f32,
}

#[derive(Default)]
pub(crate) struct AudioMeter {
    energy: AtomicU32,
    beat: AtomicU32,
    beat_count: AtomicU64,
    tempo: AtomicU32,
}

impl AudioMeter {
    fn publish(&self, value: AudioReactive) {
        self.energy.store(value.energy.to_bits(), Ordering::Relaxed);
        self.beat.store(value.beat.to_bits(), Ordering::Relaxed);
        self.tempo
            .store(value.tempo_bpm.to_bits(), Ordering::Relaxed);
        self.beat_count.store(value.beat_count, Ordering::Relaxed);
    }

    pub(crate) fn snapshot(&self) -> AudioReactive {
        AudioReactive {
            energy: f32::from_bits(self.energy.load(Ordering::Relaxed)),
            beat: f32::from_bits(self.beat.load(Ordering::Relaxed)),
            beat_count: self.beat_count.load(Ordering::Relaxed),
            tempo_bpm: f32::from_bits(self.tempo.load(Ordering::Relaxed)),
        }
    }
}

#[derive(Default)]
struct Envelope {
    value: AudioReactive,
    rms_average: f32,
    bass_average: f32,
    previous_rms: f32,
    previous_bass: f32,
    elapsed: f64,
    last_beat: Option<f64>,
    interval: Option<f32>,
}

impl Envelope {
    fn update(&mut self, rms: f32, bass: f32, dt: f32) -> AudioReactive {
        self.elapsed += f64::from(dt);
        let target = (rms * 3.8).clamp(0.0, 1.0);
        let smoothing = if target > self.value.energy {
            0.07
        } else {
            0.45
        };
        self.value.energy += (target - self.value.energy) * (1.0 - (-dt / smoothing).exp());
        self.value.beat *= (-dt / 0.16).exp();

        // Detect rising attacks against an adaptive local baseline. Bass catches
        // kick drums; broadband attacks retain acoustic/percussive material.
        let bass_attack =
            bass > self.bass_average * 1.32 + 0.004 && bass > self.previous_bass * 1.035 + 0.001;
        let wide_attack =
            rms > self.rms_average * 1.45 + 0.008 && rms > self.previous_rms * 1.08 + 0.002;
        let since = self.last_beat.map(|last| (self.elapsed - last) as f32);
        if rms > 0.006 && (bass_attack || wide_attack) && since.is_none_or(|gap| gap >= 0.20) {
            self.value.beat_count = self.value.beat_count.wrapping_add(1);
            self.value.beat = (0.5 + (rms - self.rms_average).max(0.0) * 3.0).clamp(0.5, 1.0);
            if let Some(gap) = since.filter(|gap| (0.25..=2.0).contains(gap)) {
                // Smooth plausible consecutive inter-onset intervals, allowing
                // a new section's faster/slower beat to take over gradually.
                let interval = self.interval.map_or(gap, |old| old * 0.7 + gap * 0.3);
                self.interval = Some(interval);
                self.value.tempo_bpm = (60.0 / interval).clamp(30.0, 240.0);
            }
            self.last_beat = Some(self.elapsed);
        }
        let baseline = 1.0 - (-dt / 0.6).exp();
        self.rms_average += (rms - self.rms_average) * baseline;
        self.bass_average += (bass - self.bass_average) * baseline;
        self.previous_rms = rms;
        self.previous_bass = bass;
        if rms < 0.0005 {
            self.value.beat = 0.0;
        }
        if self.last_beat.is_some_and(|last| self.elapsed - last > 3.0) {
            self.interval = None;
            self.value.tempo_bpm = 0.0;
        }
        self.value
    }
}

pub(crate) struct AnalysisSource<S> {
    input: S,
    meter: Arc<AudioMeter>,
    envelope: Envelope,
    channels: usize,
    sample_rate: u32,
    window_samples: usize,
    samples: usize,
    squares: f64,
    bass_squares: f64,
    // Independent channel filters preserve energy in opposite-phase stereo.
    bass: [f32; 32],
    bass_alpha: f32,
}

impl<S: Source> AnalysisSource<S> {
    pub(crate) fn new(input: S, meter: Arc<AudioMeter>) -> Self {
        let mut source = Self {
            input,
            meter,
            envelope: Envelope::default(),
            channels: 1,
            sample_rate: 1,
            window_samples: 1,
            samples: 0,
            squares: 0.0,
            bass_squares: 0.0,
            bass: [0.0; 32],
            bass_alpha: 1.0,
        };
        source.refresh_format();
        source
    }

    fn refresh_format(&mut self) {
        self.channels = usize::from(self.input.channels()).max(1);
        self.sample_rate = self.input.sample_rate().max(1);
        self.window_samples = (self.sample_rate as usize / 100).max(1) * self.channels;
        self.bass_alpha = 1.0 - (-std::f32::consts::TAU * 180.0 / self.sample_rate as f32).exp();
    }

    fn reset(&mut self) {
        self.envelope = Envelope::default();
        self.samples = 0;
        self.squares = 0.0;
        self.bass_squares = 0.0;
        self.bass.fill(0.0);
        self.meter.publish(AudioReactive::default());
        self.refresh_format();
    }
}

impl<S: Source> Iterator for AnalysisSource<S> {
    type Item = f32;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let Some(sample) = self.input.next() else {
            self.meter.publish(AudioReactive::default());
            return None;
        };
        // A corrupt decoder sample must not poison the light envelope. Playback
        // samples themselves pass through unchanged.
        let level = if sample.is_finite() {
            sample.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        let channel = (self.samples % self.channels) % self.bass.len();
        self.bass[channel] += self.bass_alpha * (level - self.bass[channel]);
        self.squares += f64::from(level * level);
        self.bass_squares += f64::from(self.bass[channel] * self.bass[channel]);
        self.samples += 1;
        if self.samples >= self.window_samples {
            let count = self.samples as f64;
            let rms = (self.squares / count).sqrt() as f32;
            let bass = (self.bass_squares / count).sqrt() as f32;
            let dt = self.samples as f32 / self.channels as f32 / self.sample_rate as f32;
            self.meter.publish(self.envelope.update(rms, bass, dt));
            self.samples = 0;
            self.squares = 0.0;
            self.bass_squares = 0.0;
            self.refresh_format();
        }
        Some(sample)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.input.size_hint()
    }
}

impl<S: Source> Source for AnalysisSource<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.input.current_span_len()
    }
    fn channels(&self) -> ChannelCount {
        self.input.channels()
    }
    fn sample_rate(&self) -> SampleRate {
        self.input.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        self.input.total_duration()
    }

    fn try_seek(&mut self, position: Duration) -> Result<(), SeekError> {
        self.input.try_seek(position)?;
        self.reset();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::buffer::SamplesBuffer;

    fn synthetic_track(amplitude: f32, interval: usize) -> AudioReactive {
        let mut envelope = Envelope::default();
        for window in 0..800 {
            let age = window % interval;
            let rms = amplitude
                * if age < 7 {
                    1.0 - age as f32 / 9.0
                } else {
                    0.05
                };
            envelope.update(rms, rms * 0.8, 0.01);
        }
        envelope.value
    }

    #[test]
    fn energy_and_measured_tempo_distinguish_calm_and_fast_music() {
        let calm = synthetic_track(0.055, 100);
        let energetic = synthetic_track(0.4, 40);
        assert!(
            energetic.energy > calm.energy * 2.5,
            "{calm:?} {energetic:?}"
        );
        assert!((58.0..62.0).contains(&calm.tempo_bpm), "{calm:?}");
        assert!(
            (147.0..153.0).contains(&energetic.tempo_bpm),
            "{energetic:?}"
        );
        assert!(energetic.beat_count > calm.beat_count * 2);
    }

    #[test]
    fn silence_does_not_generate_beats_and_resets_stale_tempo() {
        let mut envelope = Envelope::default();
        envelope.update(0.3, 0.2, 0.01);
        let beats = envelope.value.beat_count;
        for _ in 0..800 {
            envelope.update(0.0, 0.0, 0.01);
        }
        assert_eq!(envelope.value.beat_count, beats);
        assert!(envelope.value.energy < 0.0001);
        assert_eq!(envelope.value.beat, 0.0);
        assert_eq!(envelope.value.tempo_bpm, 0.0);
    }

    #[test]
    fn tap_preserves_opposite_phase_stereo_and_forwards_seek() {
        let samples: Vec<f32> = (0..4000)
            .map(|index| if index % 2 == 0 { 0.2 } else { -0.2 })
            .collect();
        let meter = Arc::new(AudioMeter::default());
        let mut source = AnalysisSource::new(SamplesBuffer::new(2, 1000, samples), meter.clone());
        assert_eq!(source.channels(), 2);
        assert_eq!(source.sample_rate(), 1000);
        assert_eq!(source.total_duration(), Some(Duration::from_secs(2)));
        for _ in 0..400 {
            assert!(source.next().is_some());
        }
        assert!(meter.snapshot().energy > 0.6);
        source.try_seek(Duration::ZERO).unwrap();
        assert_eq!(meter.snapshot(), AudioReactive::default());
        assert_eq!(source.next(), Some(0.2));
        assert_eq!(source.next(), Some(-0.2));
        for _ in &mut source {}
        assert_eq!(meter.snapshot(), AudioReactive::default());
    }
}
