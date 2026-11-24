use rtrb::Consumer;
use std::{
    slice,
    sync::atomic::{AtomicBool, Ordering::*},
    thread,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{E_POINTER, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
        Media::Audio::{AUDCLNT_E_BUFFER_ERROR, AUDCLNT_E_RESOURCES_INVALIDATED},
        System::Threading::{
            AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, WaitForSingleObject,
        },
    },
    core::{Error, w},
};

use super::device::Native;
use crate::audio::{
    RenderState,
    pcm::{normalized, random},
};
use crate::audio::{decoder::SampleKind, pcm::Encoding};

struct Priority(HANDLE);

impl Priority {
    fn pro_audio() -> windows::core::Result<Self> {
        let mut index = 0;
        // SAFETY: MMCSS registration and the returned handle belong to this audio thread.
        unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut index).map(Self) }
    }
}

impl Drop for Priority {
    fn drop(&mut self) {
        // SAFETY: The handle is reverted on the thread that requested MMCSS.
        let _ = unsafe { AvRevertMmThreadCharacteristics(self.0) };
    }
}

pub fn play(
    native: &Native,
    consumer: Consumer<u32>,
    state: &RenderState,
    quit: &AtomicBool,
) -> windows::core::Result<()> {
    let _priority = Priority::pro_audio().ok();
    state.priority_denied.store(_priority.is_none(), Relaxed);
    let mut render = Renderer {
        native,
        state,
        consumer,
        rng: 0x9e3779b9,
        last: [0.0; 32],
    };
    let mut running = false;
    let mut woken = Instant::now();
    loop {
        let closing = quit.load(Acquire) || state.closed.load(Acquire);
        let seek = state.drain.load(Acquire);
        let paused = state.paused.load(Relaxed);
        if !running {
            if closing {
                return Ok(());
            }
            if seek {
                render.discard();
            }
            if !paused && state.ready.load(Acquire) && !seek {
                render.packet()?;
                // SAFETY: The initialized event stream has a full first render buffer.
                unsafe {
                    native.client.Start()?;
                }
                state.running.store(true, Release);
                running = true;
                woken = Instant::now();
            } else {
                thread::sleep(Duration::from_millis(4));
            }
            continue;
        }

        let ready = wait(native, 40)?;
        if closing || seek || paused {
            if native.output.transition_fades && ready {
                render.fade()?;
                // Allow the single ramp packet to reach the device before resetting.
                let _ = wait(native, 100)?;
            }
            state.running.store(false, Release);
            // SAFETY: Stop and Reset are ordered on the owner thread, with no packet acquired.
            unsafe {
                native.client.Stop()?;
                native.client.Reset()?;
            }
            if seek {
                render.discard();
            }
            if closing {
                return Ok(());
            }
            running = false;
            continue;
        }
        if !ready {
            if woken.elapsed() >= Duration::from_secs(2) {
                return Err(Error::from(AUDCLNT_E_RESOURCES_INVALIDATED));
            }
            continue;
        }
        woken = Instant::now();
        if let Err(error) = render.packet() {
            if error.code() != AUDCLNT_E_BUFFER_ERROR {
                return Err(error);
            }
            state.underruns.fetch_add(1, Relaxed);
        }
    }
}

fn wait(native: &Native, milliseconds: u32) -> windows::core::Result<bool> {
    // SAFETY: The event belongs to the live WASAPI client on this thread.
    match unsafe { WaitForSingleObject(native.event.0, milliseconds) } {
        WAIT_OBJECT_0 => Ok(true),
        WAIT_TIMEOUT => Ok(false),
        _ => Err(Error::from_win32()),
    }
}

struct Renderer<'a> {
    native: &'a Native,
    state: &'a RenderState,
    consumer: Consumer<u32>,
    rng: u32,
    last: [f64; 32],
}

