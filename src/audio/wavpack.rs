//! Streaming WavPack (`.wv`) decoder adapter.
//!
//! `wavpack 0.4.0` wraps the reference libwavpack decoder. Integers arrive
//! right-justified in an `i32`, so they are shifted to the player's full-scale
//! convention before conversion. Float files are opened with normalization and
//! their `i32` slots are reinterpreted as normalized `f32` values. Seeking uses
//! libwavpack's exact sample index; no DSP or resampling happens in this module.

use std::{path::Path, time::Duration};

use ::wavpack::WavpackReader;
use symphonia::core::conv::ConvertibleSample;

use super::decoder::{SampleKind, SourceFormat, frames_for};

const MODE_FLOAT: i32 = 8;
const QMODE_DSD_AUDIO: i32 = 0x30;
const BLOCK_FRAMES: usize = 4096;

// SAFETY: `wavpack::WavpackReader` is moved, not shared. libwavpack documents
// that a context has no static state, and this source is constructed with a
// `BufReader<File>` whose callbacks are only invoked from the audio worker that
// owns the source. No other thread touches the context or the reader.
unsafe impl Send for WavpackSource {}

pub(super) struct WavpackSource {
    reader: WavpackReader,
    rate: u32,
    channels: usize,
    float: bool,
    shift: u32,
    total_samples: i64,
    buffer: Vec<i32>,
    decoded: usize,
    cursor: usize,
}

impl WavpackSource {
    pub(super) fn open(
        path: &Path,
        gain: Option<f32>,
    ) -> Result<(Self, SourceFormat, Option<Duration>), String> {
        // Normalization only affects float files; it guarantees +/-1.0 output.
        let mut reader = WavpackReader::open(path)
            .map_err(|error| error.to_string())?
            .normalize(0)
            .tags()
            .build()
            .map_err(|error| error.to_string())?;
        let channels = usize::try_from(
            reader
                .get_num_channels()
                .map_err(|error| error.to_string())?,
        )
        .ok()
        .filter(|&channels| (1..=32).contains(&channels))
        .ok_or("Unsupported WavPack channel count")?;
        // DSD stored in WavPack needs libwavpack's DSD flags or a DoP output;
        // neither is available here. Reject it instead of playing raw 1-bit
        // bytes as if they were PCM.
        if is_dsd(
            reader
                .get_qualify_mode()
                .map_err(|error| error.to_string())?,
        ) {
            return Err("DSD in WavPack is not supported".into());
        }
        let rate = reader
            .get_sample_rate()
            .map_err(|error| error.to_string())?;
        if !(1..=768_000).contains(&rate) {
            return Err("Unsupported source sample rate".into());
        }
        let bits = u32::try_from(
            reader
                .get_bits_per_sample()
                .map_err(|error| error.to_string())?,
        )
        .ok()
        .filter(|&bits| (1..=32).contains(&bits))
        .ok_or("Unsupported WavPack sample precision")?;
        let float = reader.get_mode().map_err(|error| error.to_string())? & MODE_FLOAT != 0;
        if float && bits != 32 {
            return Err("Unsupported WavPack floating-point precision".into());
        }
        let mask = reader
            .get_channel_mask()
            .map_err(|error| error.to_string())?;
        let mask = if mask > 0 {
            mask as u32
        } else if channels == 32 {
            u32::MAX
        } else {
            (1u32 << channels) - 1
        };
        let total_samples = reader
            .get_num_samples()
            .map_err(|error| error.to_string())?;
        let duration = (total_samples > 0)
            .then(|| Duration::try_from_secs_f64(total_samples as f64 / f64::from(rate)).ok())
            .flatten()
            .filter(|duration| !duration.is_zero());
        let info = SourceFormat {
            codec: "WavPack",
            rate,
            channels: channels as u16,
            mask,
            bits: Some(bits),
            kind: if float {
                SampleKind::Float32
            } else {
                SampleKind::Integer(bits as u16)
            },
            gain,
        };
        Ok((
            Self {
                reader,
                rate,
                channels,
                float,
                shift: 32 - bits,
                total_samples,
                buffer: vec![0; BLOCK_FRAMES * channels],
                decoded: 0,
                cursor: 0,
            },
            info,
            duration,
        ))
    }

