use cpal::{
    Device, ErrorKind, FromSample, SampleFormat, SizedSample, Stream, StreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use rtrb::Consumer;
use std::sync::{Arc, atomic::Ordering::Relaxed};

use super::{Backend, OutputFormat, OutputMode, RenderState, pcm::random};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceEntry {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct Devices {
    pub output: Vec<DeviceEntry>,
    pub default: Option<String>,
}

impl Devices {
    pub fn selected(&self, preferred: Option<&str>) -> Option<&str> {
        preferred
            .and_then(|id| self.output.iter().find(|device| device.id == id))
            .map(|device| device.id.as_str())
            .or(self.default.as_deref())
    }
}

pub fn devices() -> Result<Devices, String> {
    let host = cpal::default_host();
    let default = host
        .default_output_device()
        .and_then(|device| device.id().ok())
        .map(|id| id.to_string());
    let mut output: Vec<_> = host
        .output_devices()
        .map_err(|error| error.to_string())?
        .filter_map(|device| {
            Some(DeviceEntry {
                id: device.id().ok()?.to_string(),
                label: label(&device),
            })
        })
        .collect();
    output.sort_by(|left, right| left.label.cmp(&right.label).then(left.id.cmp(&right.id)));
    Ok(Devices { output, default })
}

pub(super) fn label(device: &Device) -> String {
    device.description().map_or_else(
        |_| "Audio output".into(),
        |description| description.name().to_owned(),
    )
}

pub struct Shared {
    device: Device,
    config: StreamConfig,
    sample: SampleFormat,
    output: OutputFormat,
}

impl Shared {
    pub fn open(preferred: Option<&str>) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = preferred
            .and_then(|id| id.parse().ok())
            .and_then(|id| host.device_by_id(&id))
            .filter(|device| device.default_output_config().is_ok())
            .or_else(|| host.default_output_device())
            .ok_or("No Windows output device is available")?;
        let supported = device
            .default_output_config()
            .map_err(|error| error.to_string())?;
        let sample = supported.sample_format();
        let config = supported.config();
        if !(1..=32).contains(&config.channels) || !(8_000..=768_000).contains(&config.sample_rate)
        {
            return Err("The Windows device uses an unsupported mix format".into());
        }
        let output = OutputFormat {
            id: device.id().map_err(|error| error.to_string())?.to_string(),
            device: label(&device),
            rate: config.sample_rate,
            channels: config.channels,
            sample: sample.to_string(),
            buffer_frames: None,
            dither: !matches!(sample, SampleFormat::F32 | SampleFormat::F64),
            mode: OutputMode::Shared,
            direct: None,
            software_gain: true,
            transition_fades: false,
            effects: Default::default(),
        };
        Ok(Self {
            device,
            config,
            sample,
            output,
        })
    }

    fn stream<T: SizedSample + FromSample<f64> + 'static>(
        &self,
        mut render: Renderer,
    ) -> Result<Stream, String> {
        let state = Arc::clone(&render.state);
        self.device
            .build_output_stream(
                self.config,
                move |buffer: &mut [T], _| render.fill(buffer),
                move |error| match error.kind() {
                    ErrorKind::RealtimeDenied => state.priority_denied.store(true, Relaxed),
                    ErrorKind::Xrun => {
                        state.underruns.fetch_add(1, Relaxed);
                    }
                    _ => state.output_lost.store(true, Relaxed),
                },
                None,
            )
            .map_err(|error| error.to_string())
    }
}

impl Backend for Shared {
    type Stream = Stream;

    fn format(&self) -> &OutputFormat {
        &self.output
    }

    fn start(
        self,
        consumer: Consumer<u32>,
        state: Arc<RenderState>,
    ) -> Result<(Stream, Option<u32>), String> {
        let dither = self
            .output
            .dither
            .then(|| 2_f64.powi(1 - self.sample.bits_per_sample() as i32));
        let renderer = Renderer {
            consumer,
            state,
            channels: usize::from(self.config.channels),
            dither,
            rng: 0xd6e8feb8,
        };
        let stream = match self.sample {
            SampleFormat::F32 => self.stream::<f32>(renderer),
            SampleFormat::F64 => self.stream::<f64>(renderer),
            SampleFormat::I16 => self.stream::<i16>(renderer),
            SampleFormat::I24 => self.stream::<cpal::I24>(renderer),
            SampleFormat::I32 => self.stream::<i32>(renderer),
            SampleFormat::U8 => self.stream::<u8>(renderer),
            other => Err(format!("Unsupported WASAPI sample format: {other}")),
        }?;
        let frames = stream.buffer_size().ok();
        stream.play().map_err(|error| error.to_string())?;
        Ok((stream, frames))
    }
}

pub(super) struct Renderer {
    pub consumer: Consumer<u32>,
    pub state: Arc<RenderState>,
    pub channels: usize,
    pub dither: Option<f64>,
    pub rng: u32,
}

impl Renderer {
    pub fn fill<T: SizedSample + FromSample<f64>>(&mut self, output: &mut [T]) {
        use std::sync::atomic::Ordering::{Acquire, Release};
        if self.state.drain.load(Acquire) {
            let pending = self.consumer.slots();
            if let Ok(chunk) = self.consumer.read_chunk(pending) {
                chunk.commit_all();
            }
            self.state.drain.store(false, Release);
        }
        let audible = self.state.ready.load(Acquire) && !self.state.paused.load(Relaxed);
        if !audible {
            output.fill(T::from_sample(0.0));
            return;
        }

        // Acquire EOF before counting available samples, so the last writes are visible.
        let eof = self.state.eof.load(Acquire);
        let count = output.len().min(self.consumer.slots()) / self.channels * self.channels;
        let gain = f64::from(f32::from_bits(self.state.volume.load(Relaxed)));
        if let Ok(chunk) = self.consumer.read_chunk(count) {
            let (head, tail) = chunk.as_slices();
            for (slot, sample) in output.iter_mut().zip(head.iter().chain(tail)) {
                let scaled = f64::from(f32::from_bits(*sample)) * gain;
                let value = if let Some(lsb) = self.dither
                    && gain != 0.0
                {
                    (scaled / lsb + random(&mut self.rng) - random(&mut self.rng)).round() * lsb
                } else {
                    scaled
                };
                *slot = T::from_sample(value.clamp(-1.0, 1.0 - f64::EPSILON));
            }
            chunk.commit_all();
        }
        output[count..].fill(T::from_sample(0.0));
        let frames = (count / self.channels) as u64;
        self.state.listened_frames.fetch_add(frames, Relaxed);
        self.state.frames.fetch_add(frames, Release);
        if eof && self.consumer.is_empty() {
            self.state.drained.store(true, Release);
        } else if count < output.len() {
            self.state.underruns.fetch_add(1, Relaxed);
        }
    }
}