impl Renderer<'_> {
    fn discard(&mut self) {
        let count = self.consumer.slots();
        if let Ok(chunk) = self.consumer.read_chunk(count) {
            chunk.commit_all();
        }
        self.state.drain.store(false, Release);
    }

    fn packet(&mut self) -> windows::core::Result<()> {
        let native = self.native;
        let size = native.encoding.width();
        let channels = usize::from(native.output.channels);
        let eof = self.state.eof.load(Acquire);
        let needed = native.frames as usize * channels;
        let count = needed.min(self.consumer.slots()) / channels * channels;
        let gain = if native.output.software_gain {
            f64::from(f32::from_bits(self.state.volume.load(Relaxed)))
        } else {
            1.0
        };
        let direct = native.output.direct;
        let dither = native.output.dither && gain != 0.0;
        // WASAPI supplies one full event buffer; padding is zero-valued PCM.
        packet(native, |bytes| {
            bytes.fill(0);
            self.last.fill(0.0);
            if let Ok(chunk) = self.consumer.read_chunk(count) {
                let (head, tail) = chunk.as_slices();
                for (i, (slot, &sample)) in bytes
                    .chunks_exact_mut(size)
                    .zip(head.iter().chain(tail))
                    .enumerate()
                {
                    let normalized = || normalized(sample, direct) * gain;
                    let word = if direct.is_some() {
                        native.encoding.word(sample)
                    } else {
                        let noise = if dither {
                            random(&mut self.rng) - random(&mut self.rng)
                        } else {
                            0.0
                        };
                        native.encoding.converted(normalized(), noise)
                    };
                    native.encoding.write(slot, word);
                    if native.output.transition_fades && count == needed && i >= count - channels {
                        self.last[i % channels] = normalized();
                    }
                }
                chunk.commit_all();
            }
        })?;
        let frames = (count / channels) as u64;
        self.state.listened_frames.fetch_add(frames, Relaxed);
        self.state.frames.fetch_add(frames, Release);
        if eof && self.consumer.is_empty() {
            self.state.drained.store(true, Release);
        } else if count < needed {
            self.state.underruns.fetch_add(1, Relaxed);
        }
        Ok(())
    }

    fn fade(&self) -> windows::core::Result<()> {
        let channels = usize::from(self.native.output.channels);
        let ramp = (self.native.output.rate as usize * 3 / 1000).min(self.native.frames as usize);
        packet(self.native, |bytes| {
            bytes.fill(0);
            for (i, slot) in bytes
                .chunks_exact_mut(self.native.encoding.width())
                .take(ramp * channels)
                .enumerate()
            {
                let scale = (ramp - 1 - i / channels) as f64 / ramp.max(1) as f64;
                let sample = self.last[i % channels] * scale;
                self.native
                    .encoding
                    .write(slot, self.native.encoding.converted(sample, 0.0));
            }
        })
    }
}

fn packet(native: &Native, fill: impl FnOnce(&mut [u8])) -> windows::core::Result<()> {
    // SAFETY: Both calls are on the owning thread. The slice size follows the
    // negotiated block alignment and exists only until the same buffer is released.
    unsafe {
        let address = native.renderer.GetBuffer(native.frames)?;
        if address.is_null() {
            native.renderer.ReleaseBuffer(native.frames, 2)?;
            return Err(Error::from(E_POINTER));
        }
        let length =
            native.frames as usize * usize::from(native.output.channels) * native.encoding.width();
        fill(slice::from_raw_parts_mut(address, length));
        native.renderer.ReleaseBuffer(native.frames, 0)
    }
}

pub(crate) fn write_exact(
    encoding: Encoding,
    kind: SampleKind,
    pcm: &[u32],
    output: &mut impl std::io::Write,
) -> std::io::Result<()> {
    assert!(matches!(
        (encoding, kind),
        (Encoding::Integer { .. }, SampleKind::Integer(_))
            | (Encoding::Float32, SampleKind::Float32)
    ));
    let mut sample = [0u8; 4];
    for &value in pcm {
        let bytes = &mut sample[..encoding.width()];
        encoding.write(bytes, encoding.word(value));
        output.write_all(bytes)?;
    }
    Ok(())
}
