mod device;
mod render;
pub(super) use device::exclusive_formats;
pub(super) use render::write_exact;

use rtrb::Consumer;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering::*},
        mpsc::{self, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows::{
    Win32::Media::Audio::{
        AUDCLNT_E_DEVICE_IN_USE, AUDCLNT_E_DEVICE_INVALIDATED,
        AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED, AUDCLNT_E_RESOURCES_INVALIDATED,
        AUDCLNT_E_SERVICE_NOT_RUNNING, AUDCLNT_E_UNSUPPORTED_FORMAT,
    },
    core::{Error, HRESULT},
};

use super::{Backend, OutputFormat, OutputOptions, RenderState, decoder::SourceFormat};

pub fn driver_message(code: HRESULT) -> String {
    match code {
        AUDCLNT_E_DEVICE_IN_USE => {
            "The device is in use. Close other audio applications or use Shared output".into()
        }
        AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED => {
            "Windows or the driver has disabled exclusive access for this device".into()
        }
        AUDCLNT_E_UNSUPPORTED_FORMAT => {
            "The device rejected the selected Exclusive PCM format".into()
        }
        AUDCLNT_E_DEVICE_INVALIDATED | AUDCLNT_E_RESOURCES_INVALIDATED => {
            "Windows disconnected or suspended the audio endpoint".into()
        }
        AUDCLNT_E_SERVICE_NOT_RUNNING => "The Windows Audio service is unavailable".into(),
        _ => format!(
            "WASAPI Exclusive error (0x{:08X}): {}",
            code.0 as u32,
            Error::from(code).message()
        ),
    }
}

fn device_lost(code: HRESULT) -> bool {
    matches!(
        code,
        AUDCLNT_E_DEVICE_INVALIDATED
            | AUDCLNT_E_RESOURCES_INVALIDATED
            | AUDCLNT_E_SERVICE_NOT_RUNNING
    )
}

enum OpenError {
    Driver(HRESULT),
    Setup(String),
}

impl OpenError {
    fn message(self) -> String {
        match self {
            Self::Driver(code) => driver_message(code),
            Self::Setup(message) => message,
        }
    }
}

impl From<Error> for OpenError {
    fn from(error: Error) -> Self {
        Self::Driver(error.code())
    }
}

pub struct Exclusive {
    output: OutputFormat,
    frames: u32,
    input: Option<SyncSender<Consumer<u32>>>,
    worker: Option<JoinHandle<()>>,
    quit: Arc<AtomicBool>,
}

impl Exclusive {
    pub fn prepare(
        device: Option<&str>,
        source: &SourceFormat,
        options: OutputOptions,
        state: Arc<RenderState>,
    ) -> Result<Self, String> {
        let (offer, setup) = mpsc::sync_channel(1);
        let (input, samples) = mpsc::sync_channel(1);
        let quit = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&quit);
        let source = source.clone();
        let device = device.map(str::to_owned);
        let worker = thread::Builder::new()
            .name("wasapi-exclusive".into())
            .spawn(move || {
                let _apartment = match device::Apartment::new() {
                    Ok(apartment) => apartment,
                    Err(error) => {
                        let _ = offer.send(Err(driver_message(error.code())));
                        return;
                    }
                };
                let deadline = Instant::now() + Duration::from_millis(500);
                let native = loop {
                    if state.closed.load(Acquire) || stop.load(Acquire) {
                        let _ = offer.send(Err("Exclusive output was closed".into()));
                        return;
                    }
                    match device::Native::new(device.as_deref(), &source, options) {
                        Ok(native) => break native,
                        // Drivers can finish releasing a previous Shared stream after it is dropped.
                        Err(OpenError::Driver(AUDCLNT_E_DEVICE_IN_USE))
                            if Instant::now() < deadline =>
                        {
                            thread::sleep(Duration::from_millis(40));
                        }
                        Err(error) => {
                            let _ = offer.send(Err(error.message()));
                            return;
                        }
                    }
                };
                if offer
                    .send(Ok((native.output.clone(), native.frames)))
                    .is_err()
                {
                    return;
                }
                let Ok(consumer) = samples.recv() else {
                    return;
                };
                if let Err(error) = render::play(&native, consumer, &state, &stop) {
                    if device_lost(error.code()) {
                        state.output_lost.store(true, Relaxed);
                    }
                    state.driver_error.store(error.code().0, Release);
                }
                state.running.store(false, Release);
            })
            .map_err(|error| error.to_string())?;
        match setup.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok((output, frames))) => Ok(Self {
                output,
                frames,
                input: Some(input),
                worker: Some(worker),
                quit,
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
            }
            Err(_) => {
                quit.store(true, Release);
                Err("Windows did not open the Exclusive audio endpoint in time".into())
            }
        }
    }
}

impl Backend for Exclusive {
    type Stream = ExclusiveStream;

    fn format(&self) -> &OutputFormat {
        &self.output
    }

    fn start(
        mut self,
        consumer: Consumer<u32>,
        _: Arc<RenderState>,
    ) -> Result<(Self::Stream, Option<u32>), String> {
        self.input
            .take()
            .ok_or("Exclusive input was already connected")?
            .send(consumer)
            .map_err(|_| "Exclusive output stopped during setup")?;
        let stream = ExclusiveStream {
            quit: Arc::clone(&self.quit),
            worker: self.worker.take(),
        };
        Ok((stream, Some(self.frames)))
    }
}

impl Drop for Exclusive {
    fn drop(&mut self) {
        self.input.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub struct ExclusiveStream {
    quit: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Drop for ExclusiveStream {
    fn drop(&mut self) {
        self.quit.store(true, Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
