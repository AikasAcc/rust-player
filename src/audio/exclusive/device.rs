use cpal::traits::{DeviceTrait, HostTrait};
use std::ptr;
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, S_OK},
        Media::Audio::{
            AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED, AUDCLNT_E_UNSUPPORTED_FORMAT,
            AUDCLNT_SHAREMODE_EXCLUSIVE, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, IAudioClient,
            IAudioRenderClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEX,
            WAVEFORMATEXTENSIBLE, WAVEFORMATEXTENSIBLE_0,
        },
        System::{
            Com::{
                CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoTaskMemFree, CoUninitialize,
            },
            Threading::CreateEventW,
        },
    },
    core::{GUID, HSTRING, PCWSTR},
};

use super::OpenError;
use crate::audio::{
    OutputFormat, OutputMode, OutputOptions,
    decoder::{SampleKind, SourceFormat},
    pcm::{Encoding, PCM16, PCM24, PCM24_IN_32, PCM32, exact_encodings},
    shared,
};

const WAVE_PCM: u16 = 1;
const WAVE_FLOAT: u16 = 3;
const WAVE_EXTENSIBLE: u16 = 0xfffe;
const PCM_GUID: GUID = GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71);
const FLOAT_GUID: GUID = GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);

pub struct Apartment;

impl Apartment {
    pub fn new() -> windows::core::Result<Self> {
        // SAFETY: The owning thread initializes and releases COM before it exits.
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        }
        Ok(Self)
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: The successful initialization on this thread is balanced exactly once.
        unsafe {
            CoUninitialize();
        }
    }
}

pub struct Event(pub HANDLE);

