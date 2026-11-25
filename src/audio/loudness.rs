use std::path::Path;

use bs1770::{ChannelLoudnessMeter, gated_mean, reduce_stereo};

use super::{OutputOptions, decoder::Source};

/// ITU-R BS.1770-4 integrated loudness and the sample peak.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Loudness {
    pub integrated_lufs: f32,
    pub peak: f32,
}

/// Decode the whole file and measure loudness. CUE entries are measured as
/// their own segment because `Source` already applies the CUE boundaries.
pub fn measure(path: &Path) -> Result<Loudness, String> {
    let mut source = Source::open_with_options(path, OutputOptions::default())?;
    let rate = source.info.rate;
    let channels = usize::from(source.info.channels);
    let mut left = ChannelLoudnessMeter::new(rate);
    let mut right = ChannelLoudnessMeter::new(rate);
    let mut mono = (channels == 1).then(|| ChannelLoudnessMeter::new(rate));
    let mut peak = 0.0_f32;
    let mut buffer = None;
    let mut left_frame = Vec::with_capacity(4_096);
    let mut right_frame = Vec::with_capacity(4_096);
    let mut mono_frame = Vec::with_capacity(4_096);

    while let Some(packet) = source.next::<f32>(&mut buffer)? {
        left_frame.clear();
        right_frame.clear();
        mono_frame.clear();
        for frame in packet.chunks_exact(channels) {
            for sample in frame {
                peak = peak.max(sample.abs());
            }
            match channels {
                1 => mono_frame.push(frame[0]),
                2 => {
                    left_frame.push(frame[0]);
                    right_frame.push(frame[1]);
                }
                count => {
                    let even = count.div_ceil(2) as f32;
                    let odd = (count / 2).max(1) as f32;
                    let mut left_sum = 0.0;
                    let mut right_sum = 0.0;
                    for (index, sample) in frame.iter().enumerate() {
                        if index % 2 == 0 {
                            left_sum += sample;
                        } else {
                            right_sum += sample;
                        }
                    }
                    left_frame.push(left_sum / even);
                    right_frame.push(right_sum / odd);
                }
            }
        }
        if let Some(meter) = &mut mono {
            meter.push(mono_frame.iter().copied());
        } else {
            left.push(left_frame.iter().copied());
            right.push(right_frame.iter().copied());
        }
    }

    let integrated_lufs = if let Some(meter) = mono {
        let windows = meter.into_100ms_windows();
        if windows.len() == 0 {
            return Err("The track is too short to measure loudness".into());
        }
        gated_mean(windows.as_ref()).loudness_lkfs()
    } else {
        let left = left.into_100ms_windows();
        let right = right.into_100ms_windows();
        if left.len() == 0 {
            return Err("The track is too short to measure loudness".into());
        }
        gated_mean(reduce_stereo(left.as_ref(), right.as_ref()).as_ref()).loudness_lkfs()
    };
    if !integrated_lufs.is_finite() {
        return Err("The track is silent or too short to measure loudness".into());
    }
    Ok(Loudness {
        integrated_lufs,
        peak: peak.clamp(0.0, 16.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_the_generated_test_tone() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("tone.wav");
        super::super::setup::tone(&path, 48_000).unwrap();
        let measured = measure(&path).unwrap();
        assert!(measured.integrated_lufs.is_finite());
        assert!(measured.integrated_lufs < -20.0);
        assert!(measured.peak > 0.0 && measured.peak < 0.1);
    }
}
