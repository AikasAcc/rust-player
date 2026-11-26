# Hardware verification: loopback and listening checks

Automated tests cover decoding, seeking, gapless continuation and the PCM bytes written by the
software renderer. They cannot prove what a Windows driver, DAC or analogue output does. This
guide separates the three questions so that a result is never reported as stronger than it is:

1. **Software integrity:** did the decoder and renderer produce the expected bytes?
2. **Digital capture:** are those bytes present at a capture point?
3. **Listening:** does the audible result behave correctly (joins, clicks, metadata, level)?

Record the Windows build, audio driver version, device, source format, negotiated output format and
underrun counter for every run.

## What each check proves

| Check | Proves | Does not prove |
| --- | --- | --- |
| `file_backend_preserves_pcm_bytes_at_multiple_depths_and_rates` | Decoder → renderer → sample packing is byte-exact | Driver, DAC, audible output |
| **Signal path: “Bit-perfect PCM at the Windows driver input”** | Exclusive stream, direct format, no software stages and no underruns at that moment | That the hardware received unchanged samples |
| WASAPI **Shared** loopback capture | Samples at the Windows engine’s loopback tap | An Exclusive stream; anything after the tap |
| Audient **Loop-back 1+2** virtual input | Host playback routed into the iD mixer | The physical DAC output; an Exclusive stream that bypasses the mixer |
| Analogue line-out → line-in capture | Converters, cabling and signal path work end to end | Bit-exact samples: converters, gain and clock drift change them |
| Digital capture directly at the DAC input | Bit-exact delivery to that input | Acoustic quality after the DAC |

