//! Streaming Monkey's Audio (APE) decoder adapter.
//!
//! `ape-decoder` emits little-endian interleaved PCM. This module only feeds
//! those samples into the player's existing `SourceFormat`/render pipeline; no
//! gain, conversion, or resampling happens here.

use std::{fs::File, io::BufReader, path::Path, time::Duration};

use ape_decoder::{ApeDecoder, ApeInfo};
use symphonia::core::conv::ConvertibleSample;

use super::decoder::{SampleKind, SourceFormat};

pub(super) struct ApeSource {
    decoder: ApeDecoder<BufReader<File>>,
    info: ApeInfo,
    next_frame: u32,
    skip_samples: usize,
    frame: Vec<u8>,
    frame_position: usize,
}

impl ApeSource {
    pub(super) fn open(path: &Path) -> Result<(Self, SourceFormat, Option<Duration>), String> {
        let file = File::open(path).map_err(|error| error.to_string())?;
        let decoder = ApeDecoder::new(BufReader::new(file)).map_err(|error| error.to_string())?;
        let info = decoder.info().clone();
        let rate = info.sample_rate;
        let channels = info.channels;
        let bits = u32::from(info.bits_per_sample);
        if !(1..=768_000).contains(&rate) {
            return Err("Unsupported source sample rate".into());
        }
        if !(1..=32).contains(&channels) {
            return Err("Unsupported source channel count".into());
        }
        if !(8..=64).contains(&bits) || bits % 8 != 0 {
            return Err("Unsupported APE sample precision".into());
        }
        let kind = if info.is_floating_point {
            match bits {
                32 => SampleKind::Float32,
                64 => SampleKind::Float64,
                _ => return Err("Unsupported APE floating-point precision".into()),
            }
        } else {
            SampleKind::Integer(bits as u16)
        };
        let format = SourceFormat {
            codec: "Monkey's Audio",
            rate,
            channels,
            mask: (1u32 << channels) - 1,
            bits: Some(bits),
            kind,
            gain: None,
        };
        let duration = if info.total_samples > 0 && rate > 0 {
            Duration::try_from_secs_f64(info.total_samples as f64 / f64::from(rate)).ok()
        } else {
            Some(Duration::from_millis(info.duration_ms))
        };
        Ok((
            Self {
                decoder,
                info,
                next_frame: 0,
                skip_samples: 0,
                frame: Vec::new(),
                frame_position: 0,
            },
            format,
            duration.filter(|duration| !duration.is_zero()),
        ))
    }

    pub(super) fn seek(&mut self, position: Duration) -> Result<(), String> {
        let sample = super::decoder::frames_for(position, self.info.sample_rate)
            .min(self.info.total_samples);
        let target = self
            .decoder
            .seek(sample)
            .map_err(|error| error.to_string())?;
        self.next_frame = target.frame_index;
        self.skip_samples = target.skip_samples as usize;
        self.frame.clear();
        self.frame_position = 0;
        Ok(())
    }

    pub(super) fn next<T: ConvertibleSample>(&mut self) -> Result<Option<Vec<T>>, String> {
        loop {
            if self.frame_position < self.frame_samples() {
                let bytes = usize::from(self.info.bytes_per_sample.max(1));
                let available = self.frame_samples() - self.frame_position;
                let start = self.frame_position * bytes * usize::from(self.info.channels);
                let end = start + available * bytes * usize::from(self.info.channels);
                let chunk = self.frame.get(start..end).ok_or("Truncated APE frame")?;
                self.frame_position += available;
                return Ok(Some(convert(
                    chunk,
                    self.info.bits_per_sample,
                    self.info.bytes_per_sample,
                    self.info.is_floating_point,
                    self.info.is_signed_8bit,
                )?));
            }
            if self.next_frame >= self.info.total_frames {
                return Ok(None);
            }
            let frame = self
                .decoder
                .decode_frame(self.next_frame)
                .map_err(|error| error.to_string())?;
            self.next_frame += 1;
            self.frame_position = self.skip_samples;
            self.skip_samples = 0;
            self.frame = frame;
        }
    }

    fn frame_samples(&self) -> usize {
        let bytes =
            usize::from(self.info.bytes_per_sample.max(1)) * usize::from(self.info.channels);
        if bytes == 0 {
            return 0;
        }
        self.frame.len() / bytes
    }
}

fn convert<T: ConvertibleSample>(
    bytes: &[u8],
    bits_per_sample: u16,
    bytes_per_sample: u16,
    floating_point: bool,
    signed_8bit: bool,
) -> Result<Vec<T>, String> {
    let bits = u32::from(bits_per_sample);
    let width = usize::from(bytes_per_sample.max(1));
    if !bytes.len().is_multiple_of(width) {
        return Err("Misaligned APE PCM frame".into());
    }
    let mut samples = Vec::with_capacity(bytes.len() / width);
    for chunk in bytes.chunks_exact(width) {
        let sample = if floating_point {
            match bits {
                32 => T::from_sample(f32::from_le_bytes(
                    chunk
                        .try_into()
                        .map_err(|_| "Invalid APE float sample".to_string())?,
                )),
                64 => T::from_sample(f64::from_le_bytes(
                    chunk
                        .try_into()
                        .map_err(|_| "Invalid APE float sample".to_string())?,
                )),
                _ => return Err("Unsupported APE floating-point precision".into()),
            }
        } else {
            let value = match bits {
                8 => {
                    let raw = chunk[0];
                    if signed_8bit {
                        i32::from(raw as i8)
                    } else {
                        i32::from(raw) - 128
                    }
                }
                16 => i32::from(i16::from_le_bytes(
                    chunk
                        .try_into()
                        .map_err(|_| "Invalid APE integer sample".to_string())?,
                )),
                24 => {
                    let raw = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], 0]);
                    if raw & 0x0080_0000 != 0 {
                        (raw | 0xff00_0000) as i32
                    } else {
                        raw as i32
                    }
                }
                32 => i32::from_le_bytes(
                    chunk
                        .try_into()
                        .map_err(|_| "Invalid APE integer sample".to_string())?,
                ),
                _ => return Err("Unsupported APE sample precision".into()),
            };
            T::from_sample(value)
        };
        samples.push(sample);
    }
    Ok(samples)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_signed_24_bit_little_endian_samples() {
        let bytes = [0x00, 0x00, 0x00, 0xff, 0xff, 0xff];
        let samples = convert::<i32>(&bytes, 24, 3, false, true).expect("24-bit samples convert");
        assert_eq!(samples, vec![0, -1]);
    }

    #[test]
    fn converts_unsigned_8_bit_samples_to_pcm_center() {
        let samples =
            convert::<i32>(&[0x80, 0xff], 8, 1, false, false).expect("8-bit samples convert");
        assert_eq!(samples, vec![0, 127]);
    }
}
