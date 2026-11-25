mod convert;
mod decoder;
pub mod dsp;
mod exclusive;
mod gapless;
pub mod loudness;
mod pcm;
pub mod replaygain;
pub mod setup;
mod shared;
#[cfg(test)]
mod tests;

use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::*},
        mpsc::{self, Receiver},
    },
    thread,
    time::{Duration, Instant},
};

use eframe::egui;
use rtrb::{Consumer, RingBuffer};
use serde::{Deserialize, Serialize};

use self::{
    convert::Converter,
    decoder::{Source, SourceFormat},
    exclusive::Exclusive,
    shared::Shared,
};
use decoder::SampleKind;
pub use decoder::container_duration;
pub use gapless::{Next, NextTarget, Transition};

const NO_SEEK: u64 = u64::MAX;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub enum OutputMode {
    #[default]
    Shared,
    Exclusive,
}

/// Sinc resampler effort. `High` preserves the previous 256-tap/Cubic
/// behaviour so existing profiles sound unchanged.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub enum ResamplerQuality {
    Fast,
    Balanced,
    #[default]
    High,
    Maximum,
}

impl ResamplerQuality {
    pub fn label(self) -> &'static str {
        match self {
            Self::Fast => "Fast (64 taps)",
            Self::Balanced => "Balanced (128 taps)",
            Self::High => "High (256 taps)",
            Self::Maximum => "Maximum (512 taps)",
        }
    }

    pub(super) fn sinc_len(self) -> usize {
        match self {
            Self::Fast => 64,
            Self::Balanced => 128,
            Self::High => 256,
            Self::Maximum => 512,
        }
    }

    pub(super) fn interpolation(self) -> rubato::SincInterpolationType {
        match self {
            Self::Fast => rubato::SincInterpolationType::Linear,
            Self::Balanced => rubato::SincInterpolationType::Quadratic,
            Self::High | Self::Maximum => rubato::SincInterpolationType::Cubic,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default)]
pub struct OutputOptions {
    pub mode: OutputMode,
    pub convert_exclusive: bool,
    pub fallback_shared: bool,
    pub software_gain: bool,
    pub transition_fades: bool,
    pub replay_gain: replaygain::Settings,
    pub verify_flac: bool,
    pub reject_downmix: bool,
    pub resampler: ResamplerQuality,
    #[serde(skip)]
    pub effects_requested: bool,
}

pub struct OutputRequest {
    pub device: Option<String>,
    pub options: OutputOptions,
    pub after: Option<Arc<AtomicBool>>,
    pub dsp: dsp::Settings,
}

#[derive(Clone, Debug)]
pub struct OutputFormat {
    pub id: String,
    pub device: String,
    pub rate: u32,
    pub channels: u16,
    pub sample: String,
    pub buffer_frames: Option<u32>,
    pub dither: bool,
    pub mode: OutputMode,
    pub direct: Option<SampleKind>,
    pub software_gain: bool,
    pub transition_fades: bool,
    pub effects: dsp::Active,
}

trait Backend {
    type Stream;
    fn format(&self) -> &OutputFormat;
    fn start(
        self,
        consumer: Consumer<u32>,
        state: Arc<RenderState>,
    ) -> Result<(Self::Stream, Option<u32>), String>;
}

struct RenderState {
    next: Mutex<Option<Next>>,
    /// Bumped only when `next` actually changes. The decoder loop reads this
    /// atomic every iteration and touches the mutex only on a real update.
    next_epoch: AtomicU64,
    options: OutputOptions,
    dsp: dsp::Settings,
    closed: AtomicBool,
    paused: AtomicBool,
    ready: AtomicBool,
    drain: AtomicBool,
    eof: AtomicBool,
    drained: AtomicBool,
    ended: AtomicBool,
    output_lost: AtomicBool,
    priority_denied: AtomicBool,
    underruns: AtomicU64,
    frames: AtomicU64,
    listened_frames: AtomicU64,
    track_start: AtomicU64,
    origin_ns: AtomicU64,
    seek_ns: AtomicU64,
    volume: AtomicU32,
    rate: AtomicU32,
    running: AtomicBool,
    driver_error: std::sync::atomic::AtomicI32,
}

