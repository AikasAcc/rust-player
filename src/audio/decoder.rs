use super::OutputOptions;
use std::{borrow::Cow, fs::File, io::ErrorKind, path::Path, time::Duration};
use symphonia::{
    core::{
        audio::{AudioBufferRef, SampleBuffer},
        codecs::{CODEC_TYPE_FLAC, CODEC_TYPE_NULL, Decoder, DecoderOptions},
        conv::ConvertibleSample,
        errors::Error,
        formats::{FormatOptions, FormatReader, SeekMode, SeekTo, Track},
        io::{MediaSourceStream, MediaSourceStreamOptions},
        meta::MetadataOptions,
        probe::Hint,
        units::{Time, TimeBase},
    },
    default::{get_codecs, get_probe},
};

#[derive(Clone, Debug)]
pub struct SourceFormat {
    pub codec: &'static str,
    pub rate: u32,
    pub channels: u16,
    pub mask: u32,
    pub bits: Option<u32>,
    pub kind: SampleKind,
    pub gain: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleKind {
    Integer(u16),
    Float32,
    Float64,
}

impl SampleKind {
    pub fn label(self) -> String {
        match self {
            Self::Integer(bits) => format!("{bits}-bit integer PCM"),
            Self::Float32 => "32-bit float PCM".into(),
            Self::Float64 => "64-bit float PCM".into(),
        }
    }

    fn decoded(buffer: &AudioBufferRef<'_>, valid: Option<u32>) -> Self {
        use AudioBufferRef::*;
        let width: u16 = match buffer {
            U8(_) | S8(_) => 8,
            U16(_) | S16(_) => 16,
            U24(_) | S24(_) => 24,
            U32(_) | S32(_) => 32,
            F32(_) => return Self::Float32,
            F64(_) => return Self::Float64,
        };
        let bits = valid.filter(|&bits| (1..=u32::from(width)).contains(&bits));
        Self::Integer(bits.map_or(width, |bits| bits as u16))
    }
}

fn open_format(path: &Path) -> Result<Box<dyn FormatReader>, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    let stream = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|extension| extension.to_str()) {
        hint.with_extension(extension);
    }
    get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions {
                enable_gapless: true,
                ..FormatOptions::default()
            },
            &MetadataOptions::default(),
        )
        .map(|probe| probe.format)
        .map_err(|error| error.to_string())
}

fn audio_track(reader: &dyn FormatReader) -> Option<&Track> {
    // Matroska files may carry video alongside audio. Prefer a track whose
    // codec is actually compiled in, then fall back to the old "not null"
    // rule so a probe failure still produces the usual error message.
    let decodable = |track: &&Track| {
        track.codec_params.codec != CODEC_TYPE_NULL
            && get_codecs()
                .make(&track.codec_params, &DecoderOptions::default())
                .is_ok()
    };
    let present = |track: &&Track| track.codec_params.codec != CODEC_TYPE_NULL;
    reader
        .default_track()
        .filter(decodable)
        .or_else(|| reader.tracks().iter().find(decodable))
        .or_else(|| reader.default_track().filter(present))
        .or_else(|| reader.tracks().iter().find(present))
}

fn duration(frames: u64, base: TimeBase) -> Option<Duration> {
    let time = base.calc_time(frames);
    Duration::try_from_secs_f64(time.seconds as f64 + time.frac).ok()
}

pub fn container_duration(path: &Path) -> Option<Duration> {
    let extension = path.extension().and_then(|extension| extension.to_str());
    if extension.is_some_and(|extension| extension.eq_ignore_ascii_case("ape")) {
        return super::ape::ApeSource::open(path)
            .ok()
            .and_then(|(_, _, duration)| duration);
    }
    if extension.is_some_and(|extension| {
        extension.eq_ignore_ascii_case("dsf") || extension.eq_ignore_ascii_case("dff")
    }) {
        return super::dsd::DsdSource::open(path)
            .ok()
            .and_then(|(_, _, duration)| duration);
    }
    if extension.is_some_and(|extension| extension.eq_ignore_ascii_case("wv")) {
        return super::wavpack::WavpackSource::open(path)
            .ok()
            .and_then(|(_, _, duration)| duration);
    }
    let reader = open_format(path).ok()?;
    let params = &audio_track(reader.as_ref())?.codec_params;
    let frames = params.n_frames.filter(|&frames| frames > 0)?;
    let base = params.time_base.or_else(|| {
        params
            .sample_rate
            .filter(|&rate| rate > 0)
            .map(|rate| TimeBase::new(1, rate))
    })?;
    duration(frames, base)
}

struct SymphoniaSource {
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    id: u32,
    time_base: Option<TimeBase>,
    skipped_frames: u64,
    initial_pending: bool,
    verifying: bool,
    start_ts: u64,
}

