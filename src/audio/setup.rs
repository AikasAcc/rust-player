use super::{shared::Shared, *};
use std::{fs, io::Write, path::PathBuf};
use symphonia::core::{checksum::Md5, io::Monitor};

pub struct DeviceCheck {
    pub shared_rate: u32,
    pub shared_format: String,
    pub exclusive: Vec<(u32, Vec<u16>)>,
    pub probe_warning: Option<String>,
}

impl DeviceCheck {
    pub fn run(device: Option<&str>) -> Result<Self, String> {
        let shared = Shared::open(device)?;
        let format = shared.format();
        let (exclusive, probe_warning) = match exclusive::exclusive_formats(device, format.rate) {
            Ok(formats) => (formats, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        Ok(Self {
            shared_rate: format.rate,
            shared_format: format!(
                "{} Hz · {} ch · {}",
                format.rate, format.channels, format.sample
            ),
            exclusive,
            probe_warning,
        })
    }

    pub fn recommended(&self) -> OutputMode {
        if self
            .exclusive
            .iter()
            .any(|(rate, depths)| *rate == self.shared_rate && depths.contains(&16))
        {
            OutputMode::Exclusive
        } else {
            OutputMode::Shared
        }
    }

    pub fn tone_rate(&self, mode: OutputMode) -> Option<u32> {
        match mode {
            OutputMode::Shared => Some(self.shared_rate),
            OutputMode::Exclusive => self
                .exclusive
                .iter()
                .filter(|(_, depths)| depths.contains(&16))
                .min_by_key(|(rate, _)| rate.abs_diff(self.shared_rate))
                .map(|(rate, _)| *rate),
        }
    }
}

pub fn tone(path: &Path, rate: u32) -> std::io::Result<Vec<u8>> {
    let mut wave = fs::File::create(path)?;
    let count = rate / 2;
    let mut pcm = Vec::with_capacity(count as usize * 4);
    for frame in 0..count {
        let seconds = f64::from(frame) / f64::from(rate);
        let envelope = (seconds * 40.0).min((0.5 - seconds) * 40.0).clamp(0.0, 1.0);
        let sample = (f64::from(i16::MAX)
            * 0.015
            * envelope
            * (std::f64::consts::TAU * 440.0 * seconds).sin()) as i16;
        pcm.extend(sample.to_le_bytes());
        pcm.extend(sample.to_le_bytes());
    }
    let bytes = pcm.len() as u32;
    wave.write_all(b"RIFF")?;
    wave.write_all(&(bytes + 36).to_le_bytes())?;
    wave.write_all(b"WAVEfmt \x10\x00\x00\x00\x01\x00\x02\x00")?;
    wave.write_all(&rate.to_le_bytes())?;
    wave.write_all(&(rate * 4).to_le_bytes())?;
    wave.write_all(b"\x04\x00\x10\x00data")?;
    wave.write_all(&bytes.to_le_bytes())?;
    wave.write_all(&pcm)?;
    Ok(pcm)
}

pub fn check_pcm(rate: u32) -> Result<String, String> {
    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let input = directory.path().join("test.wav");
    let path = directory.path().join("capture.pcm");
    let expected = tone(&input, rate).map_err(|error| error.to_string())?;
    let source = Source::open_with_options(&input, OutputOptions::default())?;
    let state = Arc::new(RenderState::new(Duration::ZERO, 1.0, false));
    pump(
        FileBackend {
            format: OutputFormat {
                id: "file".into(),
                device: "File".into(),
                rate,
                channels: 2,
                sample: pcm::PCM16.label().into(),
                buffer_frames: None,
                mode: OutputMode::Exclusive,
                direct: Some(SampleKind::Integer(16)),
                dither: false,
                software_gain: false,
                transition_fades: false,
                effects: Default::default(),
            },
            encoding: pcm::PCM16,
            path: path.clone(),
        },
        source,
        Duration::ZERO,
        &state,
        &|_| {},
        || state.closed.store(true, Release),
    )?;
    let captured = fs::read(path).map_err(|error| error.to_string())?;
    let hash = |bytes: &[u8]| {
        let mut digest = Md5::default();
        digest.process_buf_bytes(bytes);
        digest.md5()
    };
    let checksum = hash(&captured);
    if expected.len() != captured.len() || hash(&expected) != checksum {
        return Err("The FileBackend output differs from the generated PCM.".into());
    }
    Ok(checksum.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(test)]
#[test]
fn generated_test_tone_matches_the_file_backend() {
    for rate in [44_100, 96_000] {
        assert_eq!(check_pcm(rate).unwrap().len(), 32);
    }
}

pub(super) struct FileBackend {
    pub(super) format: OutputFormat,
    pub(super) encoding: pcm::Encoding,
    pub(super) path: PathBuf,
}
pub(super) struct FileThread {
    stop: Arc<AtomicBool>,
    task: Option<thread::JoinHandle<()>>,
}

impl Drop for FileThread {
    fn drop(&mut self) {
        self.stop.store(true, Release);
        if let Some(task) = self.task.take() {
            let _ = task.join();
        }
    }
}

impl Backend for FileBackend {
    type Stream = FileThread;

    fn format(&self) -> &OutputFormat {
        &self.format
    }

    fn start(
        self,
        mut consumer: Consumer<u32>,
        state: Arc<RenderState>,
    ) -> Result<(Self::Stream, Option<u32>), String> {
        let mut file = fs::File::create(&self.path).map_err(|error| error.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        let quit = Arc::clone(&stop);
        let direct = self.format.direct.unwrap_or(SampleKind::Float32);
        let channels = usize::from(self.format.channels);
        let task = thread::spawn(move || {
            let timeout = Instant::now() + Duration::from_secs(2);
            while !quit.load(Acquire) && !state.closed.load(Acquire) {
                if Instant::now() >= timeout {
                    state.closed.store(true, Release);
                    return;
                }
                if state.ready.load(Acquire) {
                    let eof = state.eof.load(Acquire);
                    let count = consumer.slots() / channels * channels;
                    if let Ok(chunk) = consumer.read_chunk(count) {
                        let (head, tail) = chunk.as_slices();
                        if exclusive::write_exact(self.encoding, direct, head, &mut file)
                            .and_then(|()| {
                                exclusive::write_exact(self.encoding, direct, tail, &mut file)
                            })
                            .is_err()
                        {
                            state.closed.store(true, Release);
                            return;
                        }
                        state.frames.fetch_add((count / channels) as u64, Relaxed);
                        chunk.commit_all();
                    }
                    if eof && consumer.is_empty() {
                        state.drained.store(true, Release);
                    }
                }
                thread::sleep(Duration::from_millis(1));
            }
        });
        Ok((
            FileThread {
                stop,
                task: Some(task),
            },
            Some(1),
        ))
    }
}
