//! Ten cascaded RBJ biquads (https://www.w3.org/TR/audio-eq-cookbook/).
//! UI settings are copied with try_lock once per 128 frames. The sample loop
//! never blocks, allocates, performs file I/O, or sends a message. Coefficients
//! are calculated only on changes and ramped over 30 ms, once per audio frame,
//! so all channels see the same filter and gain throughout a stereo frame.
use rodio::{source::SeekError, ChannelCount, SampleRate, Source};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

pub const BAND_COUNT: usize = 10;
pub const FREQUENCIES: [f32; BAND_COUNT] = [
    60., 170., 310., 600., 1000., 3000., 6000., 12000., 14000., 16000.,
];
const MAX_CHANNELS: usize = 32;
const UPDATE_FRAMES: usize = 128;
const RAMP_SECONDS: f64 = 0.030;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterType {
    #[default]
    Peaking,
    LowShelf,
    HighShelf,
}

impl FilterType {
    pub const ALL: [Self; 3] = [Self::Peaking, Self::LowShelf, Self::HighShelf];
    pub fn label(self) -> &'static str {
        match self {
            Self::Peaking => "Peaking",
            Self::LowShelf => "Low shelf",
            Self::HighShelf => "High shelf",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FilterBand {
    pub frequency: f32,
    pub q: f32,
    pub gain_db: f32,
    pub filter_type: FilterType,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Equalizer {
    pub enabled: bool,
    pub preamp_db: f32,
    pub bands: [FilterBand; BAND_COUNT],
}

impl Default for Equalizer {
    fn default() -> Self {
        Self {
            enabled: true,
            preamp_db: 0.,
            bands: FREQUENCIES.map(|frequency| FilterBand {
                frequency,
                q: 0.707,
                gain_db: 0.,
                filter_type: FilterType::Peaking,
            }),
        }
    }
}

impl Equalizer {
    /// Reject non-finite or out-of-range user data before publishing to audio.
    /// f0 is additionally limited below Nyquist for the actual source rate.
    pub fn validate(&self) -> Result<(), String> {
        if !self.preamp_db.is_finite() || !(-36.0..=12.0).contains(&self.preamp_db) {
            return Err("Preamp trebuie să fie între -36 și +12 dB".into());
        }
        for (index, band) in self.bands.iter().enumerate() {
            if !band.frequency.is_finite()
                || !(20.0..=20000.0).contains(&band.frequency)
                || !band.q.is_finite()
                || !(0.2..=10.0).contains(&band.q)
                || !band.gain_db.is_finite()
                || !(-12.0..=12.0).contains(&band.gain_db)
            {
                return Err(format!(
                    "Banda {}: frecvență 20–20000 Hz, Q 0,2–10, gain -12…+12 dB",
                    index + 1
                ));
            }
        }
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        // Bound input size before allocating a file buffer.
        use std::io::Read;
        let file = fs::File::open(path).map_err(|e| format!("Preset inaccesibil: {e}"))?;
        let mut bytes = Vec::new();
        file.take(65537)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 65536 {
            return Err("Fișierul preset este prea mare (maximum 64 KiB)".into());
        }
        let settings: Self =
            serde_json::from_slice(&bytes).map_err(|e| format!("Preset JSON invalid: {e}"))?;
        settings.validate()?;
        Ok(settings)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        fs::write(&temporary, bytes).map_err(|e| format!("Nu am putut salva presetul: {e}"))?;
        if let Err(error) = fs::rename(&temporary, path) {
            let _ = fs::remove_file(&temporary);
            return Err(format!("Nu am putut salva presetul: {error}"));
        }
        Ok(())
    }

    /// Conservative UI starting point; resonant shelves and transients can
    /// exceed this estimate, so the final linked peak guard is still required.
    pub fn suggested_preamp(&self) -> f32 {
        -self
            .bands
            .iter()
            .map(|b| b.gain_db.max(0.))
            .sum::<f32>()
            .min(36.)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Flat,
    BassBoost,
    TrebleBoost,
    Vocal,
    Rock,
}
impl Preset {
    pub const ALL: [Self; 5] = [
        Self::Flat,
        Self::BassBoost,
        Self::TrebleBoost,
        Self::Vocal,
        Self::Rock,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Flat => "Flat",
            Self::BassBoost => "Bass Boost",
            Self::TrebleBoost => "Treble Boost",
            Self::Vocal => "Vocal",
            Self::Rock => "Rock",
        }
    }
    pub fn settings(self) -> Equalizer {
        let gains = match self {
            Self::Flat => [0.; BAND_COUNT],
            Self::BassBoost => [6., 4., 2., 0., 0., 0., 0., 0., 0., 0.],
            Self::TrebleBoost => [0., 0., 0., 0., 0., 1., 3., 4., 3., 2.],
            Self::Vocal => [-2., -2., -1., 1., 3., 4., 1., 0., -1., -2.],
            Self::Rock => [4., 3., 0., -2., -1., 2., 3., 2., 1., 1.],
        };
        let mut settings = Equalizer::default();
        for (band, gain) in settings.bands.iter_mut().zip(gains) {
            band.gain_db = gain;
        }
        settings.preamp_db = settings.suggested_preamp();
        settings
    }
}

pub type SharedEqualizer = Arc<Mutex<Equalizer>>;

// Normalized [b0, b1, b2, a1, a2]. f64 prevents cancellation at low f0/high Q.
#[derive(Clone, Copy, Debug)]
struct Coefficients([f64; 5]);
impl Coefficients {
    const IDENTITY: Self = Self([1., 0., 0., 0., 0.]);
    fn for_band(band: FilterBand, sample_rate: u32) -> Self {
        if band.gain_db == 0. {
            return Self::IDENTITY;
        }
        let frequency = f64::from(band.frequency).min(f64::from(sample_rate) * 0.45);
        let w = std::f64::consts::TAU * frequency / f64::from(sample_rate);
        let (sin, cos) = w.sin_cos();
        let a = 10_f64.powf(f64::from(band.gain_db) / 40.);
        let alpha = sin / (2. * f64::from(band.q));
        let beta = 2. * a.sqrt() * alpha;
        // Shelf filters use the cookbook's Q definition (not shelf slope S).
        let (b0, b1, b2, a0, a1, a2) = match band.filter_type {
            FilterType::Peaking => (
                1. + alpha * a,
                -2. * cos,
                1. - alpha * a,
                1. + alpha / a,
                -2. * cos,
                1. - alpha / a,
            ),
            FilterType::LowShelf => (
                a * ((a + 1.) - (a - 1.) * cos + beta),
                2. * a * ((a - 1.) - (a + 1.) * cos),
                a * ((a + 1.) - (a - 1.) * cos - beta),
                (a + 1.) + (a - 1.) * cos + beta,
                -2. * ((a - 1.) + (a + 1.) * cos),
                (a + 1.) + (a - 1.) * cos - beta,
            ),
            FilterType::HighShelf => (
                a * ((a + 1.) + (a - 1.) * cos + beta),
                -2. * a * ((a - 1.) + (a + 1.) * cos),
                a * ((a + 1.) + (a - 1.) * cos - beta),
                (a + 1.) - (a - 1.) * cos + beta,
                2. * ((a - 1.) - (a + 1.) * cos),
                (a + 1.) - (a - 1.) * cos - beta,
            ),
        };
        Self([b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0])
    }
}

#[derive(Clone, Copy, Default)]
struct History {
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}
impl History {
    fn process(&mut self, input: f64, coefficients: Coefficients) -> f64 {
        let [b0, b1, b2, a1, a2] = coefficients.0;
        // Direct form I retains physically meaningful sample histories when
        // coefficients change, unlike a transposed form's coefficient-scaled state.
        let output = b0 * input + b1 * self.x1 + b2 * self.x2 - a1 * self.y1 - a2 * self.y2;
        let output = if output.is_finite() && output.abs() >= 1e-24 {
            output
        } else {
            0.
        };
        self.x2 = self.x1;
        self.x1 = input;
        self.y2 = self.y1;
        self.y1 = output;
        output
    }
}

pub struct EqualizerSource<S> {
    input: S,
    shared: SharedEqualizer,
    settings: Equalizer,
    coefficients: [Coefficients; BAND_COUNT],
    target: [Coefficients; BAND_COUNT],
    step: [Coefficients; BAND_COUNT],
    history: [[History; BAND_COUNT]; MAX_CHANNELS],
    gain: f64,
    gain_target: f64,
    gain_step: f64,
    ramp_remaining: usize,
    update_in: usize,
    channels: usize,
    sample_rate: u32,
    span_remaining: Option<usize>,
    frame: [f64; MAX_CHANNELS],
    frame_cursor: usize,
    frame_len: usize,
    limiter_gain: f64,
    limiter_release: f64,
}

impl<S: Source> EqualizerSource<S> {
    pub fn new(input: S, shared: SharedEqualizer) -> Result<Self, String> {
        let channels = usize::from(input.channels());
        let sample_rate = input.sample_rate();
        if channels == 0 || channels > MAX_CHANNELS || sample_rate == 0 {
            return Err("Format audio incompatibil cu egalizatorul (1–32 canale)".into());
        }
        let settings = *shared
            .lock()
            .map_err(|_| "Starea egalizatorului nu este disponibilă")?;
        settings.validate()?;
        let mut source = Self {
            input,
            shared,
            settings,
            coefficients: [Coefficients::IDENTITY; BAND_COUNT],
            target: [Coefficients::IDENTITY; BAND_COUNT],
            step: [Coefficients([0.; 5]); BAND_COUNT],
            history: [[History::default(); BAND_COUNT]; MAX_CHANNELS],
            gain: 1.,
            gain_target: 1.,
            gain_step: 0.,
            ramp_remaining: 0,
            update_in: 0,
            channels,
            sample_rate,
            span_remaining: None,
            frame: [0.; MAX_CHANNELS],
            frame_cursor: 0,
            frame_len: 0,
            limiter_gain: 1.,
            limiter_release: 1. - (-1. / (0.05 * f64::from(sample_rate))).exp(),
        };
        source.span_remaining = source.input.current_span_len();
        source.set_target(settings, false);
        Ok(source)
    }

    fn set_target(&mut self, settings: Equalizer, smooth: bool) {
        self.settings = settings;
        self.target = settings.bands.map(|band| {
            if settings.enabled {
                Coefficients::for_band(band, self.sample_rate)
            } else {
                Coefficients::IDENTITY
            }
        });
        self.gain_target = if settings.enabled {
            10_f64.powf(f64::from(settings.preamp_db) / 20.)
        } else {
            1.
        };
        self.ramp_remaining = if smooth {
            (f64::from(self.sample_rate) * RAMP_SECONDS).round().max(1.) as usize
        } else {
            0
        };
        if smooth {
            for band in 0..BAND_COUNT {
                for coefficient in 0..5 {
                    self.step[band].0[coefficient] = (self.target[band].0[coefficient]
                        - self.coefficients[band].0[coefficient])
                        / self.ramp_remaining as f64;
                }
            }
            self.gain_step = (self.gain_target - self.gain) / self.ramp_remaining as f64;
        } else {
            self.coefficients = self.target;
            self.gain = self.gain_target;
        }
    }

    fn next_frame(&mut self) -> Option<()> {
        if self.span_remaining == Some(0) {
            let rate = self.input.sample_rate().max(1);
            let channels = usize::from(self.input.channels()).max(1);
            if rate != self.sample_rate || channels != self.channels {
                // Standard decoders keep a fixed format; handle source span
                // changes too, with no reallocations or shared channel histories.
                self.channels = channels;
                self.sample_rate = rate;
                self.history.fill([History::default(); BAND_COUNT]);
                self.limiter_gain = 1.;
                self.limiter_release = 1. - (-1. / (0.05 * f64::from(rate))).exp();
                self.set_target(self.settings, false);
            }
            self.span_remaining = self.input.current_span_len();
        }
        // An exotic dynamically changing source with >32 channels is passed
        // through safely, rather than aliasing histories or allocating on audio.
        if self.channels > MAX_CHANNELS {
            let value = self.input.next()?;
            self.frame[0] = if value.is_finite() {
                f64::from(value.clamp(-1., 1.))
            } else {
                0.
            };
            self.frame_len = 1;
            if let Some(remaining) = &mut self.span_remaining {
                *remaining = remaining.saturating_sub(1);
            }
            return Some(());
        }
        if self.update_in == 0 {
            // A busy/poisoned settings mutex leaves the previous sound intact.
            // Copy then drop the guard before doing coefficient arithmetic.
            let update = self.shared.try_lock().ok().map(|settings| *settings);
            if let Some(settings) = update.filter(|settings| *settings != self.settings) {
                self.set_target(settings, true);
            }
            self.update_in = UPDATE_FRAMES;
        }
        self.update_in -= 1;
        if self.ramp_remaining > 0 {
            self.ramp_remaining -= 1;
            for band in 0..BAND_COUNT {
                for coefficient in 0..5 {
                    self.coefficients[band].0[coefficient] += self.step[band].0[coefficient];
                }
            }
            self.gain += self.gain_step;
            if self.ramp_remaining == 0 {
                self.coefficients = self.target;
                self.gain = self.gain_target;
            }
        }
        let mut peak = 0_f64;
        self.frame_len = 0;
        for channel in 0..self.channels {
            let Some(sample) = self.input.next() else {
                break;
            };
            if let Some(remaining) = &mut self.span_remaining {
                *remaining = remaining.saturating_sub(1);
            }
            let mut value = if sample.is_finite() {
                f64::from(sample) * self.gain
            } else {
                0.
            };
            for band in 0..BAND_COUNT {
                value = self.history[channel][band].process(value, self.coefficients[band]);
            }
            peak = peak.max(value.abs());
            self.frame[channel] = value;
            self.frame_len += 1;
        }
        if self.frame_len == 0 {
            return None;
        }
        // Zero-lookahead peak protection: immediate linked attack, 50 ms
        // release. Both stereo channels receive the same gain, preserving pan.
        // It is a final guard, not a substitute for sensible preamp/headroom.
        let required = if peak > 1. { 1. / peak } else { 1. };
        self.limiter_gain =
            required.min(self.limiter_gain + (1. - self.limiter_gain) * self.limiter_release);
        for value in &mut self.frame[..self.frame_len] {
            *value = (*value * self.limiter_gain).clamp(-1., 1.);
        }
        Some(())
    }
}

impl<S: Source> Iterator for EqualizerSource<S> {
    type Item = f32;
    #[inline]
    fn next(&mut self) -> Option<f32> {
        if self.frame_cursor == self.frame_len {
            self.next_frame()?;
            self.frame_cursor = 0;
        }
        let value = self.frame[self.frame_cursor];
        self.frame_cursor += 1;
        Some(value as f32)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let buffered = self.frame_len - self.frame_cursor;
        let (low, high) = self.input.size_hint();
        (
            low.saturating_add(buffered),
            high.map(|n| n.saturating_add(buffered)),
        )
    }
}

impl<S: Source> Source for EqualizerSource<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.span_remaining
            .map(|remaining| remaining + self.frame_len - self.frame_cursor)
    }
    fn channels(&self) -> ChannelCount {
        self.channels as ChannelCount
    }
    fn sample_rate(&self) -> SampleRate {
        self.sample_rate
    }
    fn total_duration(&self) -> Option<Duration> {
        self.input.total_duration()
    }
    fn try_seek(&mut self, position: Duration) -> Result<(), SeekError> {
        self.input.try_seek(position)?;
        self.history.fill([History::default(); BAND_COUNT]);
        self.frame_cursor = 0;
        self.frame_len = 0;
        self.span_remaining = Some(0);
        self.update_in = 0;
        self.limiter_gain = 1.;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::buffer::SamplesBuffer;

    fn control(settings: Equalizer) -> SharedEqualizer {
        Arc::new(Mutex::new(settings))
    }
    fn tone(rate: u32, frequency: f32, amplitude: f32, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|n| (std::f32::consts::TAU * frequency * n as f32 / rate as f32).sin() * amplitude)
            .collect()
    }
    fn measured_gain(kind: FilterType, frequency: f32, gain_db: f32, rate: u32) -> f64 {
        let mut settings = Equalizer::default();
        settings.bands[4] = FilterBand {
            frequency: 1000.,
            q: 0.707,
            gain_db,
            filter_type: kind,
        };
        let samples = tone(rate, frequency, 0.01, rate as usize);
        let source = SamplesBuffer::new(1, rate, samples.clone());
        let filtered: Vec<_> = EqualizerSource::new(source, control(settings))
            .unwrap()
            .collect();
        let energy = |values: &[f32]| {
            values[rate as usize / 2..]
                .iter()
                .map(|v| f64::from(*v).powi(2))
                .sum::<f64>()
        };
        10. * (energy(&filtered) / energy(&samples)).log10()
    }

    #[test]
    fn cookbook_response_matches_peak_and_shelf_gains() {
        for rate in [44100, 48000, 96000] {
            for gain in [-12., -6., 6., 12.] {
                let measured = measured_gain(FilterType::Peaking, 1000., gain, rate);
                assert!(
                    (measured - f64::from(gain)).abs() < 0.03,
                    "{rate}: {gain} vs {measured}"
                );
            }
            let low = measured_gain(FilterType::LowShelf, 40., 6., rate);
            let high = measured_gain(FilterType::HighShelf, 10000., 6., rate);
            assert!((low - 6.).abs() < 0.05, "{low}");
            assert!((high - 6.).abs() < 0.05, "{high}");
            assert!(measured_gain(FilterType::LowShelf, 10000., 6., rate).abs() < 0.05);
            assert!(measured_gain(FilterType::HighShelf, 40., 6., rate).abs() < 0.05);
        }
    }

    #[test]
    fn flat_is_exact_and_stereo_channels_have_independent_history() {
        let input = tone(48000, 1000., 0.1, 4800);
        let flat: Vec<_> = EqualizerSource::new(
            SamplesBuffer::new(1, 48000, input.clone()),
            control(Equalizer::default()),
        )
        .unwrap()
        .collect();
        assert_eq!(input, flat);
        let stereo: Vec<_> = input.iter().flat_map(|sample| [*sample, 0.]).collect();
        let mut settings = Equalizer::default();
        settings.bands[4].gain_db = 12.;
        let filtered: Vec<_> =
            EqualizerSource::new(SamplesBuffer::new(2, 48000, stereo), control(settings))
                .unwrap()
                .collect();
        assert!(filtered
            .as_chunks::<2>()
            .0
            .iter()
            .all(|frame| frame[1] == 0.));
        assert!(filtered
            .as_chunks::<2>()
            .0
            .iter()
            .skip(500)
            .any(|frame| frame[0].abs() > 0.3));
    }

    #[test]
    fn linked_peak_guard_preserves_stereo_balance_and_rejects_nonfinite_samples() {
        let settings = Equalizer {
            preamp_db: 12.,
            ..Equalizer::default()
        };
        let input: Vec<_> = (0..1000).flat_map(|_| [0.9, 0.45]).collect();
        let filtered: Vec<_> =
            EqualizerSource::new(SamplesBuffer::new(2, 48000, input), control(settings))
                .unwrap()
                .collect();
        for frame in filtered.as_chunks::<2>().0 {
            assert!(frame[0] <= 1. && frame[0] > 0.99);
            assert!((frame[1] / frame[0] - 0.5).abs() < 1e-6);
        }
        let input = vec![f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.2, f32::MAX];
        let output: Vec<_> = EqualizerSource::new(
            SamplesBuffer::new(1, 48000, input),
            control(Equalizer::default()),
        )
        .unwrap()
        .collect();
        assert!(output
            .iter()
            .all(|value| value.is_finite() && value.abs() <= 1.));
        assert_eq!(&output[..3], &[0.; 3]);
    }

    #[test]
    fn live_edits_are_smoothed_and_a_busy_ui_never_blocks_audio() {
        let shared = control(Equalizer::default());
        let input = SamplesBuffer::new(
            2,
            48000,
            tone(48000, 500., 0.02, 48000)
                .into_iter()
                .flat_map(|s| [s, s])
                .collect::<Vec<_>>(),
        );
        let mut source = EqualizerSource::new(input, shared.clone()).unwrap();
        for _ in 0..2048 {
            source.next().unwrap();
        }
        {
            let mut guard = shared.lock().unwrap();
            guard.preamp_db = -12.;
            guard.bands[4].gain_db = 12.;
            // A blocking lock here would deadlock the test on this same thread.
            for _ in 0..1024 {
                assert!(source.next().unwrap().is_finite());
            }
            assert_eq!(source.settings, Equalizer::default());
        }
        let mut previous = source.next().unwrap();
        for _ in 0..6000 {
            let value = source.next().unwrap();
            assert!(
                (value - previous).abs() < 0.01,
                "Abrupt audio transition: {previous} -> {value}"
            );
            previous = value;
        }
        assert_eq!(source.settings.preamp_db, -12.);
        assert_eq!(source.ramp_remaining, 0);
        shared.lock().unwrap().enabled = false;
        for _ in 0..4000 {
            source.next().unwrap();
        }
        assert_eq!(
            source.coefficients.map(|c| c.0),
            [Coefficients::IDENTITY.0; BAND_COUNT]
        );
        assert_eq!(source.gain, 1.);
    }

    #[test]
    fn rapid_extreme_changes_remain_finite_at_low_and_high_sample_rates() {
        for rate in [8000, 22050, 44100, 48000, 96000, 192000] {
            let shared = control(Equalizer::default());
            let mut source = EqualizerSource::new(
                SamplesBuffer::new(1, rate, tone(rate, 400., 0.5, 50000)),
                shared.clone(),
            )
            .unwrap();
            for index in 0..20 {
                let mut settings = Equalizer::default();
                for (band_index, band) in settings.bands.iter_mut().enumerate() {
                    band.gain_db = if index % 2 == 0 { 12. } else { -12. };
                    band.q = if index % 3 == 0 { 10. } else { 0.2 };
                    band.frequency = if band_index % 2 == 0 { 20. } else { 20000. };
                    band.filter_type = FilterType::ALL[index % 3];
                }
                *shared.lock().unwrap() = settings;
                for _ in 0..2048 {
                    let value = source.next().unwrap();
                    assert!(value.is_finite() && value.abs() <= 1., "{rate}: {value}");
                }
            }
        }
    }

    #[test]
    fn seek_clears_filter_tail_and_preserves_source_metadata() {
        let mut input = vec![0.; 9600];
        input[0] = 0.2;
        let settings = Preset::BassBoost.settings();
        let buffer = SamplesBuffer::new(2, 48000, input);
        let mut source = EqualizerSource::new(buffer.clone(), control(settings)).unwrap();
        assert_eq!(source.channels(), 2);
        assert_eq!(source.sample_rate(), 48000);
        assert_eq!(source.total_duration(), buffer.total_duration());
        let expected: Vec<_> = source.by_ref().take(1000).collect();
        source.try_seek(Duration::ZERO).unwrap();
        let after_seek: Vec<_> = source.take(1000).collect();
        assert_eq!(expected, after_seek);
    }

    #[test]
    fn presets_and_custom_parameters_roundtrip_and_bad_files_are_rejected() {
        let library = crate::test_support::TestLibrary::new("equalizer-presets");
        let path = library.directory.join("custom.json");
        for preset in Preset::ALL {
            let mut settings = preset.settings();
            settings.bands[0].filter_type = FilterType::LowShelf;
            settings.bands[9].filter_type = FilterType::HighShelf;
            settings.bands[3].q = 2.5;
            settings.save(&path).unwrap();
            assert_eq!(Equalizer::load(&path).unwrap(), settings);
        }
        fs::write(&path, b"not json").unwrap();
        assert!(Equalizer::load(&path).is_err());
        fs::write(&path, vec![b' '; 65537]).unwrap();
        assert!(Equalizer::load(&path).is_err());
        let mut invalid = Equalizer::default();
        invalid.bands[0].q = 0.;
        assert!(invalid.validate().is_err());
        invalid.bands[0].q = 1.;
        invalid.bands[0].frequency = f32::NAN;
        assert!(invalid.save(&path).is_err());
        let original: crate::settings::AppSettings =
            serde_json::from_str(r#"{"fish_enabled":false}"#).unwrap();
        assert!(!original.fish_enabled);
        assert_eq!(original.equalizer, Equalizer::default());
    }
}
