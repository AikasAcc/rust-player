//! DSF/DFF playback as a separate source path.
//!
//! A PCM-only output cannot carry DSD bit-perfectly, so the 1-bit stream is
//! decimated to floats by `dsd-pcm` (linear-phase FIR cascade, 88.2/96 kHz
//! for DSD64/48 kHz-family files). `dsd-reader` normalises both containers to
//! MSB-first interleaved bytes. Seeking rebuilds the reader and resets the
//! filter state, because the decimator has no random-access API.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use dsd_pcm::Decimator;
use dsd_reader::{DSD_64_RATE, DsdIter, DsdReader};
use id3::TagLike;
use symphonia::core::conv::ConvertibleSample;

use super::decoder::{SampleKind, SourceFormat, frames_for};

pub(super) struct DsdSource {
    path: PathBuf,
    iter: DsdIter,
    decimator: Decimator,
    channels: usize,
    out_block: u32,
    pending: Vec<f64>,
    decoded: Vec<f64>,
    drained: bool,
}

fn open_container(path: &Path) -> Result<(DsdReader, u32, usize, Option<Duration>), String> {
    let reader =
        DsdReader::from_container(path.to_path_buf()).map_err(|error| error.to_string())?;
    // `dsd_rate()` is the DSD64 multiplier (1, 2, 4, 8), not Hz.
    let rate = u32::try_from(reader.dsd_rate())
        .ok()
        .and_then(|multiplier| multiplier.checked_mul(DSD_64_RATE))
        .ok_or_else(|| "Unsupported DSD rate".to_string())?;
    let channels = reader.channels_num();
    if rate == 0 || !(1..=32).contains(&channels) {
        return Err("Unsupported DSD stream format".into());
    }
    let bytes = reader.audio_length();
    let duration =
        Duration::try_from_secs_f64(bytes as f64 / (channels as f64 * 8.0 * rate as f64))
            .ok()
            .filter(|duration| !duration.is_zero());
    Ok((reader, rate, channels, duration))
}

fn iterator(reader: &DsdReader, out_block: u32) -> Result<DsdIter, String> {
    // `out_lsbf = false` gives MSB-first bytes, the decimator's default order.
    reader
        .interl_iter(false, Some(out_block))
        .map_err(|error| error.to_string())
}

impl DsdSource {
    pub(super) fn open(path: &Path) -> Result<(Self, SourceFormat, Option<Duration>), String> {
        let (reader, rate, channels, duration) = open_container(path)?;
        let decimator = Decimator::new(rate, channels).map_err(|error| error.to_string())?;
        let input_block = reader.block_size().max(1);
        // A smaller output block keeps seeks fine-grained; it must divide the
        // container's block size exactly.
        let out_block = [4096, 2048, 1024, 512, 256, 128, 64, 32, 16, 8, 4, 2, 1]
            .into_iter()
            .find(|&size| size <= input_block && input_block % size == 0)
            .unwrap_or(1);
        let iter = iterator(&reader, out_block)?;
        let mask = if channels >= 32 {
            u32::MAX
        } else {
            (1u32 << channels) - 1
        };
        let info = SourceFormat {
            codec: "DSD→PCM",
            rate: decimator.output_rate(),
            channels: channels as u16,
            mask,
            bits: None,
            kind: SampleKind::Float32,
            gain: None,
        };
        Ok((
            Self {
                path: path.to_path_buf(),
                iter,
                decimator,
                channels,
                out_block,
                pending: Vec::new(),
                decoded: Vec::new(),
                drained: false,
            },
            info,
            duration,
        ))
    }

    fn convert(&mut self, bytes: &[u8]) -> Result<(), String> {
        if bytes.is_empty() {
            return Ok(());
        }
        self.decoded.clear();
        self.decimator
            .process_interleaved(bytes, &mut self.decoded)
            .map_err(|error| error.to_string())?;
        self.pending.append(&mut self.decoded);
        Ok(())
    }

