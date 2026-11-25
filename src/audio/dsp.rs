use super::{OutputOptions, decoder::Source};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const MAX_TAPS: usize = 16_384;
const MAX_BANDS: usize = 24;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub eq: bool,
    pub bands: Vec<Band>,
    pub preamp: f32,
    pub fir: bool,
    pub impulse: Option<PathBuf>,
    pub crossfeed: bool,
}

impl Settings {
    pub fn active(&self) -> bool {
        (self.eq && (!self.bands.is_empty() || self.preamp != 0.0))
            || (self.fir && self.impulse.is_some())
            || self.crossfeed
    }

    pub fn import_eq(&mut self, file: &Path) -> Result<(), String> {
        use std::io::Read;
        let mut text = String::new();
        std::fs::File::open(file)
            .and_then(|f| f.take(65_537).read_to_string(&mut text))
            .map_err(|error| error.to_string())?;
        if text.len() > 65_536 {
            return Err("The EQ profile is larger than 64 KiB.".into());
        }
        let (preamp, bands) = parse_eq(&text)?;
        self.preamp = preamp;
        self.bands = bands;
        self.eq = true;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Band {
    kind: Filter,
    hz: f32,
    db: f32,
    q: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
enum Filter {
    Peak,
    LowShelf,
    HighShelf,
}

fn parse_eq(text: &str) -> Result<(f32, Vec<Band>), String> {
    let mut preamp = 0.0;
    let mut bands = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim().trim_start_matches('\u{feff}');
        let error = || format!("Invalid AutoEQ filter or value on line {}.", index + 1);
        if let Some(value) = line.strip_prefix("Preamp:") {
            preamp = value
                .split_whitespace()
                .next()
                .and_then(|value| value.parse::<f32>().ok())
                .filter(|value| (-40.0..=20.0).contains(value))
                .ok_or_else(error)?;
        } else if line.starts_with("Filter") {
            let (_, values) = line.split_once(':').ok_or_else(error)?;
            let words: Vec<_> = values.split_whitespace().collect();
            if words.first() == Some(&"OFF") {
                continue;
            }
            let kind = match words.get(1).copied() {
                Some("PK") => Filter::Peak,
                Some("LS" | "LSC") => Filter::LowShelf,
                Some("HS" | "HSC") => Filter::HighShelf,
                _ => return Err(error()),
            };
            let value = |key, range: std::ops::RangeInclusive<f32>| {
                words
                    .windows(2)
                    .find(|pair| pair[0] == key)
                    .and_then(|pair| pair[1].parse::<f32>().ok())
                    .filter(|value| range.contains(value))
                    .ok_or_else(error)
            };
            if words.first() != Some(&"ON") || bands.len() == MAX_BANDS {
                return Err(error());
            }
            bands.push(Band {
                kind,
                hz: value("Fc", 10.0..=40_000.0)?,
                db: value("Gain", -40.0..=20.0)?,
                q: value("Q", 0.05..=25.0)?,
            });
        }
    }
    if bands.is_empty() && preamp == 0.0 {
        return Err("No enabled AutoEQ filters were found.".into());
    }
    Ok((preamp, bands))
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Active {
    pub eq: bool,
    pub fir: bool,
    pub crossfeed: bool,
}

struct Biquad {
    coeff: [f32; 5],
    memory: Vec<[f32; 2]>,
}

impl Biquad {
    fn new(band: Band, rate: u32, channels: usize) -> Option<Self> {
        if !(10.0..(rate as f32 * 0.49)).contains(&band.hz)
            || !(-40.0..=20.0).contains(&band.db)
            || !(0.05..=25.0).contains(&band.q)
        {
            return None;
        }
        let (sin, cos) = (std::f64::consts::TAU * f64::from(band.hz) / f64::from(rate)).sin_cos();
        let a = 10_f64.powf(f64::from(band.db) / 40.0);
        let alpha = sin / (2.0 * f64::from(band.q));
        let beta = 2.0 * a.sqrt() * alpha;
        let (p, m) = (a + 1.0, a - 1.0);
        let [b0, b1, b2, a0, a1, a2] = match band.kind {
            Filter::Peak => [
                1.0 + alpha * a,
                -2.0 * cos,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cos,
                1.0 - alpha / a,
            ],
            Filter::LowShelf => [
                a * (p - m * cos + beta),
                2.0 * a * (m - p * cos),
                a * (p - m * cos - beta),
                p + m * cos + beta,
                -2.0 * (m + p * cos),
                p + m * cos - beta,
            ],
            Filter::HighShelf => [
                a * (p + m * cos + beta),
                -2.0 * a * (m + p * cos),
                a * (p + m * cos - beta),
                p - m * cos + beta,
                2.0 * (m - p * cos),
                p - m * cos - beta,
            ],
        };
        Some(Self {
            coeff: [b0, b1, b2, a1, a2].map(|x| (x / a0) as f32),
            memory: vec![[0.0; 2]; channels],
        })
    }

    fn process(&mut self, channel: usize, x: f32) -> f32 {
        let [b0, b1, b2, a1, a2] = self.coeff;
        let memory = &mut self.memory[channel];
        let y = b0 * x + memory[0];
        *memory = [b1 * x - a1 * y + memory[1], b2 * x - a2 * y];
        y
    }
}

struct Fir {
    taps: Vec<Vec<f32>>,
    histories: Vec<Vec<f32>>,
    cursor: usize,
}

impl Fir {
    fn load(path: &Path, rate: u32, channels: usize) -> Result<Self, String> {
        if channels > 2
            || !path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("wav"))
        {
            return Err("FIR requires a WAV impulse and mono or stereo output.".into());
        }
        let mut source = Source::open_with_options(path, OutputOptions::default())?;
        let input_channels = usize::from(source.info.channels);
        if input_channels > 2 || (input_channels == 2 && channels == 1) {
            return Err("Choose a mono impulse for mono output, or a mono or stereo impulse for stereo output.".into());
        }
        let mut response = vec![Vec::<f32>::new(); input_channels];
        let mut buffer = None;
        while let Some(packet) = source.next::<f32>(&mut buffer)? {
            if response[0].len() + packet.len() / input_channels > MAX_TAPS {
                return Err(format!(
                    "FIR impulses are limited to {MAX_TAPS} samples per channel."
                ));
            }
            for frame in packet.chunks_exact(input_channels) {
                for (response, sample) in response.iter_mut().zip(frame) {
                    response.push(if sample.is_finite() { *sample } else { 0.0 });
                }
            }
        }
        let ratio = f64::from(source.info.rate) / f64::from(rate);
        let length = (response[0].len() as f64 / ratio).round() as usize;
        if !(1..=MAX_TAPS).contains(&length) {
            return Err(format!(
                "The FIR impulse must fit {MAX_TAPS} samples at the output rate."
            ));
        }
        if source.info.rate != rate {
            let input: Vec<_> = (0..response[0].len())
                .flat_map(|frame| response.iter().map(move |channel| channel[frame]))
                .collect();
            let mut resampler = super::convert::Converter::new(
                &source.info,
                rate,
                source.info.channels,
                super::ResamplerQuality::High,
            )?;
            let mut output = Vec::new();
            resampler.push(&input, &mut output)?;
            resampler.finish(&mut output)?;
            for channel in &mut response {
                channel.clear();
            }
            for frame in output.chunks_exact(input_channels) {
                for (channel, sample) in response.iter_mut().zip(frame) {
                    channel.push(*sample * ratio as f32);
                }
            }
        }
        Ok(Self {
            taps: response,
            histories: vec![vec![0.0; length * 2]; channels],
            cursor: 0,
        })
    }

    fn process(&mut self, frame: &mut [f32]) {
        let length = self.taps[0].len();
        self.cursor = (self.cursor + length - 1) % length;
        for (channel, output) in frame.iter_mut().enumerate() {
            let history = &mut self.histories[channel];
            history[self.cursor] = *output;
            history[self.cursor + length] = *output;
            *output = history[self.cursor..self.cursor + length]
                .iter()
                .zip(&self.taps[channel % self.taps.len()])
                .map(|(sample, tap)| sample * tap)
                .sum();
        }
    }
}

pub(super) struct Processor {
    pub active: Active,
    channels: usize,
    preamp: f32,
    eq: Vec<Biquad>,
    fir: Option<Fir>,
    crossfeed: [f32; 2],
    low_pass: f32,
}

impl Processor {
    pub fn new(settings: &Settings, rate: u32, channels: usize) -> (Self, Option<String>) {
        let preamp = if settings.eq && (-40.0..=20.0).contains(&settings.preamp) {
            10_f32.powf(settings.preamp / 20.0)
        } else {
            1.0
        };
        let eq: Vec<_> = settings
            .bands
            .iter()
            .take(MAX_BANDS)
            .copied()
            .filter(|_| settings.eq)
            .filter_map(|band| Biquad::new(band, rate, channels))
            .collect();
        let (fir, warning) = match settings.impulse.as_deref().filter(|_| settings.fir) {
            Some(path) => match Fir::load(path, rate, channels) {
                Ok(fir) => (Some(fir), None),
                Err(error) => (None, Some(error)),
            },
            None => (None, None),
        };
        let active = Active {
            eq: !eq.is_empty() || preamp != 1.0,
            fir: fir.is_some(),
            crossfeed: settings.crossfeed && channels == 2,
        };
        (
            Self {
                active,
                channels,
                preamp,
                eq,
                fir,
                crossfeed: [0.0; 2],
                low_pass: 1.0 - (-std::f32::consts::TAU * 700.0 / rate as f32).exp(),
            },
            warning,
        )
    }

    pub fn process(&mut self, samples: &mut [f32]) {
        if !(self.active.eq || self.active.fir || self.active.crossfeed) {
            return;
        }
        for frame in samples.chunks_exact_mut(self.channels) {
            if self.active.eq {
                for (channel, sample) in frame.iter_mut().enumerate() {
                    let mut value = *sample * self.preamp;
                    for filter in &mut self.eq {
                        value = filter.process(channel, value);
                    }
                    *sample = value;
                }
            }
            if let Some(fir) = &mut self.fir {
                fir.process(frame);
            }
            if self.active.crossfeed {
                for (low, sample) in self.crossfeed.iter_mut().zip(frame.iter()) {
                    *low += (*sample - *low) * self.low_pass;
                }
                frame[0] = (frame[0] + 0.16 * self.crossfeed[1]) / 1.16;
                frame[1] = (frame[1] + 0.16 * self.crossfeed[0]) / 1.16;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_autoeq_and_applies_an_impulse_across_packet_boundaries() {
        let text = "Preamp: -6 dB\nFilter 1: ON PK Fc 1000 Hz Gain 3 dB Q 1.0\nFilter 2: OFF PK Fc 400 Hz Gain 4 dB Q 1.2";
        let (preamp, bands) = parse_eq(text).unwrap();
        assert_eq!(preamp, -6.0);
        assert_eq!(bands.len(), 1);
        assert!(parse_eq("Filter 1: ON PK Fc 1000 Hz Gain 4 dB Q NaN").is_err());
        let mut fir = Fir {
            taps: vec![vec![0.5, 0.25]],
            histories: vec![vec![0.0; 4]],
            cursor: 0,
        };
        let mut impulse = [1.0];
        let mut next = [0.0];
        fir.process(&mut impulse);
        fir.process(&mut next);
        assert_eq!(impulse, [0.5]);
        assert_eq!(next, [0.25]);
    }
}