impl RenderState {
    fn new(position: Duration, volume: f32, paused: bool) -> Self {
        Self {
            next: Mutex::new(None),
            next_epoch: AtomicU64::new(0),
            options: OutputOptions::default(),
            dsp: dsp::Settings::default(),
            closed: AtomicBool::new(false),
            paused: AtomicBool::new(paused),
            ready: AtomicBool::new(false),
            drain: AtomicBool::new(false),
            eof: AtomicBool::new(false),
            drained: AtomicBool::new(false),
            ended: AtomicBool::new(false),
            output_lost: AtomicBool::new(false),
            priority_denied: AtomicBool::new(false),
            underruns: AtomicU64::new(0),
            frames: AtomicU64::new(0),
            listened_frames: AtomicU64::new(0),
            track_start: AtomicU64::new(0),
            origin_ns: AtomicU64::new(nanoseconds(position)),
            seek_ns: AtomicU64::new(NO_SEEK),
            volume: AtomicU32::new(volume.clamp(0.0, 1.0).to_bits()),
            rate: AtomicU32::new(0),
            running: AtomicBool::new(false),
            driver_error: std::sync::atomic::AtomicI32::new(0),
        }
    }

    /// Returns `Some(plan)` only when the queue changed since `seen`. The
    /// caller keeps `seen` locally, so the audio loop normally performs just
    /// one atomic load instead of locking and cloning `Next` every iteration.
    fn changed_next(&self, seen: &mut u64) -> Option<Option<Next>> {
        let epoch = self.next_epoch.load(Acquire);
        if epoch == *seen {
            return None;
        }
        *seen = epoch;
        Some(self.next.lock().ok().and_then(|plan| plan.clone()))
    }

    fn set_next(&self, next: Option<Next>) {
        if let Ok(mut plan) = self.next.lock()
            && *plan != next
        {
            *plan = next;
            drop(plan);
            // Publish after the mutex write; the decoder's Acquire load then
            // observes the new plan before it takes the lock.
            self.next_epoch.fetch_add(1, Release);
        }
    }
}

pub enum Event {
    Opened(OutputFormat),
    Transition(Transition),
    Warning(String),
    Error(String),
}

pub struct AudioTrack {
    pub source: SourceFormat,
    pub output: Option<OutputFormat>,
    pub duration: Option<Duration>,
    pub failed: bool,
    state: Arc<RenderState>,
    events: Receiver<Event>,
    released: Arc<AtomicBool>,
    transition: Option<Transition>,
}

impl AudioTrack {
    pub fn open(
        path: &Path,
        request: OutputRequest,
        position: Duration,
        volume: f32,
        paused: bool,
        ctx: &egui::Context,
    ) -> Result<Self, String> {
        // Probing follows the existing synchronous track selection. Packets are decoded by the worker.
        let source = Source::open_with_options(path, request.options)?;
        let duration = source.duration;
        let position = bounded_position(position, duration);
        let source_format = source.info.clone();
        let mut render_state = RenderState::new(position, volume, paused);
        render_state.options = request.options;
        render_state.dsp = request.dsp;
        let state = Arc::new(render_state);
        let worker_state = Arc::clone(&state);
        let (tx, events) = mpsc::channel();
        let ctx = ctx.clone();
        let released = Arc::new(AtomicBool::new(false));
        let worker_released = Arc::clone(&released);
        thread::Builder::new()
            .name("audio-decoder".into())
            .spawn(move || {
                let _release = ReleaseOnDrop(worker_released);
                let report = |event| {
                    let _ = tx.send(event);
                    ctx.request_repaint();
                };
                if !wait_for_gate(request.after.as_deref(), Duration::from_secs(5)) {
                    report(Event::Warning(
                        "Gapless handoff timed out; starting track independently".into(),
                    ));
                }
                if worker_state.closed.load(Acquire) {
                    return;
                }
                let result = play(
                    source,
                    request.device.as_deref(),
                    request.options,
                    position,
                    &worker_state,
                    &report,
                    || ctx.request_repaint(),
                );
                if let Err(error) = result {
                    worker_state.paused.store(true, Relaxed);
                    report(Event::Error(error));
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            source: source_format,
            output: None,
            state,
            events,
            released,
            transition: None,
            duration,
            failed: false,
        })
    }

    pub fn queue_next(&self, next: Option<Next>) {
        self.state.set_next(next);
    }

    pub fn take_transition(&mut self) -> Option<Transition> {
        self.transition.take()
    }

    pub fn set_volume(&self, volume: f32) {
        self.state
            .volume
            .store(volume.clamp(0.0, 1.0).to_bits(), Relaxed);
    }

    pub fn release_gate(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.released)
    }

