use super::{setup::FileBackend, *};
use std::{fs, io::Write, path::PathBuf};

#[test]
fn cue_segments_decode_and_seek_within_their_own_track() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let mut wav = fs::File::create(directory.path().join("disc.wav"))?;
    let rate = 48_000_u32;
    let samples: Vec<i16> = (0..1_280_i16).map(|value| value - 640).collect();
    let bytes = (samples.len() * 2) as u32;
    wav.write_all(b"RIFF")?;
    wav.write_all(&(bytes + 36).to_le_bytes())?;
    wav.write_all(b"WAVEfmt \x10\x00\x00\x00\x01\x00\x01\x00")?;
    wav.write_all(&rate.to_le_bytes())?;
    wav.write_all(&(rate * 2).to_le_bytes())?;
    wav.write_all(b"\x02\x00\x10\x00data")?;
    wav.write_all(&bytes.to_le_bytes())?;
    for sample in &samples {
        wav.write_all(&sample.to_le_bytes())?;
    }
    drop(wav);
    let sheet = directory.path().join("disc.cue");
    fs::write(
        &sheet,
        "FILE \"disc.wav\" WAVE\nTRACK 01 AUDIO\nINDEX 01 00:00:00\nTRACK 02 AUDIO\nINDEX 01 00:00:01\n",
    )?;
    let parts = crate::cue::entries(&sheet)?;
    for (index, path) in parts.iter().enumerate() {
        let mut source = Source::open(path)?;
        let advertised = crate::library::TrackInfo::read(path).duration.unwrap();
        assert!(advertised.abs_diff(crate::cue::duration(1)) <= Duration::from_nanos(1));
        let mut buffer = None;
        let mut read = Vec::new();
        while let Some(packet) = source.next::<i16>(&mut buffer)? {
            read.extend_from_slice(packet);
        }
        assert!(
            read == samples[index * 640..(index + 1) * 640],
            "CUE segment {index} differs or has the wrong length"
        );
        source.seek(Duration::from_millis(5))?;
        let next = source
            .next::<i16>(&mut buffer)?
            .ok_or("No PCM after seek")?;
        assert_eq!(next[0], samples[index * 640 + 240]);
    }
    Ok(())
}

#[test]
fn seek_discards_old_samples_before_rendering_the_new_position() -> Result<(), String> {
    let state = Arc::new(RenderState::new(Duration::ZERO, 1.0, false));
    let (mut producer, consumer) = RingBuffer::new(8);
    for value in [0.25, 0.25, 0.25, 0.25] {
        producer
            .push(f32::to_bits(value))
            .map_err(|error| error.to_string())?;
    }
    let mut render = shared::Renderer {
        consumer,
        state: Arc::clone(&state),
        channels: 2,
        dither: None,
        rng: 1,
    };
    state.drain.store(true, Release);
    let mut output = [1.0_f32; 2];
    render.fill(&mut output);
    assert_eq!(output, [0.0; 2]);
    assert_eq!(producer.slots(), 8);
    assert!(!state.drain.load(Acquire));
    producer
        .push(0.125_f32.to_bits())
        .map_err(|error| error.to_string())?;
    producer
        .push((-0.125_f32).to_bits())
        .map_err(|error| error.to_string())?;
    state.ready.store(true, Release);
    render.fill(&mut output);
    assert_eq!(output, [0.125, -0.125]);
    assert_eq!(state.frames.load(Relaxed), 1);
    Ok(())
}

struct Null {
    format: OutputFormat,
}

impl Backend for Null {
    type Stream = thread::JoinHandle<()>;

    fn format(&self) -> &OutputFormat {
        &self.format
    }