impl Event {
    fn new() -> windows::core::Result<Self> {
        // SAFETY: An unnamed event with default security has no borrowed parameters.
        unsafe { CreateEventW(None, false, false, PCWSTR::null()).map(Self) }
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        // SAFETY: The event is owned here and is closed after releasing its audio client.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

#[derive(Clone, Copy, Debug)]
struct Layout {
    rate: u32,
    channels: u16,
    mask: u32,
}

#[derive(Clone, Copy)]
struct Format {
    layout: Layout,
    encoding: Encoding,
    legacy: bool,
    direct: Option<SampleKind>,
}

impl Format {
    fn wave(self) -> WAVEFORMATEXTENSIBLE {
        let subtype = match self.encoding {
            Encoding::Float32 => FLOAT_GUID,
            Encoding::Integer { .. } => PCM_GUID,
        };
        let tag = if !self.legacy {
            WAVE_EXTENSIBLE
        } else if subtype == FLOAT_GUID {
            WAVE_FLOAT
        } else {
            WAVE_PCM
        };
        let block = self.layout.channels * (self.encoding.bits() / 8);
        WAVEFORMATEXTENSIBLE {
            Format: WAVEFORMATEX {
                wFormatTag: tag,
                nChannels: self.layout.channels,
                nSamplesPerSec: self.layout.rate,
                nAvgBytesPerSec: self.layout.rate * u32::from(block),
                nBlockAlign: block,
                wBitsPerSample: self.encoding.bits(),
                cbSize: if self.legacy { 0 } else { 22 },
            },
            Samples: WAVEFORMATEXTENSIBLE_0 {
                wValidBitsPerSample: self.encoding.valid_bits(),
            },
            dwChannelMask: self.layout.mask,
            SubFormat: subtype,
        }
    }

    fn can_use_legacy(self) -> bool {
        matches!((self.layout.channels, self.layout.mask), (1, 4) | (2, 3))
            && self.encoding.bits() == self.encoding.valid_bits()
    }
}

pub struct Native {
    pub renderer: IAudioRenderClient,
    pub client: IAudioClient,
    pub event: Event,
    pub encoding: Encoding,
    pub output: OutputFormat,
    pub frames: u32,
}

impl Native {
    pub fn new(
        preferred: Option<&str>,
        source: &SourceFormat,
        options: OutputOptions,
    ) -> Result<Self, OpenError> {
        let (id, name, device) = endpoint(preferred)?;
        let client = activate(&device)?;
        let format = negotiate(&client, source, options)?;
        let (client, frames) = initialize(&device, client, format)?;
        let event = Event::new()?;
        // SAFETY: The initialized event client borrows this event until the client is released.
        unsafe {
            client.SetEventHandle(event.0)?;
        }
        // SAFETY: A render service is requested from an initialized output client.
        let renderer = unsafe { client.GetService::<IAudioRenderClient>()? };
        let output = OutputFormat {
            id,
            device: name,
            rate: format.layout.rate,
            channels: format.layout.channels,
            sample: format.encoding.label().into(),
            buffer_frames: Some(frames),
            mode: OutputMode::Exclusive,
            direct: format.direct,
            software_gain: options.software_gain,
            transition_fades: options.transition_fades,
            dither: format.direct.is_none() && format.encoding != Encoding::Float32,
            effects: Default::default(),
        };
        Ok(Self {
            renderer,
            client,
            event,
            encoding: format.encoding,
            output,
            frames,
        })
    }
}

fn endpoint(preferred: Option<&str>) -> Result<(String, String, IMMDevice), OpenError> {
    let host = cpal::default_host();
    let device = preferred
        .and_then(|id| id.parse().ok())
        .and_then(|id| {
            host.output_devices()
                .ok()?
                .find(|device| device.id().ok().as_ref() == Some(&id))
        })
        .or_else(|| host.default_output_device())
        .ok_or_else(|| OpenError::Setup("No Windows output device is available".into()))?;
    let id = device
        .id()
        .map_err(|error| OpenError::Setup(error.to_string()))?;
    // The caller owns the COM apartment for the lifetime of the endpoint.
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let endpoint = unsafe { enumerator.GetDevice(&HSTRING::from(id.id()))? };
    Ok((id.to_string(), shared::label(&device), endpoint))
}

pub(in crate::audio) fn exclusive_formats(
    preferred: Option<&str>,
    rate: u32,
) -> Result<Vec<(u32, Vec<u16>)>, String> {
    let run = || -> Result<_, OpenError> {
        let _apartment = Apartment::new()?;
        let (_, _, device) = endpoint(preferred)?;
        let client = activate(&device)?;
        let mut candidates = vec![
            44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800, 384_000, rate,
        ];
        candidates.sort_unstable();
        candidates.dedup();
        let mut formats = Vec::new();
        for rate in candidates {
            let layout = Layout {
                rate,
                channels: 2,
                mask: 3,
            };
            let mut depths = Vec::new();
            for bits in [16, 24, 32] {
                let kind = SampleKind::Integer(bits);
                for &encoding in exact_encodings(kind) {
                    let format = Format {
                        layout,
                        encoding,
                        legacy: false,
                        direct: Some(kind),
                    };
                    if probe(&client, format)?.is_some() {
                        depths.push(bits);
                        break;
                    }
                }
            }
            if !depths.is_empty() {
                formats.push((rate, depths));
            }
        }
        Ok(formats)
    };
    run().map_err(OpenError::message)
}

fn activate(device: &IMMDevice) -> windows::core::Result<IAudioClient> {
    // SAFETY: Runs on the same initialized apartment as the render endpoint.
    unsafe { device.Activate(CLSCTX_ALL, None) }
}

fn initialize(
    device: &IMMDevice,
    mut client: IAudioClient,
    format: Format,
) -> windows::core::Result<(IAudioClient, u32)> {
    let wave = format.wave();
    let mut period = 0;
    // SAFETY: The output pointer is writable and used synchronously.
    unsafe {
        client.GetDevicePeriod(Some(&mut period), None)?;
    }
    let mut period = period.max(100_000);
    // SAFETY: The wave header and its extension remain live for Initialize.
    let open = |client: &IAudioClient, period| unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_EXCLUSIVE,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            period,
            period,
            ptr::addr_of!(wave.Format),
            None,
        )
    };
    if let Err(error) = open(&client, period) {
        if error.code() != AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED {
            return Err(error);
        }
        // SAFETY: WASAPI exposes the aligned size on this specific Initialize error.
        let aligned = unsafe { client.GetBufferSize()? };
        period = ((u64::from(aligned) * 10_000_000 + u64::from(format.layout.rate) / 2)
            / u64::from(format.layout.rate)) as i64;
        drop(client);
        client = activate(device)?;
        open(&client, period)?;
    }
    // SAFETY: The client has been successfully initialized.
    let frames = unsafe { client.GetBufferSize()? };
    Ok((client, frames))
}