    pub fn bit_perfect_now(&self) -> bool {
        self.output
            .as_ref()
            .is_some_and(|output| output.direct.is_some() && !output.transition_fades)
            && self.state.running.load(Acquire)
            && self.state.ready.load(Acquire)
            && !self.state.drained.load(Acquire)
            && !self.is_paused()
            && !self.failed
            && !self.output_lost()
            && self.underruns() == 0
    }

    pub fn play(&self) {
        self.state.paused.store(false, Relaxed);
    }

    pub fn pause(&self) {
        self.state.paused.store(true, Relaxed);
    }

    pub fn is_paused(&self) -> bool {
        self.state.paused.load(Relaxed)
    }

    pub fn empty(&self) -> bool {
        self.state.ended.load(Acquire)
    }

    pub fn get_pos(&self) -> Duration {
        let requested = self.state.seek_ns.load(Relaxed);
        if requested != NO_SEEK {
            return Duration::from_nanos(requested);
        }
        let origin = Duration::from_nanos(self.state.origin_ns.load(Relaxed));
        let rate = self.state.rate.load(Relaxed);
        if rate == 0 {
            return origin;
        }
        let frames = self
            .state
            .frames
            .load(Relaxed)
            .saturating_sub(self.state.track_start.load(Relaxed));
        let played = Duration::from_secs_f64(frames as f64 / f64::from(rate));
        let position = origin.saturating_add(played);
        self.duration
            .map_or(position, |duration| position.min(duration))
    }

    pub fn listened(&self) -> Duration {
        let frames = self.state.listened_frames.load(Relaxed);
        let rate = self.state.rate.load(Relaxed).max(1);
        Duration::from_secs_f64(frames as f64 / f64::from(rate))
    }

    pub fn try_seek(&self, position: Duration) -> Result<(), String> {
        if self.output_lost() || self.failed {
            return Err("Audio device is unavailable".into());
        }
        let position = bounded_position(position, self.duration);
        self.state.ended.store(false, Release);
        self.state.seek_ns.store(nanoseconds(position), Release);
        Ok(())
    }

    pub fn output_lost(&self) -> bool {
        self.state.output_lost.load(Relaxed)
    }

    pub fn underruns(&self) -> u64 {
        self.state.underruns.load(Relaxed)
    }

    pub fn priority_denied(&self) -> bool {
        self.state.priority_denied.load(Relaxed)
    }

    pub fn poll(&mut self) -> Option<String> {
        let mut error = None;
        for event in self.events.try_iter() {
            match event {
                Event::Opened(format) => self.output = Some(format),
                Event::Transition(transition) => {
                    self.source = transition.source.clone();
                    self.duration = transition.duration;
                    self.transition = Some(transition);
                }
                Event::Warning(message) => error = Some(message),
                Event::Error(message) => {
                    self.failed = true;
                    error = Some(message);
                }
            }
        }
        error
    }
}

struct ReleaseOnDrop(Arc<AtomicBool>);

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Release);
    }
}

impl Drop for AudioTrack {
    fn drop(&mut self) {
        self.state.paused.store(true, Relaxed);
        self.state.closed.store(true, Release);
    }
}

fn nanoseconds(value: Duration) -> u64 {
    value.as_nanos().min(u128::from(NO_SEEK - 1)) as u64
}

fn bounded_position(position: Duration, duration: Option<Duration>) -> Duration {
    duration.map_or(position, |duration| {
        position.min(duration.saturating_sub(Duration::from_millis(1)))
    })
}

fn play(
    source: Source,
    device: Option<&str>,
    options: OutputOptions,
    position: Duration,
    state: &Arc<RenderState>,
    report: &impl Fn(Event),
    repaint: impl Fn(),
) -> Result<(), String> {
    if options.mode == OutputMode::Exclusive {
        match Exclusive::prepare(device, &source.info, options, Arc::clone(state)) {
            Ok(backend) => return pump(backend, source, position, state, report, repaint),
            Err(error) if options.fallback_shared => {
                report(Event::Warning(format!("{error}. Using WASAPI Shared.")))
            }
            Err(error) => return Err(error),
        }
    }
    let backend = Shared::open(device).inspect_err(|_| state.output_lost.store(true, Relaxed))?;
    pump(backend, source, position, state, report, repaint)
}

