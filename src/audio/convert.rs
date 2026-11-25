use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, WindowFunction, calculate_cutoff,
};

use super::decoder::SourceFormat;

const BLOCK: usize = 512;

pub struct Converter {
    source_channels: usize,
    output_channels: usize,
    mix: Vec<Vec<f32>>,
    sinc: Option<SincFixedIn<f32>>,
    planes: Vec<Vec<f32>>,
    resampled: Vec<Vec<f32>>,
    delay: usize,
    input_frames: u64,
    output_frames: u64,
    ratio: f64,
    gain: f32,
}

impl Converter {
    pub fn new(
        source: &SourceFormat,
        rate: u32,
        channels: u16,
        quality: super::ResamplerQuality,
    ) -> Result<Self, String> {
        let source_channels = usize::from(source.channels);
        let output_channels = usize::from(channels);
        let ratio = f64::from(rate) / f64::from(source.rate);
        let mut sinc = None;
        let mut delay = 0;
        let mut resampled = Vec::new();
        if rate != source.rate {
            let window = WindowFunction::BlackmanHarris2;
            let sinc_len = quality.sinc_len();
            let params = SincInterpolationParameters {
                sinc_len,
                f_cutoff: calculate_cutoff(sinc_len, window),
                oversampling_factor: 256,
                interpolation: quality.interpolation(),
                window,
            };
            let resampler = SincFixedIn::new(ratio, 1.0, params, BLOCK, output_channels)
                .map_err(|error| error.to_string())?;
            delay = resampler.output_delay();
            resampled = resampler.output_buffer_allocate(true);
            sinc = Some(resampler);
        }
        Ok(Self {
            source_channels,
            output_channels,
            mix: channel_mix(source, output_channels),
            sinc,
            planes: (0..output_channels)
                .map(|_| Vec::with_capacity(BLOCK))
                .collect(),
            resampled,
            delay,
            input_frames: 0,
            output_frames: 0,
            ratio,
            gain: 1.0,
        })
    }

    pub fn push(&mut self, pcm: &[f32], output: &mut Vec<f32>) -> Result<(), String> {
        if self.sinc.is_none() && self.source_channels == self.output_channels {
            output.extend(pcm.iter().copied().map(|sample| finite(sample) * self.gain));
            return Ok(());
        }
        for input in pcm.chunks_exact(self.source_channels) {
            self.input_frames += 1;
            for (index, weights) in self.mix.iter().enumerate() {
                let sample = weights
                    .iter()
                    .zip(input)
                    .map(|(gain, sample)| gain * finite(*sample) * self.gain)
                    .sum();
                if self.sinc.is_some() {
                    self.planes[index].push(sample);
                } else {
                    output.push(sample);
                }
            }
            if self.sinc.is_some() && self.planes[0].len() == BLOCK {
                self.process_block(output)?;
            }
        }
        Ok(())
    }

    pub fn set_gain(&mut self, gain: f32) {
        self.gain = gain;
    }

    pub fn finish(&mut self, output: &mut Vec<f32>) -> Result<(), String> {
        if self.sinc.is_none() {
            return Ok(());
        }
        // Pad the final sinc window, then keep exactly the rounded duration of the source.
        while self.output_frames < self.target_frames() {
            for plane in &mut self.planes {
                plane.resize(BLOCK, 0.0);
            }
            self.process_block(output)?;
        }
        Ok(())
    }

    pub(super) fn target_frames(&self) -> u64 {
        (self.input_frames as f64 * self.ratio).round() as u64
    }

    fn process_block(&mut self, output: &mut Vec<f32>) -> Result<(), String> {
        let target = self.target_frames();
        let sinc = self.sinc.as_mut().ok_or("Sinc resampler is unavailable")?;
        let (_, written) = sinc
            .process_into_buffer(&self.planes, &mut self.resampled, None)
            .map_err(|error| error.to_string())?;
        let start = self.delay.min(written);
        self.delay -= start;
        let frames = (target - self.output_frames).min((written - start) as u64) as usize;
        for frame in start..start + frames {
            output.extend(self.resampled.iter().map(|plane| finite(plane[frame])));
        }
        self.output_frames += frames as u64;
        for plane in &mut self.planes {
            plane.clear();
        }
        Ok(())
    }
}