    pub(super) fn next<T: ConvertibleSample>(&mut self) -> Result<Option<Vec<T>>, String> {
        while self.pending.is_empty() {
            if self.drained {
                return Ok(None);
            }
            let Some((_, buffers)) = self.iter.next() else {
                self.decoded.clear();
                self.decimator.finish(&mut self.decoded);
                self.pending.append(&mut self.decoded);
                self.drained = true;
                break;
            };
            if let Some(bytes) = buffers.first() {
                self.convert(bytes)?;
            }
        }
        Ok(Some(
            std::mem::take(&mut self.pending)
                .into_iter()
                .map(T::from_sample)
                .collect(),
        ))
    }

    pub(super) fn seek(&mut self, position: Duration) -> Result<(), String> {
        let frames = frames_for(position, self.decimator.output_rate());
        let factor = u64::from(self.decimator.factor());
        let bytes_per_channel = frames.saturating_mul(factor) / 8;
        let block = u64::from(self.out_block);
        let skip = bytes_per_channel / block;
        let remainder = (bytes_per_channel % block) as usize;

        let (reader, _, _, _) = open_container(&self.path)?;
        let mut iter = iterator(&reader, self.out_block)?;
        for _ in 0..skip {
            if iter.next().is_none() {
                break;
            }
        }
        self.iter = iter;
        self.decimator.reset();
        self.pending.clear();
        self.decoded.clear();
        self.drained = false;

        if remainder > 0
            && let Some((_, buffers)) = self.iter.next()
            && let Some(bytes) = buffers.first()
        {
            self.convert(bytes)?;
            let drop_frames = remainder as u64 * 8 / factor;
            if let Ok(drop) = usize::try_from(drop_frames.saturating_mul(self.channels as u64)) {
                self.pending.drain(..drop.min(self.pending.len()));
            }
        }
        Ok(())
    }
}

/// The ID3 fields we can show for DSF/DFF files, which lofty cannot probe.
#[derive(Default)]
pub struct DsdMetadata {
    pub title: Option<String>,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub year: Option<u16>,
    pub disc: Option<u32>,
    pub number: Option<u32>,
}