fn pump<B: Backend>(
    backend: B,
    mut source: Source,
    position: Duration,
    state: &Arc<RenderState>,
    report: &impl Fn(Event),
    repaint: impl Fn(),
) -> Result<(), String> {
    let mut output = backend.format().clone();
    if state.options.reject_downmix && source.info.channels > output.channels {
        return Err(format!(
            "This output has {} channels. Enable downmix to play {} channels.",
            output.channels, source.info.channels
        ));
    }
    let channels = usize::from(output.channels);
    let capacity_frames = (output.rate as usize).min(2_000_000 / channels);
    let (mut producer, consumer) = RingBuffer::new(capacity_frames * channels);
    let resampler = state.options.resampler;
    let new_converter = |source: &SourceFormat| {
        output
            .direct
            .is_none()
            .then(|| Converter::new(source, output.rate, output.channels, resampler))
            .transpose()
    };
    let mut converter = new_converter(&source.info)?;
    let make_effects = || dsp::Processor::new(&state.dsp, output.rate, channels);
    let (mut effects, warning) = make_effects();
    output.effects = effects.active;
    if let Some(warning) = warning {
        report(Event::Warning(format!("FIR bypassed: {warning}")));
    }
    if !position.is_zero()
        && let Err(error) = source.seek(position)
    {
        report(Event::Warning(format!(
            "Could not restore the audio position: {error}"
        )));
        state.origin_ns.store(0, Relaxed);
    }
    state.rate.store(output.rate, Relaxed);
    let (_stream, buffer_frames) =
        backend
            .start(consumer, Arc::clone(state))
            .inspect_err(|_| {
                state.output_lost.store(true, Relaxed);
            })?;
    output.buffer_frames = buffer_frames;
    let tail_time = buffer_frames.map_or(Duration::from_millis(30), |frames| {
        Duration::from_secs_f64(f64::from(frames) / f64::from(output.rate))
    });
    let warm_samples = (output.rate as usize / 10 * channels).min(capacity_frames * channels);
    report(Event::Opened(output.clone()));
    let mut pending = Vec::new();
    let mut converted = Vec::new();
    let mut float_samples = None;
    let mut integer_samples = None;
    let mut cursor = 0;
    let mut finished_decode = false;
    let mut flushed_resampler = false;
    let mut tail_since: Option<Instant> = None;
    let mut planned = None;
    let mut prepared = None;
    let mut handoff: Option<gapless::Handoff> = None;
    let mut submitted = 0_u64;
    let mut listened_at_seek = 0_u64;
    // Sentinel forces the first iteration to observe a plan queued before the
    // pump started. After that only epoch changes cost a mutex + clone.
    let mut seen_epoch = u64::MAX;
    let mut desired = None;
    while !state.closed.load(Acquire) {
        let driver_error = state.driver_error.load(Acquire);
        if driver_error != 0 {
            return Err(exclusive::driver_message(windows::core::HRESULT(
                driver_error,
            )));
        }
        if state.output_lost.load(Relaxed) {
            return Err("Windows interrupted the audio output".into());
        }
        if let Some(updated) = state.changed_next(&mut seen_epoch) {
            desired = updated;
        }
        let mut seek = state.seek_ns.swap(NO_SEEK, AcqRel);
        let rendered = state.frames.load(Acquire);
        if let Some(pending) = &handoff {
            if rendered > pending.boundary {
                let event = Transition {
                    next: pending.next.clone(),
                    source: source.info.clone(),
                    duration: source.duration,
                    listened_at_boundary: Duration::from_secs_f64(
                        (listened_at_seek + pending.boundary) as f64 / f64::from(output.rate),
                    ),
                };
                state.track_start.store(pending.boundary, Relaxed);
                state.origin_ns.store(0, Relaxed);
                // Only clear this request. A newer playlist edit might already be present.
                if let Ok(mut plan) = state.next.lock()
                    && plan.as_ref() == Some(&pending.next)
                {
                    *plan = None;
                    drop(plan);
                    state.next_epoch.fetch_add(1, Release);
                }
                // Re-observe on the next iteration: a newer plan may already
                // be queued, and `seen_epoch` still names the cleared one.
                if desired.as_ref() == Some(&pending.next) {
                    desired = None;
                }
                handoff = None;
                planned = None;
                report(Event::Transition(event));
            } else if (seek != NO_SEEK || desired.as_ref() != Some(&pending.next))
                && let Some(pending) = handoff.take()
            {
                source = pending.previous;
                if seek == NO_SEEK {
                    let elapsed = rendered.saturating_sub(state.track_start.load(Relaxed));
                    let position = Duration::from_nanos(state.origin_ns.load(Relaxed))
                        + Duration::from_secs_f64(elapsed as f64 / f64::from(output.rate));
                    seek = nanoseconds(bounded_position(position, source.duration));
                }
                planned = None;
            }
        }
        if seek != NO_SEEK {
            // Retire all queued samples before the producer resumes writing.
            state.ready.store(false, Release);
            state.drain.store(true, Release);
            let deadline = Instant::now() + Duration::from_secs(1);
            while state.drain.load(Acquire) && !state.closed.load(Acquire) {
                if state.output_lost.load(Relaxed) {
                    return Err("Audio device was lost during seek".into());
                }
                if Instant::now() >= deadline {
                    return Err("Windows audio output did not respond to the seek".into());
                }
                thread::sleep(Duration::from_millis(2));
            }
            if state.closed.load(Acquire) {
                break;
            }
            source
                .seek(Duration::from_nanos(seek))
                .map_err(|error| format!("Could not seek: {error}"))?;
            converter = new_converter(&source.info)?;
            effects = make_effects().0;
            pending.clear();
            prepared = None;
            planned = None;
            cursor = 0;
            finished_decode = false;
            flushed_resampler = false;
            tail_since = None;
            state.drained.store(false, Release);
            state.eof.store(false, Release);
            state.ended.store(false, Release);
            listened_at_seek = state.listened_frames.load(Relaxed);
            state.frames.store(0, Relaxed);
            state.track_start.store(0, Relaxed);
            submitted = 0;
            state.origin_ns.store(seek, Relaxed);
            repaint();
        }
        if handoff.is_none() && desired != planned {
            prepared = None;
            // Keep the current stream supplied before touching the next file.
            if finished_decode || capacity_frames * channels - producer.slots() >= warm_samples {
                planned = desired.clone();
                if let Some(candidate) = desired
                    .as_ref()
                    .filter(|_| !state.drained.load(Acquire))
                    .cloned()
                {
                    match Source::open_with_options(&candidate.path, state.options) {
                        Ok(next_source) if gapless::compatible(&source.info, &next_source.info) => {
                            prepared = Some((candidate, next_source));
                        }
                        Ok(_) => {}
                        Err(error) => report(Event::Warning(format!(
                            "Could not prepare next track: {error}"
                        ))),
                    }
                }
            }
        }
        let slots = producer.slots() / channels * channels;
        if cursor == pending.len() && slots != 0 && !finished_decode {
            pending.clear();
            cursor = 0;
            if let Some(SampleKind::Integer(bits)) = output.direct {
                if let Some(samples) =
                    gapless::next_packet::<i32>(&mut source, &mut integer_samples, report)
                {
                    let padding = u32::MAX.checked_shr(u32::from(bits)).unwrap_or(0);
                    if samples.iter().any(|&sample| (sample as u32) & padding != 0) {
                        return Err(
                            "The decoder returned values outside the declared PCM precision".into(),
                        );
                    }
                    pending.extend(samples.iter().map(|&sample| sample as u32));
                } else {
                    finished_decode = true;
                }
            } else if let Some(samples) =
                gapless::next_packet::<f32>(&mut source, &mut float_samples, report)
            {
                if let Some(converter) = &mut converter {
                    converted.clear();
                    converter.set_gain(source.info.gain.unwrap_or(1.0));
                    converter.push(samples, &mut converted)?;
                    effects.process(&mut converted);
                    pending.extend(converted.iter().map(|sample| sample.to_bits()));
                } else {
                    pending.extend(samples.iter().map(|sample| sample.to_bits()));
                }
            } else {
                if let Some(converter) = &mut converter
                    && (prepared.is_none() || handoff.is_some())
                {
                    converted.clear();
                    converter.finish(&mut converted)?;
                    effects.process(&mut converted);
                    pending.extend(converted.iter().map(|sample| sample.to_bits()));
                    flushed_resampler = source.info.rate != output.rate;
                }
                finished_decode = true;
            }
        }
        let count = slots.min(pending.len() - cursor);
        if count != 0
            && let Ok(chunk) = producer.write_chunk_uninit(count)
        {
            chunk.fill_from_iter(pending[cursor..cursor + count].iter().copied());
            cursor += count;
            submitted += (count / channels) as u64;
        }
        if finished_decode && cursor == pending.len() {
            if handoff.is_none()
                && !state.drained.load(Acquire)
                && !flushed_resampler
                && let Some((next, ready)) = prepared.take()
            {
                // A continuous sinc filter still holds some of the preceding track.
                let boundary = if source.info.rate != output.rate {
                    converter
                        .as_ref()
                        .map_or(submitted, Converter::target_frames)
                } else {
                    submitted
                };
                let previous = std::mem::replace(&mut source, ready);
                handoff = Some(gapless::Handoff {
                    next,
                    previous,
                    boundary,
                });
                finished_decode = false;
                state.eof.store(false, Release);
                // Keep filling the same ring without advertising end-of-stream.
                continue;
            }
            state.eof.store(true, Release);
        }
        if state.eof.load(Relaxed) || capacity_frames * channels - producer.slots() >= warm_samples
        {
            state.ready.store(true, Release);
        }
        if state.drained.load(Acquire) {
            let since = tail_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= tail_time && !state.ended.swap(true, AcqRel) {
                repaint();
            }
        }
        if slots == 0 || (finished_decode && cursor == pending.len()) {
            thread::sleep(Duration::from_millis(5));
        }
    }
    Ok(())
}

