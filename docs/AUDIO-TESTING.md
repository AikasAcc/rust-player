# Windows audio checks

Run `cargo build --release --locked`, `cargo fmt --check`, `cargo clippy --release --locked --all-targets`, and `cargo test --release --locked` on Windows x64.

The hardware-free tests validate resampler output length and a complete WAV decode, resample and render through the Null backend. Hardware tests cover Windows 10 and 11 independently.

- Select the Windows default output. Play a stereo WAV and confirm the rate, channel count, output format and buffer in **Settings → Audio**. Try a different source rate and confirm that sinc resampling becomes active.
- If two outputs are available, switch between them during playback. Check that the title and position are retained, playback is paused and resuming does not count the title again.
- With a saved device selected, disconnect it and verify fallback to the Windows default. Reconnect it and verify that the saved preference is restored. Repeat with **Windows default** selected and change the default in Windows Sound Settings.
- Seek forward and backward, pause and resume, and close and reopen the player. Check the paused position, saved volume, shuffle and title sorting, play counts, local lyrics, the example Rhai plugin, and the taskbar and Windows media controls.
- Test Discord Presence with an opted-in desktop client. Verify track changes, seek position updates and clearing on pause.
- Play while scrolling a large playlist and watch the underrun counter. On an Audient iD4 MKII, check 44.1, 48, 88.2 and 96 kHz source material. Shared output follows the Windows mix format.

See [Exclusive playback checks](TESTING.md#exclusive-playback) for the device test procedures.

## Verify the PCM sent to the backend

The FileBackend test passes deterministic fixtures through the same decoder, ring buffer and sample-packing functions as Exclusive. It asserts exact output bytes for 16-, 24-, 24-in-32 and 32-bit signed PCM and 32-bit float, at 44.1, 48, 96 and 192 kHz.

```powershell
$env:RUST_PLAYER_PCM_CHECK_DIR = "$PWD\target\pcm-check"
cargo test --release --locked file_backend_preserves_pcm_bytes_at_multiple_depths_and_rates
ffmpeg -v error -y -i "$env:RUST_PLAYER_PCM_CHECK_DIR\48000-24-padded.wav" -c:a pcm_s32le -f s32le "$env:RUST_PLAYER_PCM_CHECK_DIR\reference.pcm"
Get-FileHash "$env:RUST_PLAYER_PCM_CHECK_DIR\48000-24-padded.pcm", "$env:RUST_PLAYER_PCM_CHECK_DIR\reference.pcm"
```

The SHA-256 values should match. The other fixture variants use `s16le`, `s24le`, `s32le` or `f32le` according to their output format. FFmpeg is used only to produce an independent test reference.

## Audio dependencies

- CPAL 0.18 (Apache-2.0) exposes stable endpoint IDs and WASAPI streams. Its realtime feature registers the Windows callback thread with the MMCSS Audio profile. [CPAL](https://github.com/RustAudio/cpal)
- Symphonia 0.5.5 (MPL-2.0) was already in the lockfile. It now provides the decoder directly. [Symphonia](https://github.com/pdeljanov/Symphonia)
- wavpack 0.4.0 (BSD-3-Clause) wraps the reference libwavpack for `.wv` files. It builds the bundled C library with CMake and generated bindings with libclang; both tools are build-time requirements, not runtime dependencies of the player. [wavpack-rs](https://github.com/irh/wavpack-rs)
- rtrb 0.3 (MIT or Apache-2.0) provides a bounded, wait-free single-producer single-consumer buffer. [rtrb](https://github.com/mgeier/rtrb)
- Rubato 0.16.2 (MIT) provides the sinc resampler. The code uses its fixed-input API with the FFT feature disabled. [Rubato](https://github.com/HEnquist/rubato)