WASAPI loopback can only be opened on a **shared-mode** stream; an exclusive-mode stream cannot be
captured with `AUDCLNT_STREAMFLAGS_LOOPBACK`
([Microsoft loopback documentation](https://learn.microsoft.com/windows/win32/coreaudio/loopback-recording)).
Treat a “bit-perfect” label as a software state report until a capture point for the Exclusive
stream has been established.

## 1. Software PCM check (no hardware)

```powershell
$env:RUST_PLAYER_PCM_CHECK_DIR = "$PWD\target\pcm-check"
cargo test --release --locked file_backend_preserves_pcm_bytes_at_multiple_depths_and_rates
ffmpeg -v error -y -i "$env:RUST_PLAYER_PCM_CHECK_DIR\48000-24-padded.wav" -c:a pcm_s32le -f s32le "$env:RUST_PLAYER_PCM_CHECK_DIR\reference.pcm"
Get-FileHash "$env:RUST_PLAYER_PCM_CHECK_DIR\48000-24-padded.pcm", "$env:RUST_PLAYER_PCM_CHECK_DIR\reference.pcm"
```

The two hashes must match. These fixtures are only eight samples long; use a longer signal for a
loopback comparison.

## 2. Prepare a deterministic loopback source

Noise avoids the false alignment caused by a periodic sine wave.

```powershell
$check = "$PWD\hardware-check"
New-Item -ItemType Directory -Force -Path $check | Out-Null
ffmpeg -v error -y -f lavfi -i "anoisesrc=d=10:a=0.25:r=48000" -ac 2 -c:a pcm_s16le "$check\noise-48k-16.wav"
ffmpeg -v error -y -i "$check\noise-48k-16.wav" -f s16le -acodec pcm_s16le "$check\expected.pcm"
```

Use a rate and depth the endpoint accepts in Shared **and** Exclusive mode. For a bit-exact
comparison the Windows mix format must match the source (48 kHz, stereo, 16-bit here); otherwise
the mixer resamples or converts and equality is not expected.

## 3. Shared-mode loopback capture

1. Set the iD4 mk2 as the Windows playback device. Turn off system sounds. Fix the Windows volume
   and the iD Mixer/knob level before the test and do not change them during it.
2. In Rust Player choose **WASAPI Shared**, volume 100%, ReplayGain, DSP, downmix and transition
   fades off. Confirm this in **Signal path**.
3. In Audacity choose Audio Host **Windows WASAPI** and the `(loopback)` input of the playback
   endpoint. On the iD4 mk2 the `Loop-back 1+2` inputs are an alternative tap; confirm with the
   Shared reference first that they actually carry the playback. Record two channels at 48 kHz,
   16-bit PCM.
4. Start recording, then play `noise-48k-16.wav` from the beginning. Stop after about ten seconds
   and export the capture as `capture.wav`.
5. Convert and compare:

```powershell
ffmpeg -v error -y -i "$check\capture.wav" -f s16le -acodec pcm_s16le "$check\captured.pcm"
python -m pip install numpy
python scripts/compare_loopback.py "$check\expected.pcm" "$check\captured.pcm" --format s16le --channels 2 --rate 48000 --seconds 8 --max-offset-seconds 3
```

The script finds the lead-in by FFT correlation and reports the exact-match ratio, maximum and RMS
difference. `PASS` means equality at the capture point. A small nonzero difference usually points
to volume, conversion or resampling; a large difference points to a wrong format, channel count or
capture source. Use `--offset-frames` if the capture starts with a long silence and auto-alignment
cannot find a match. Compare `f32le` instead if the endpoint mix format is float, but convert both
sides to the same format first.

This verifies the Shared path only. It does **not** verify a bit-perfect Exclusive stream.

## 4. Exclusive path and the iD4 mk2

- Repeat the capture with **WASAPI Exclusive** and the bit-perfect preset applied. Check **Signal
  path** for the negotiated format and zero underruns.
- Try the Audient `Loop-back 1+2` inputs. If the Exclusive stream disappears there, the stream
  bypasses the device mixer and that tap cannot prove bit-perfect delivery. If it is present and
  matches the reference sample-for-sample, record it as evidence for this specific capture point.
- For a physical end-to-end check, connect line outputs to line inputs and record the return. This
  is an analogue capture: compare waveform, channel order and level, not individual samples.
- A claim of hardware bit-perfect playback requires a digital capture at the DAC input (external
  recorder or analyser). That equipment is not part of this repository; leave the claim unverified
  until such a capture exists.

The Audient iD4 mk2 is specified for 24-bit/96 kHz and provides virtual loopback inputs
([Audient specifications](https://docs.audient.com/id4/technical-specifications),
[iD4 mk2 features](https://support.audient.com/hc/en-us/articles/13258749159572-iD4-MKII-features)).
It cannot carry DSD64-DoP, which needs a 176.4 kHz PCM transport
([dCS DoP explanation](https://dcsltd.fra1.cdn.digitaloceanspaces.com/media/2014/03/Puccini-Player-Manual-v1_4x.pdf)).

## 5. Listening regression checks

| Fixture | Setting | Check |
| --- | --- | --- |
| Two gapless WAVs from the Phase 0 pair, same rate | Gapless on | No click or silence at the join; title, cover, lyrics, progress and media controls change; one history entry per title |
| Same pair | Gapless off, then shuffle/repeat/seek across the join | Normal track changes; no stuck decoder after a cancelled handoff |
| Stereo PCM at 44.1, 48, 88.2 and 96 kHz | Exclusive, bit-perfect preset | Rate shown in **Signal path** and on the DAC; no underruns; correct channel order |
| 192 kHz PCM, then 16/24/32-bit variants | Conversion off, then Shared fallback | Unsupported native format reports an error; conversion or fallback resumes playback |
| FLAC, MP3, APE, WavPack, DSF/DFF | Seek to start, middle, end and back | Position lands where expected; no click at seek boundaries or track ends |
| Tagged DSF and DFF | Default settings | ID3 title/artist/album/genre/year and duration; **Signal path** shows 88.2 kHz for DSD64, 96 kHz for DSD48-family files |
| External and FLAC-embedded CUE | Adjacent entries | Titles, start and end positions match the sheet |
| Device change while paused, then an import | Default settings | Short status fades after about three seconds; import progress stays visible until it finishes |

Keep a results row per run:

| Windows build / driver | Device and negotiated format | Fixture | Method (Shared loopback / Exclusive tap / analogue / listening) | Under-runs | Result |
| --- | --- | --- | --- | ---: | --- |

Related workflows: [manual release checklist](TESTING.md), [Windows audio checks](AUDIO-TESTING.md).