pub struct DeviceMonitor {
    pub devices: shared::Devices,
    updates: Receiver<shared::Devices>,
    closed: Arc<AtomicBool>,
}

impl DeviceMonitor {
    pub fn new(ctx: &egui::Context) -> Self {
        let (sender, updates) = mpsc::sync_channel(1);
        let closed = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&closed);
        let ctx = ctx.clone();
        let _ = thread::Builder::new()
            .name("audio-devices".into())
            .spawn(move || {
                let mut sent = None;
                while !stop.load(Acquire) {
                    if let Ok(devices) = shared::devices()
                        && sent.as_ref() != Some(&devices)
                        && sender.try_send(devices.clone()).is_ok()
                    {
                        sent = Some(devices);
                        ctx.request_repaint();
                    }
                    thread::sleep(Duration::from_millis(1500));
                }
            });
        Self {
            devices: shared::Devices::default(),
            updates,
            closed,
        }
    }

    pub fn refresh(&mut self) -> bool {
        let mut changed = false;
        for devices in self.updates.try_iter() {
            self.devices = devices;
            changed = true;
        }
        changed
    }
}

impl Drop for DeviceMonitor {
    fn drop(&mut self) {
        self.closed.store(true, Release);
    }
}

/// Wait for a gapless handoff, but never let a stuck gate hold a decoder worker
/// forever. Returns `false` only after the timeout; `None` means no handoff.
fn wait_for_gate(gate: Option<&AtomicBool>, timeout: Duration) -> bool {
    let Some(gate) = gate else {
        return true;
    };
    let started = Instant::now();
    while !gate.load(Acquire) {
        if started.elapsed() >= timeout {
            return false;
        }
        thread::sleep(Duration::from_millis(5));
    }
    true
}