fn finite(sample: f32) -> f32 {
    if sample.is_finite() { sample } else { 0.0 }
}

fn channel_mix(source: &SourceFormat, output_channels: usize) -> Vec<Vec<f32>> {
    let input_channels = usize::from(source.channels);
    let mut weights = vec![vec![0.0; input_channels]; output_channels];
    if input_channels == output_channels || (input_channels == 2 && output_channels > 2) {
        for (i, output) in weights.iter_mut().enumerate().take(input_channels) {
            output[i] = 1.0;
        }
    } else if input_channels == 1 {
        for output in weights.iter_mut().take(2) {
            output[0] = 1.0;
        }
    } else {
        let positions = (0..32).filter(|bit| source.mask & (1 << bit) != 0);
        for (index, position) in positions.enumerate() {
            let surround = std::f32::consts::FRAC_1_SQRT_2;
            let (left, right) = match position {
                0 => (1.0, 0.0),
                1 => (0.0, 1.0),
                4 | 6 | 9 | 12 | 15 => (surround, 0.0),
                5 | 7 | 10 | 14 | 17 => (0.0, surround),
                3 => (0.0, 0.0), // LFE is only used with a matching channel layout.
                _ => (surround, surround),
            };
            if output_channels == 1 {
                weights[0][index] = 0.5 * (left + right);
            } else {
                weights[0][index] = left;
                weights[1][index] = right;
            }
        }
        for output in &mut weights {
            let scale = output.iter().sum::<f32>().max(1.0);
            for gain in output {
                *gain /= scale;
            }
        }
    }
    weights
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_quality_profile_resamples_a_sine() -> Result<(), String> {
        let source = SourceFormat {
            codec: "pcm",
            rate: 48_000,
            channels: 1,
            mask: 4,
            bits: Some(24),
            kind: super::super::decoder::SampleKind::Integer(24),
            gain: None,
        };
        for quality in [
            super::super::ResamplerQuality::Fast,
            super::super::ResamplerQuality::Balanced,
            super::super::ResamplerQuality::High,
            super::super::ResamplerQuality::Maximum,
        ] {
            let mut converter = Converter::new(&source, 44_100, 1, quality)?;
            let input: Vec<f32> = (0..4_800)
                .map(|frame| (frame as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin())
                .collect();
            let mut pcm = Vec::new();
            converter.push(&input, &mut pcm)?;
            converter.finish(&mut pcm)?;
            assert!(!pcm.is_empty(), "{quality:?} produced no output");
            assert!(pcm.iter().all(|sample| sample.is_finite()));
            let peak = pcm
                .iter()
                .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
            assert!((0.8..=1.2).contains(&peak), "{quality:?} peak {peak}");
        }
        Ok(())
    }

    #[test]
    fn sinc_flush_keeps_duration_for_short_and_long_files() -> Result<(), String> {
        let source = SourceFormat {
            codec: "pcm",
            rate: 44_100,
            channels: 1,
            mask: 4,
            bits: Some(24),
            kind: super::super::decoder::SampleKind::Integer(24),
            gain: None,
        };
        for rate in [8_000, 48_000, 192_000] {
            for frames in [1, 39, 512, 2_017] {
                let mut sinc =
                    Converter::new(&source, rate, 2, super::super::ResamplerQuality::High)?;
                let mut pcm = Vec::new();
                sinc.push(&vec![0.0; frames], &mut pcm)?;
                sinc.finish(&mut pcm)?;
                let expected = 2 * (frames as f64 * f64::from(rate) / 44_100.0).round() as usize;
                assert_eq!(pcm.len(), expected);
                assert!(pcm.iter().all(|sample| *sample == 0.0));
            }
        }
        Ok(())
    }
}