    pub(super) fn seek(&mut self, position: Duration) -> Result<(), String> {
        let frames = frames_for(position, self.rate);
        let limit = u64::try_from(self.total_samples.max(0)).unwrap_or(u64::MAX);
        let sample = i64::try_from(frames.min(limit)).unwrap_or(i64::MAX);
        let status = self
            .reader
            .seek_sample(sample)
            .map_err(|error| error.to_string())?;
        if status == 0 {
            return Err("WavPack seek failed".into());
        }
        self.decoded = 0;
        self.cursor = 0;
        Ok(())
    }

    pub(super) fn next<T: ConvertibleSample>(&mut self) -> Result<Option<Vec<T>>, String> {
        if self.cursor == self.decoded {
            let frames = self
                .reader
                .unpack_samples(&mut self.buffer)
                .map_err(|error| error.to_string())? as usize;
            if frames == 0 {
                return Ok(None);
            }
            self.decoded = frames * self.channels;
            self.cursor = 0;
        }
        let pending = &self.buffer[self.cursor..self.decoded];
        self.cursor = self.decoded;
        let samples = if self.float {
            pending
                .iter()
                .map(|&sample| T::from_sample(f32::from_bits(sample as u32)))
                .collect()
        } else {
            pending
                .iter()
                .map(|&sample| T::from_sample(sample.wrapping_shl(self.shift)))
                .collect()
        };
        Ok(Some(samples))
    }
}

/// The fields lofty cannot read from a `.wv` file.
#[derive(Default)]
pub struct WavpackMetadata {
    pub title: Option<String>,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub year: Option<u16>,
    pub disc: Option<u32>,
    pub number: Option<u32>,
}

pub fn metadata(path: &Path) -> Option<WavpackMetadata> {
    let mut reader = WavpackReader::open(path).ok()?.tags().build().ok()?;
    let tags = reader.get_text_tag_items().ok()?;
    let mut result = WavpackMetadata::default();
    apply_tags(tags, &mut result);
    Some(result)
}

fn apply_tags(tags: Vec<(String, String)>, result: &mut WavpackMetadata) {
    for (key, value) in tags {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        match key.to_ascii_uppercase().as_str() {
            "TITLE" => result.title = Some(value.to_owned()),
            "ARTIST" => result.artist = value.to_owned(),
            "ALBUM" => result.album = value.to_owned(),
            "ALBUMARTIST" | "ALBUM ARTIST" => result.album_artist = value.to_owned(),
            "GENRE" => result.genre = value.to_owned(),
            "DATE" | "YEAR" | "RECORDINGDATE" => result.year = parse_year(value),
            "DISC" | "DISCNUMBER" => result.disc = parse_number(value),
            "TRACK" | "TRACKNUMBER" => result.number = parse_number(value),
            _ => {}
        }
    }
}

fn parse_number(value: &str) -> Option<u32> {
    value
        .split('/')
        .next()
        .and_then(|value| value.trim().parse().ok())
        .filter(|&number| number > 0)
}

fn parse_year(value: &str) -> Option<u16> {
    value
        .trim()
        .get(..4)
        .and_then(|year| year.parse().ok())
        .filter(|&year| year > 0)
}