#[cfg(test)]
mod gate_tests {
    use super::*;

    #[test]
    fn gate_timeout_does_not_hold_the_worker_forever() {
        let stalled = AtomicBool::new(false);
        let started = Instant::now();
        assert!(!wait_for_gate(Some(&stalled), Duration::from_millis(25)));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(wait_for_gate(None, Duration::from_millis(25)));
        let open = AtomicBool::new(true);
        assert!(wait_for_gate(Some(&open), Duration::from_millis(25)));
    }
}

#[cfg(test)]
mod queue_tests {
    use super::*;

    fn next(path: &'static str, generation: u64) -> Next {
        Next {
            path: path.into(),
            target: NextTarget::Playlist(generation as usize),
            generation,
        }
    }

    #[test]
    fn queue_changes_are_observed_once_without_polling_the_mutex() {
        let state = RenderState::new(Duration::ZERO, 1.0, false);
        let mut seen = u64::MAX;
        assert_eq!(state.changed_next(&mut seen), Some(None));
        assert_eq!(state.changed_next(&mut seen), None);

        let first = next("a.wav", 1);
        state.set_next(Some(first.clone()));
        assert_eq!(state.changed_next(&mut seen), Some(Some(first.clone())));
        assert_eq!(state.changed_next(&mut seen), None);

        // An identical re-queue must not dirty the decoder's cache.
        state.set_next(Some(first));
        assert_eq!(state.changed_next(&mut seen), None);

        let second = next("b.wav", 2);
        state.set_next(Some(second.clone()));
        assert_eq!(state.changed_next(&mut seen), Some(Some(second)));
        assert_eq!(state.changed_next(&mut seen), None);

        state.set_next(None);
        assert_eq!(state.changed_next(&mut seen), Some(None));
        assert_eq!(state.changed_next(&mut seen), None);
    }
}