    fn start(
        self,
        consumer: Consumer<u32>,
        state: Arc<RenderState>,
    ) -> Result<(Self::Stream, Option<u32>), String> {
        let channels = usize::from(self.format.channels);
        let handle = thread::spawn(move || {
            let mut renderer = shared::Renderer {
                consumer,
                state: Arc::clone(&state),
                channels,
                dither: None,
                rng: 1,
            };
            let mut packet = vec![0_f32; 128 * channels];
            while !state.closed.load(Acquire) {
                renderer.fill(&mut packet);
                thread::sleep(Duration::from_millis(1));
            }
        });
        Ok((handle, Some(128)))
    }
}

#[test]
fn decoder_ring_and_null_output_play_to_end() -> Result<(), Box<dyn std::error::Error>> {
    let mut wav = tempfile::Builder::new().suffix(".wav").tempfile()?;
    let rate = 48_000u32;
    let frames = rate / 4;
    let size = frames * 2;
    wav.write_all(b"RIFF")?;
    wav.write_all(&(size + 36).to_le_bytes())?;
    wav.write_all(b"WAVEfmt \x10\x00\x00\x00\x01\x00\x01\x00")?;
    wav.write_all(&rate.to_le_bytes())?;
    wav.write_all(&(rate * 2).to_le_bytes())?;
    wav.write_all(b"\x02\x00\x10\x00data")?;
    wav.write_all(&size.to_le_bytes())?;
    wav.write_all(&vec![0; size as usize])?;
    wav.as_file().sync_all()?;

    let source = Source::open(wav.path())?;
    assert_eq!(source.info.bits, Some(16));
    let state = Arc::new(RenderState::new(Duration::ZERO, 0.25, false));
    let timeout = Arc::clone(&state);
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(3));
        timeout.closed.store(true, Release);
    });
    let backend = Null {
        format: OutputFormat {
            id: "null".into(),
            device: "Null".into(),
            rate: 44_100,
            channels: 2,
            sample: "f32".into(),
            buffer_frames: None,
            dither: false,
            mode: OutputMode::Shared,
            direct: None,
            software_gain: true,
            transition_fades: false,
            effects: Default::default(),
        },
    };
    pump(backend, source, Duration::ZERO, &state, &|_| {}, || {
        state.closed.store(true, Release);
    })?;
    assert!(state.ended.load(Acquire));
    assert_eq!(state.frames.load(Relaxed), 11_025);
    assert_eq!(state.underruns.load(Relaxed), 0);
    Ok(())
}

#[test]
fn compatible_tracks_write_the_continuous_reference_in_one_stream()
-> Result<(), Box<dyn std::error::Error>> {
    check_continuous_output(48_000)?;
    check_continuous_output(44_100)
}