impl SymphoniaSource {
    fn seek(&mut self, position: Duration, info: &SourceFormat, cue: bool) -> Result<(), String> {
        let time = Time::new(position.as_secs(), f64::from(position.subsec_nanos()) / 1e9);
        let target = if let Some(base) = self.time_base.filter(|_| cue) {
            let frame = u128::from(frames_for(position, info.rate));
            let ts =
                frame * u128::from(base.denom) / (u128::from(base.numer) * u128::from(info.rate));
            SeekTo::TimeStamp {
                ts: (ts.min(u128::from(u64::MAX)) as u64).saturating_add(self.start_ts),
                track_id: self.id,
            }
        } else {
            SeekTo::Time {
                time,
                track_id: Some(self.id),
            }
        };
        let seek = self
            .reader
            .seek(SeekMode::Accurate, target)
            .map_err(|error| error.to_string())?;
        self.decoder.reset();
        self.verifying = false;
        self.initial_pending = false;
        let ticks = seek.required_ts.saturating_sub(seek.actual_ts);
        self.skipped_frames = self.time_base.map_or(ticks, |base| {
            let time = base.calc_time(ticks);
            ((time.seconds as f64 + time.frac) * f64::from(info.rate)).round() as u64
        });
        Ok(())
    }

    fn next<'a, T: ConvertibleSample>(
        &mut self,
        info: &SourceFormat,
        scratch: &'a mut Option<SampleBuffer<T>>,
    ) -> Result<Option<&'a [T]>, String> {
        loop {
            let decoded = if std::mem::take(&mut self.initial_pending) {
                self.decoder.last_decoded()
            } else {
                let packet = match self.reader.next_packet() {
                    Ok(packet) => packet,
                    Err(Error::IoError(error)) if error.kind() == ErrorKind::UnexpectedEof => {
                        if std::mem::take(&mut self.verifying)
                            && self.decoder.finalize().verify_ok == Some(false)
                        {
                            return Err("FLAC PCM does not match its embedded MD5 checksum".into());
                        }
                        return Ok(None);
                    }
                    Err(error) => return Err(error.to_string()),
                };
                if packet.track_id() != self.id {
                    continue;
                }
                self.decoder
                    .decode(&packet)
                    .map_err(|error| error.to_string())?
            };
            if decoded.spec().rate != info.rate
                || decoded.spec().channels.bits() != info.mask
                || SampleKind::decoded(&decoded, info.bits) != info.kind
            {
                return Err("The audio format changed inside the track".into());
            }
            let skip = self.skipped_frames.min(decoded.frames() as u64) as usize;
            self.skipped_frames -= skip as u64;
            if skip == decoded.frames() {
                continue;
            }
            let capacity = decoded.capacity() * usize::from(info.channels);
            if scratch
                .as_ref()
                .is_none_or(|buffer| buffer.capacity() < capacity)
            {
                *scratch = Some(SampleBuffer::new(
                    decoded.capacity() as u64,
                    *decoded.spec(),
                ));
            }
            let buffer = scratch
                .as_mut()
                .ok_or("Could not initialize decoder buffer")?;
            buffer.copy_interleaved_ref(decoded);
            let start = skip * usize::from(info.channels);
            return Ok(Some(&buffer.samples()[start..]));
        }
    }
}

enum Inner {
    Symphonia(SymphoniaSource),
    Ape(Box<super::ape::ApeSource>),
    Dsd(Box<super::dsd::DsdSource>),
    Wavpack(Box<super::wavpack::WavpackSource>),
}

pub struct Source {
    inner: Inner,
    offset: Duration,
    remaining_frames: Option<u64>,
    pub info: SourceFormat,
    pub duration: Option<Duration>,
}

impl Source {
    #[cfg(test)]
    pub fn open(path: &Path) -> Result<Self, String> {
        Self::open_with_options(path, OutputOptions::default())
    }