fn is_dsd(qualify_mode: i32) -> bool {
    qualify_mode & QMODE_DSD_AUDIO != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::decoder::Source;
    use ::wavpack::WavpackWriter;

    fn write_wv(path: &Path, samples: Vec<i32>, bits: i32, float: bool) {
        let mut writer = WavpackWriter::create(path)
            .unwrap()
            .add_bits_per_sample(bits)
            .add_bytes_per_sample(bits / 8)
            .add_num_channels(1)
            .add_sample_rate(48_000)
            .add_channel_mask(4)
            .add_float_norm_exp(if float { 127 } else { 0 })
            .build()
            .unwrap();
        writer.pack_samples(&mut samples.clone()).unwrap();
        writer.close().unwrap();
    }

    #[test]
    fn integer_samples_are_normalized_to_full_scale() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("pcm.wv");
        let expected = [257_i32, -257, 32_767, -32_768];
        write_wv(&path, expected.to_vec(), 16, false);

        let (mut source, info, duration) = WavpackSource::open(&path, None).unwrap();
        assert_eq!(info.codec, "WavPack");
        assert_eq!(info.rate, 48_000);
        assert_eq!(info.channels, 1);
        assert_eq!(info.kind, SampleKind::Integer(16));
        assert_eq!(duration, Some(Duration::from_secs_f64(4.0 / 48_000.0)));

        let mut decoded = Vec::new();
        while let Some(block) = source.next::<i32>().unwrap() {
            decoded.extend(block);
        }
        let normalized: Vec<i32> = expected.iter().map(|sample| sample << 16).collect();
        assert_eq!(decoded, normalized);
    }

    #[test]
    fn float_samples_are_returned_as_normalized_floats() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("float.wv");
        let values = [0.5_f32, -0.25, 0.125];
        let bits = values.map(|value| value.to_bits() as i32).to_vec();
        write_wv(&path, bits, 32, true);

        let (mut source, info, _) = WavpackSource::open(&path, None).unwrap();
        assert_eq!(info.kind, SampleKind::Float32);
        let mut decoded = Vec::new();
        while let Some(block) = source.next::<f32>().unwrap() {
            decoded.extend(block);
        }
        assert_eq!(decoded.len(), values.len());
        for (actual, expected) in decoded.iter().zip(values) {
            assert!((*actual - expected).abs() < 1e-6, "{actual} != {expected}");
        }
    }

    #[test]
    fn replay_gain_is_carried_into_the_source_format() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("gain.wv");
        write_wv(&path, vec![0; 16], 16, false);

        let (_, info, _) = WavpackSource::open(&path, Some(0.5)).unwrap();
        assert_eq!(info.gain, Some(0.5));
    }

    #[test]
    fn dsd_qualify_mode_is_detected_before_pcm_decoding() {
        assert!(is_dsd(QMODE_DSD_AUDIO));
        assert!(is_dsd(0x10));
        assert!(is_dsd(0x20));
        assert!(!is_dsd(0));
        assert!(!is_dsd(0x40));
    }

    #[test]
    fn seeking_lands_on_the_requested_sample() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("seek.wv");
        let samples: Vec<i32> = (0..16_384).map(|index| (index % 1_024) - 512).collect();
        write_wv(&path, samples.clone(), 16, false);

        let (mut source, _, _) = WavpackSource::open(&path, None).unwrap();
        source
            .seek(Duration::from_secs_f64(8_192.0 / 48_000.0))
            .unwrap();
        let mut first = source
            .next::<i32>()
            .unwrap()
            .expect("sample after seek")
            .into_iter();
        assert_eq!(first.next(), Some(samples[8_192] << 16));
    }

    #[test]
    fn common_tags_are_mapped_for_the_library() {
        let mut metadata = WavpackMetadata::default();
        apply_tags(
            vec![
                ("Title".into(), " A Title ".into()),
                ("ARTIST".into(), "An Artist".into()),
                ("ALBUM".into(), "An Album".into()),
                ("ALBUMARTIST".into(), "Album Artist".into()),
                ("GENRE".into(), "Jazz".into()),
                ("DATE".into(), "1975-04-01".into()),
                ("DISCNUMBER".into(), "2/3".into()),
                ("TRACKNUMBER".into(), "07/12".into()),
            ],
            &mut metadata,
        );
        assert_eq!(metadata.title.as_deref(), Some("A Title"));
        assert_eq!(metadata.artist, "An Artist");
        assert_eq!(metadata.album, "An Album");
        assert_eq!(metadata.album_artist, "Album Artist");
        assert_eq!(metadata.genre, "Jazz");
        assert_eq!(metadata.year, Some(1975));
        assert_eq!(metadata.disc, Some(2));
        assert_eq!(metadata.number, Some(7));
    }

    #[test]
    fn source_open_routes_the_wv_extension_to_wavpack() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("routed.wv");
        write_wv(&path, vec![1_000, -1_000], 16, false);
        let source = Source::open(&path).unwrap();
        assert_eq!(source.info.codec, "WavPack");
        assert_eq!(
            source.duration,
            Some(Duration::from_secs_f64(2.0 / 48_000.0))
        );
    }

    #[test]
    fn metadata_survives_a_real_file_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("tagged.wv");
        write_wv(&path, vec![0, 0], 16, false);
        let metadata = metadata(&path).expect("WavPack metadata");
        assert!(metadata.title.is_none());
        let info = crate::library::TrackInfo::read(&path);
        assert_eq!(info.duration, Some(Duration::from_secs_f64(2.0 / 48_000.0)));
    }
}