fn check_continuous_output(output_rate: u32) -> Result<(), Box<dyn std::error::Error>> {
    use pcm::{Encoding, PCM16};
    let temp = tempfile::tempdir()?;
    let rate = 48_000_u32;
    let reference: Vec<u8> = (0..24_000)
        .flat_map(|frame| {
            let sample =
                (f64::sin(f64::from(frame) * 997.0 * std::f64::consts::TAU / f64::from(rate))
                    * 14_000.0)
                    .round() as i16;
            sample.to_le_bytes()
        })
        .collect();
    let split = 12_137 * 2;
    for (name, data) in [
        ("first.wav", &reference[..split]),
        ("second.wav", &reference[split..]),
    ] {
        let mut file = fs::File::create(temp.path().join(name))?;
        let length = data.len() as u32;
        file.write_all(b"RIFF")?;
        file.write_all(&(length + 36).to_le_bytes())?;
        file.write_all(b"WAVEfmt \x10\x00\x00\x00\x01\x00\x01\x00")?;
        file.write_all(&rate.to_le_bytes())?;
        file.write_all(&(rate * 2).to_le_bytes())?;
        file.write_all(b"\x02\x00\x10\x00data")?;
        file.write_all(&length.to_le_bytes())?;
        file.write_all(data)?;
    }
    let source = Source::open(&temp.path().join("first.wav"))?;
    let (encoding, direct, reference) = if output_rate == rate {
        (PCM16, Some(source.info.kind), reference)
    } else {
        let samples: Vec<_> = reference
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&bytes| f32::from(i16::from_le_bytes(bytes)) / 32768.0)
            .collect();
        let mut reference_converter =
            Converter::new(&source.info, output_rate, 1, super::ResamplerQuality::High)?;
        let mut converted = Vec::new();
        reference_converter.push(&samples, &mut converted)?;
        reference_converter.finish(&mut converted)?;
        (
            Encoding::Float32,
            None,
            converted
                .iter()
                .flat_map(|sample| sample.to_le_bytes())
                .collect(),
        )
    };
    let next = Next {
        path: temp.path().join("second.wav"),
        target: super::NextTarget::Playlist(1),
        generation: 0,
    };
    let state = Arc::new(RenderState::new(Duration::ZERO, 1.0, false));
    *state.next.lock().map_err(|_| "Could not set test queue")? = Some(next.clone());
    state.next_epoch.store(1, Release);
    let path = temp.path().join("joined.pcm");
    let backend = FileBackend {
        encoding,
        path: path.clone(),
        format: OutputFormat {
            id: "file".into(),
            device: "File".into(),
            rate: output_rate,
            channels: 1,
            sample: encoding.label().into(),
            buffer_frames: None,
            dither: false,
            mode: OutputMode::Exclusive,
            direct,
            software_gain: false,
            transition_fades: false,
            effects: Default::default(),
        },
    };
    let transitions = std::cell::Cell::new(0);
    let streams = std::cell::Cell::new(0);
    pump(
        backend,
        source,
        Duration::ZERO,
        &state,
        &|event| match event {
            Event::Transition(event) => {
                assert_eq!(event.next, next);
                transitions.set(transitions.get() + 1);
            }
            Event::Opened(_) => streams.set(streams.get() + 1),
            _ => {}
        },
        || state.closed.store(true, Release),
    )?;
    assert_eq!(streams.get(), 1);
    assert_eq!(transitions.get(), 1);
    let boundary = (12_137.0 * f64::from(output_rate) / f64::from(rate)).round() as u64;
    assert_eq!(state.track_start.load(Relaxed), boundary);
    assert!(
        fs::read(path)? == reference,
        "PCM differs at output rate {output_rate}"
    );
    Ok(())
}