    pub fn open_with_options(path: &Path, options: OutputOptions) -> Result<Self, String> {
        let cue = crate::cue::resolve(path)?;
        let audio = cue.as_ref().map_or(path, |track| track.audio.as_path());
        let extension = audio.extension().and_then(|extension| extension.to_str());
        let (inner, info, full_duration) = if extension
            .is_some_and(|extension| extension.eq_ignore_ascii_case("ape"))
        {
            let (source, format, duration) = super::ape::ApeSource::open(audio)?;
            (Inner::Ape(Box::new(source)), format, duration)
        } else if extension.is_some_and(|extension| {
            extension.eq_ignore_ascii_case("dsf") || extension.eq_ignore_ascii_case("dff")
        }) {
            let (source, format, duration) = super::dsd::DsdSource::open(audio)?;
            (Inner::Dsd(Box::new(source)), format, duration)
        } else if extension.is_some_and(|extension| extension.eq_ignore_ascii_case("wv")) {
            let (source, format, duration) = super::wavpack::WavpackSource::open(audio)?;
            (Inner::Wavpack(Box::new(source)), format, duration)
        } else {
            let mut reader = open_format(audio)?;
            let track = audio_track(reader.as_ref()).ok_or("No supported audio track")?;
            let id = track.id;
            let params = track.codec_params.clone();
            let verifying = options.verify_flac && params.codec == CODEC_TYPE_FLAC && cue.is_none();
            let mut decoder = get_codecs()
                .make(&params, &DecoderOptions { verify: verifying })
                .map_err(|error| error.to_string())?;
            // Inspect a decoded packet. Container bits and the decoder sample type can differ.
            let (spec, kind) = loop {
                let packet = reader.next_packet().map_err(|error| error.to_string())?;
                if packet.track_id() != id {
                    continue;
                }
                let decoded = decoder.decode(&packet).map_err(|error| error.to_string())?;
                if decoded.frames() == 0 {
                    continue;
                }
                let kind = SampleKind::decoded(&decoded, params.bits_per_sample);
                break (*decoded.spec(), kind);
            };
            let rate = spec.rate;
            if !(1..=768_000).contains(&rate) {
                return Err("Unsupported source sample rate".into());
            }
            let channels = spec.channels.count() as u16;
            if !(1..=32).contains(&channels) {
                return Err("Unsupported source channel count".into());
            }
            let time_base = params.time_base.or_else(|| Some(TimeBase::new(1, rate)));
            let full_duration = params
                .n_frames
                .zip(time_base)
                .and_then(|(frames, base)| duration(frames, base));
            let info = SourceFormat {
                codec: get_codecs()
                    .get_codec(params.codec)
                    .map_or("Unknown", |desc| desc.short_name),
                rate,
                channels,
                mask: spec.channels.bits(),
                bits: params.bits_per_sample,
                kind,
                gain: options.replay_gain.factor(audio),
            };
            (
                Inner::Symphonia(SymphoniaSource {
                    reader,
                    decoder,
                    id,
                    time_base,
                    skipped_frames: 0,
                    initial_pending: true,
                    verifying,
                    start_ts: params.start_ts,
                }),
                info,
                full_duration,
            )
        };
        let offset = cue
            .as_ref()
            .map_or(Duration::ZERO, |track| crate::cue::duration(track.start));
        let duration = if let Some(track) = &cue {
            track
                .end
                .map(crate::cue::duration)
                .map(|end| full_duration.map_or(end, |full| end.min(full)))
                .or(full_duration)
                .map(|end| end.saturating_sub(offset))
        } else {
            full_duration
        };
        if cue.is_some() && duration.is_some_and(|duration| duration.is_zero()) {
            return Err("CUE track starts beyond the audio file".into());
        }
        let mut source = Self {
            inner,
            offset,
            remaining_frames: cue
                .as_ref()
                .and_then(|track| track.end)
                .zip(duration)
                .map(|(_, duration)| frames_for(duration, info.rate)),
            info,
            duration,
        };
        // APE and WavPack take absolute sample positions; CUE offsets are
        // applied by the common `Source::seek` path.
        if !offset.is_zero() {
            source.seek(Duration::ZERO)?;
        }
        Ok(source)
    }

    pub fn seek(&mut self, position: Duration) -> Result<(), String> {
        let cue = self.remaining_frames.is_some() || !self.offset.is_zero();
        if self.remaining_frames.is_some() {
            self.remaining_frames = self
                .duration
                .map(|duration| frames_for(duration.saturating_sub(position), self.info.rate));
        }
        let position = position.saturating_add(self.offset);
        match &mut self.inner {
            Inner::Symphonia(source) => source.seek(position, &self.info, cue),
            Inner::Ape(source) => source.seek(position),
            Inner::Dsd(source) => source.seek(position),
            Inner::Wavpack(source) => source.seek(position),
        }
    }

    pub fn next<'a, T: ConvertibleSample>(
        &mut self,
        scratch: &'a mut Option<SampleBuffer<T>>,
    ) -> Result<Option<Cow<'a, [T]>>, String> {
        if self.remaining_frames == Some(0) {
            return Ok(None);
        }
        let samples = match &mut self.inner {
            Inner::Symphonia(source) => source.next(&self.info, scratch)?.map(Cow::Borrowed),
            Inner::Ape(source) => source.next()?.map(Cow::Owned),
            Inner::Dsd(source) => source.next()?.map(Cow::Owned),
            Inner::Wavpack(source) => source.next()?.map(Cow::Owned),
        };
        let Some(mut samples) = samples else {
            return Ok(None);
        };
        if let Some(remaining) = &mut self.remaining_frames {
            let channels = usize::from(self.info.channels);
            let take = ((samples.len() / channels) as u64).min(*remaining) as usize;
            *remaining -= take as u64;
            samples = match samples {
                Cow::Borrowed(samples) => Cow::Borrowed(&samples[..take * channels]),
                Cow::Owned(mut samples) => {
                    samples.truncate(take * channels);
                    Cow::Owned(samples)
                }
            };
        }
        Ok(Some(samples))
    }
}

pub(super) fn frames_for(duration: Duration, rate: u32) -> u64 {
    duration
        .as_secs()
        .saturating_mul(u64::from(rate))
        .saturating_add(
            (u64::from(duration.subsec_nanos()) * u64::from(rate) + 500_000_000) / 1_000_000_000,
        )
}