pub fn metadata(path: &Path) -> Option<DsdMetadata> {
    let reader = DsdReader::from_container(path.to_path_buf()).ok()?;
    let tag = reader.tag().as_ref()?;
    let text = |value: Option<&str>| {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let year = tag
        .year()
        .and_then(|year| u16::try_from(year).ok())
        .or_else(|| {
            tag.date_recorded()
                .and_then(|date| u16::try_from(date.year).ok())
        });
    Some(DsdMetadata {
        title: text(tag.title()),
        artist: text(tag.artist()).unwrap_or_default(),
        album: text(tag.album()).unwrap_or_default(),
        album_artist: text(tag.album_artist()).unwrap_or_default(),
        genre: tag
            .genre_parsed()
            .map(|genre| genre.trim().to_owned())
            .unwrap_or_default(),
        year: year.filter(|&year| year > 0),
        disc: tag.disc().filter(|&disc| disc > 0),
        number: tag.track().filter(|&track| track > 0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn frame(id: &[u8; 4], text: &str) -> Vec<u8> {
        let mut result = Vec::new();
        result.extend_from_slice(id);
        result.extend_from_slice(&((text.len() + 1) as u32).to_be_bytes());
        result.extend_from_slice(&[0, 0]);
        result.push(0); // UTF-8
        result.extend_from_slice(text.as_bytes());
        result
    }

    fn id3_tag() -> Vec<u8> {
        let mut frames = Vec::new();
        frames.extend(frame(b"TIT2", "DSD Title"));
        frames.extend(frame(b"TPE1", "DSD Artist"));
        frames.extend(frame(b"TALB", "DSD Album"));
        frames.extend(frame(b"TCON", "Jazz"));
        frames.extend(frame(b"TDRC", "1959"));
        let mut tag = Vec::from(b"ID3");
        tag.extend_from_slice(&[3, 0, 0]);
        let size = frames.len() as u32;
        tag.extend_from_slice(&[
            ((size >> 21) & 0x7f) as u8,
            ((size >> 14) & 0x7f) as u8,
            ((size >> 7) & 0x7f) as u8,
            (size & 0x7f) as u8,
        ]);
        tag.extend(frames);
        tag
    }

    /// Minimal DSF: stereo DSD64, planar blocks of 4096 bytes, 0x69 silence.
    fn write_dsf(path: &Path, blocks: usize, tag: Option<&[u8]>) {
        let bytes_per_channel = blocks * 4096;
        let data = bytes_per_channel * 2;
        let tag_len = tag.map_or(0, <[u8]>::len);
        let metadata_offset = if tag.is_some() { 92 + data } else { 0 };
        let mut file = Vec::from(b"DSD ");
        file.extend_from_slice(&28u64.to_le_bytes());
        file.extend_from_slice(&((92 + data + tag_len) as u64).to_le_bytes());
        file.extend_from_slice(&(metadata_offset as u64).to_le_bytes());
        file.extend_from_slice(b"fmt ");
        file.extend_from_slice(&52u64.to_le_bytes());
        file.extend_from_slice(&1u32.to_le_bytes()); // version
        file.extend_from_slice(&0u32.to_le_bytes()); // DSD raw
        file.extend_from_slice(&2u32.to_le_bytes()); // stereo
        file.extend_from_slice(&2u32.to_le_bytes()); // channels
        file.extend_from_slice(&2_822_400u32.to_le_bytes());
        file.extend_from_slice(&1u32.to_le_bytes()); // LSB first
        file.extend_from_slice(&((bytes_per_channel * 8) as u64).to_le_bytes());
        file.extend_from_slice(&4096u32.to_le_bytes());
        file.extend_from_slice(&0u32.to_le_bytes());
        file.extend_from_slice(b"data");
        file.extend_from_slice(&((data + 12) as u64).to_le_bytes());
        file.extend(std::iter::repeat_n(0x69u8, data));
        if let Some(tag) = tag {
            file.extend_from_slice(tag);
        }
        fs::write(path, file).unwrap();
    }

    #[test]
    fn dsf_silence_decodes_to_float_pcm() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("silent.dsf");
        write_dsf(&path, 1, None);
        let (mut source, info, duration) = DsdSource::open(&path).unwrap();
        assert_eq!(info.rate, 88_200);
        assert_eq!(info.channels, 2);
        assert_eq!(info.kind, SampleKind::Float32);
        assert!(duration.is_some_and(|duration| duration > Duration::ZERO));
        let samples = source.next::<f32>().unwrap().expect("PCM block");
        // The first block loses the filter's group delay, so fewer than 1024
        // whole frames survive; later blocks and `finish` emit the rest.
        assert!(samples.len() >= 1400, "unexpected length {}", samples.len());
        assert!(
            samples
                .iter()
                .all(|sample| sample.is_finite() && sample.abs() < 0.05)
        );
        while source.next::<f32>().unwrap().is_some() {}
    }

    #[test]
    fn seeking_skips_dsd_blocks_before_decoding() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("long.dsf");
        write_dsf(&path, 4, None);
        let (mut source, _, _) = DsdSource::open(&path).unwrap();
        let first = source.next::<f32>().unwrap().expect("first block").len();
        source
            .seek(Duration::from_secs_f64(0.0058))
            .expect("seek into first block");
        let shifted = source
            .next::<f32>()
            .unwrap()
            .expect("block after seek")
            .len();
        assert!(
            shifted > 0 && shifted < first,
            "first={first}, shifted={shifted}"
        );
        source.seek(Duration::from_secs_f64(0.0232)).unwrap();
        let tail = source.next::<f32>().unwrap().expect("tail block").len();
        assert!(tail > 0, "first={first}, tail={tail}");
    }

    #[test]
    fn id3_tags_are_read_for_dsf_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("tagged.dsf");
        let tag = id3_tag();
        write_dsf(&path, 1, Some(&tag));
        let info = crate::library::TrackInfo::read(&path);
        assert_eq!(info.title, "DSD Title");
        assert_eq!(info.artist, "DSD Artist");
        assert_eq!(info.album, "DSD Album");
        assert_eq!(info.genre, "Jazz");
        assert_eq!(info.year, Some(1959));
        assert!(info.duration.is_some());
    }
}