#[test]
fn file_backend_preserves_pcm_bytes_at_multiple_depths_and_rates()
-> Result<(), Box<dyn std::error::Error>> {
    use pcm::{Encoding, PCM16, PCM24, PCM24_IN_32, PCM32};
    let temporary = tempfile::tempdir()?;
    let directory = std::env::var_os("RUST_PLAYER_PCM_CHECK_DIR")
        .map_or_else(|| temporary.path().to_owned(), PathBuf::from);
    fs::create_dir_all(&directory)?;
    let exact_values: [i32; 8] = [
        i32::MIN,
        i32::MAX,
        -1,
        0,
        1,
        0x1234_5678,
        -0x1234_5678,
        0x0000_0081,
    ];
    for rate in [44_100_u32, 48_000, 96_000, 192_000] {
        for (bits, encoding) in [
            (16_u16, PCM16),
            (24, PCM24),
            (24, PCM24_IN_32),
            (32, PCM32),
            (32, Encoding::Float32),
        ] {
            let is_float = encoding == Encoding::Float32;
            let width = usize::from(bits / 8);
            let samples: Vec<_> = if is_float {
                [
                    1.5_f32,
                    -1.0,
                    -0.0,
                    0.0,
                    0.125,
                    0.25,
                    -0.875,
                    f32::MIN_POSITIVE,
                ]
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect()
            } else {
                exact_values
                    .iter()
                    .flat_map(|value| (value >> (32 - bits)).to_le_bytes().into_iter().take(width))
                    .collect()
            };
            let expected = if encoding == PCM24_IN_32 {
                samples
                    .chunks_exact(width)
                    .flat_map(|chunk| [0, chunk[0], chunk[1], chunk[2]])
                    .collect()
            } else {
                samples.clone()
            };
            let stem = format!(
                "{rate}-{bits}-{}",
                if is_float {
                    "float"
                } else if encoding == PCM24_IN_32 {
                    "padded"
                } else {
                    "packed"
                }
            );
            let input = directory.join(format!("{stem}.wav"));
            let output = directory.join(format!("{stem}.pcm"));
            let mut wav = fs::File::create(&input)?;
            let bytes = samples.len() as u32;
            let block = 2 * width as u16;
            let tag = if is_float { 3_u16 } else { 1_u16 };
            wav.write_all(b"RIFF")?;
            wav.write_all(&(bytes + 36).to_le_bytes())?;
            wav.write_all(b"WAVEfmt \x10\x00\x00\x00")?;
            wav.write_all(&tag.to_le_bytes())?;
            wav.write_all(&2_u16.to_le_bytes())?;
            wav.write_all(&rate.to_le_bytes())?;
            wav.write_all(&(rate * u32::from(block)).to_le_bytes())?;
            wav.write_all(&block.to_le_bytes())?;
            wav.write_all(&bits.to_le_bytes())?;
            wav.write_all(b"data")?;
            wav.write_all(&bytes.to_le_bytes())?;
            wav.write_all(&samples)?;
            drop(wav);

            let source = Source::open(&input)?;
            let kind = if is_float {
                SampleKind::Float32
            } else {
                SampleKind::Integer(bits)
            };
            assert_eq!(source.info.kind, kind, "{stem}");
            let backend = FileBackend {
                format: OutputFormat {
                    id: "file".into(),
                    device: "File".into(),
                    rate,
                    channels: 2,
                    sample: encoding.label().into(),
                    buffer_frames: None,
                    dither: false,
                    mode: OutputMode::Exclusive,
                    direct: Some(kind),
                    software_gain: false,
                    transition_fades: false,
                    effects: Default::default(),
                },
                encoding,
                path: output.clone(),
            };
            let state = Arc::new(RenderState::new(Duration::ZERO, 0.0, false));
            let deadline = Instant::now() + Duration::from_secs(2);
            pump(backend, source, Duration::ZERO, &state, &|_| {}, || {
                state.closed.store(true, Release);
            })?;
            assert!(Instant::now() < deadline, "{stem}");
            assert_eq!(state.frames.load(Relaxed), 4, "{stem}");
            assert_eq!(fs::read(output)?, expected, "{stem}");
        }
    }
    Ok(())
}

#[test]
fn extra_codecs_are_compiled_in() {
    use symphonia::core::codecs::{
        CODEC_TYPE_ADPCM_IMA_WAV, CODEC_TYPE_ADPCM_MS, CODEC_TYPE_MP1, CODEC_TYPE_MP2,
        CodecParameters,
    };

    let registry = symphonia::default::get_codecs();
    for codec in [CODEC_TYPE_MP1, CODEC_TYPE_MP2] {
        let mut params = CodecParameters::new();
        params.codec = codec;
        params.sample_rate = Some(48_000);
        assert!(
            registry.make(&params, &Default::default()).is_ok(),
            "decoder for {codec:?} must be compiled in"
        );
    }
    for codec in [CODEC_TYPE_ADPCM_MS, CODEC_TYPE_ADPCM_IMA_WAV] {
        let mut params = CodecParameters::new();
        params.codec = codec;
        params.sample_rate = Some(48_000);
        params.channels = Some(
            symphonia::core::audio::Channels::FRONT_LEFT
                | symphonia::core::audio::Channels::FRONT_RIGHT,
        );
        params.frames_per_block = Some(256);
        params.max_frames_per_packet = Some(256);
        assert!(
            registry.make(&params, &Default::default()).is_ok(),
            "decoder for {codec:?} must be compiled in"
        );
    }
}