fn negotiate(
    client: &IAudioClient,
    source: &SourceFormat,
    options: OutputOptions,
) -> Result<Format, OpenError> {
    let native = Layout {
        rate: source.rate,
        channels: source.channels,
        mask: source.mask,
    };
    for &encoding in exact_encodings(source.kind) {
        let direct = (!options.software_gain
            && !options.replay_gain.enabled()
            && !options.effects_requested)
            .then_some(source.kind);
        if let Some(format) = probe(
            client,
            Format {
                layout: native,
                encoding,
                legacy: false,
                direct,
            },
        )? {
            return Ok(format);
        }
    }
    if options.convert_exclusive {
        const CONVERTED: [Encoding; 5] = [Encoding::Float32, PCM32, PCM24_IN_32, PCM24, PCM16];
        let mix = mix_layout(client);
        let same_channels = mix.map(|mix| Layout {
            rate: mix.rate,
            ..native
        });
        for layout in [Some(native), same_channels, mix].into_iter().flatten() {
            if options.reject_downmix && layout.channels < native.channels {
                continue;
            }
            if layout.channels == native.channels && layout.mask != native.mask {
                continue;
            }
            for encoding in CONVERTED {
                if let Some(format) = probe(
                    client,
                    Format {
                        layout,
                        encoding,
                        legacy: false,
                        direct: None,
                    },
                )? {
                    return Ok(format);
                }
            }
        }
    }
    let pcm = match source.kind {
        SampleKind::Integer(bits) => format!("{bits}-bit integer PCM"),
        SampleKind::Float32 => "32-bit float PCM".into(),
        SampleKind::Float64 => "64-bit float PCM".into(),
    };
    Err(OpenError::Setup(format!(
        "No supported WASAPI Exclusive format for {} Hz, {} channels and {pcm}",
        source.rate, source.channels
    )))
}

fn probe(client: &IAudioClient, mut format: Format) -> Result<Option<Format>, OpenError> {
    let supported = |format: Format| {
        let wave = format.wave();
        // SAFETY: Exclusive probing uses a valid live wave format and no closest-match pointer.
        let status = unsafe {
            client.IsFormatSupported(
                AUDCLNT_SHAREMODE_EXCLUSIVE,
                ptr::addr_of!(wave.Format),
                None,
            )
        };
        match status {
            S_OK => Ok(true),
            AUDCLNT_E_UNSUPPORTED_FORMAT => Ok(false),
            status if status.is_err() => Err(OpenError::Driver(status)),
            _ => Ok(false),
        }
    };
    if supported(format)? {
        return Ok(Some(format));
    }
    if format.can_use_legacy() {
        format.legacy = true;
        if supported(format)? {
            return Ok(Some(format));
        }
    }
    Ok(None)
}

fn mix_layout(client: &IAudioClient) -> Option<Layout> {
    // SAFETY: The task-allocated format contains a WAVEFORMATEX header. Access the
    // extension only if the tag and size guarantee its presence.
    unsafe {
        let address = client.GetMixFormat().ok()?;
        if address.is_null() {
            return None;
        }
        let header = address.read_unaligned();
        let mask = if header.wFormatTag == WAVE_EXTENSIBLE && header.cbSize >= 22 {
            address
                .cast::<WAVEFORMATEXTENSIBLE>()
                .read_unaligned()
                .dwChannelMask
        } else {
            match header.nChannels {
                1 => 4,
                2 => 3,
                _ => 0,
            }
        };
        CoTaskMemFree(Some(address.cast()));
        let layout = Layout {
            rate: header.nSamplesPerSec,
            channels: header.nChannels,
            mask,
        };
        ((1..=32).contains(&layout.channels)
            && (8_000..=768_000).contains(&layout.rate)
            && mask.count_ones() == u32::from(layout.channels))
        .then_some(layout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surround_wave_format_identifies_speaker_order_and_valid_bits() {
        let format = Format {
            layout: Layout {
                rate: 96_000,
                channels: 6,
                mask: 0x3f,
            },
            encoding: PCM24_IN_32,
            legacy: false,
            direct: Some(SampleKind::Integer(24)),
        };
        let wave = format.wave();
        let base = wave.Format;
        // SAFETY: Format::wave initializes the valid-precision member.
        let valid = unsafe { wave.Samples.wValidBitsPerSample };
        let actual = (
            base.nChannels,
            base.nSamplesPerSec,
            base.wBitsPerSample,
            valid,
            wave.dwChannelMask,
            base.nBlockAlign,
            base.nAvgBytesPerSec,
            base.wFormatTag,
        );
        assert_eq!(
            actual,
            (6, 96_000, 32, 24, 0x3f, 24, 2_304_000, WAVE_EXTENSIBLE)
        );
        assert!(!format.can_use_legacy());
    }
}
