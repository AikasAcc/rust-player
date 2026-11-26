# Playback testing

Run the [Windows build and playback regression checks](AUDIO-TESTING.md) on Windows 10 and 11. Record the Windows build, device and driver version, source format, negotiated output format and underruns. For sample-level loopback captures and a clear boundary between software and hardware claims, use the [hardware verification workflow](HARDWARE-VERIFICATION.md).

## Manual release checklist

- [ ] Start on a clean profile and with an existing library. Check saved volume, playback position, playlist order and theme settings after restarting.
- [ ] Exercise Shared and Exclusive with known-rate stereo PCM. Inspect the DAC rate when it is available and record the software and digital loopback results separately.
- [ ] Switch devices during playback, change the Windows default, unplug and reconnect a saved device, and repeat after sleep.
- [ ] Try matching and mixed-format transitions with Gapless and verify that the new track's metadata appears at the expected time.
- [ ] Use keyboard media keys, Windows media controls and the thumbnail buttons with Rust Player focused and minimized. Switch to another player and confirm that the active Windows media session gets the keys.
- [ ] Apply and cancel theme edits. Drag in still and animated images, vary blur, opacity and fit, and restart. Check a missing image and verify that blur leaves the playlist text and controls sharp.
- [ ] With separate test accounts or fixtures, check Discord artwork, Last.fm authentication and offline recovery, and LRCLIB lookup.
- [ ] Trigger a short status message (for example by changing the audio device while paused) and confirm it fades out after a few seconds. Start an import and confirm its progress text stays visible until the import finishes.

## Exclusive playback

1. Allow applications to take exclusive control of the device in Windows Sound Settings. Set your hardware volume before playback. Open **Settings → Audio**, select the device and choose **WASAPI Exclusive**. Leave the four optional settings disabled for the direct PCM check.
2. Play 16-, 24- and 32-bit PCM at rates supported by your device. Check the negotiated format, channels, rate and underrun counter. A 24-bit track can use packed 24-bit PCM, 24-in-32, or another lossless integer container. A track with an unsupported native format displays an error until a conversion or Shared fallback is enabled. A 192 kHz test file can exercise this behavior on an iD4 MKII.
3. Play a second track with a different rate. The endpoint reopens for the new format. A short gap is expected at this boundary. Check pause, seek, stop, track switches, saved position and device volume. Enable software gain or transition fades individually and verify the corresponding indicator and transport behavior.
4. Check the in-use message when another app holds the endpoint. Enable **Fall back to WASAPI Shared on an Exclusive error** and repeat. **Signal path** should show the actual mode. Repeat after disconnecting and reconnecting the device or resuming Windows from sleep.
5. Verify the rate on the DAC or its control application. For an end-to-end sample comparison, use a verified digital capture point, align the samples, then compare the PCM with [the loopback script](../scripts/compare_loopback.py). A normal WASAPI loopback capture runs in Shared and cannot capture an Exclusive stream. See [hardware verification](HARDWARE-VERIFICATION.md) for the full procedure and the limits of each capture method. [Microsoft WASAPI loopback documentation](https://learn.microsoft.com/windows/win32/coreaudio/loopback-recording)

The [FileBackend verification](AUDIO-TESTING.md#verify-the-pcm-sent-to-the-backend) checks the PCM bytes at the application output using a separate reference decoder.

## Gapless playback

Run `cargo test --release --locked compatible_tracks_write_the_continuous_reference_in_one_stream`. The test splits a continuous sine wave into two WAV files, renders them through one stream and compares every sample with the uninterrupted reference at both the source rate and a resampled output rate.

- Play the two files from the Phase 0 gapless pair in sequence at their original rate. Confirm that the title, cover, lyrics, progress, Windows media controls and plugin details change at the join. The listening history should gain one entry per played title.
- Check shuffle, repeat-one, repeat-playlist, the final title with repeat off and an unreadable file between two valid files. Change the ordering or seek before the join to exercise queue cancellation.
- Turn Gapless off and check the normal track changes. Repeat with different rates to test the output reopening path.

## Taskbar buttons and additional formats

- Minimize the player and hover over its Windows taskbar icon. Check Previous, Play/Pause, Next and Shuffle. Repeat with a fresh window and after the thumbnail toolbar is registered again.
- Open ALAC in M4A, AIFF and PCM CAF through **Add files** and drag-and-drop. Compare the decoded rate, depth and channels in **Signal path**. A file can decode at 384 kHz while the output requires conversion.
- Drop a single-file, multi-file, and FLAC-embedded CUE sheet. Play two adjacent entries and seek within the second one. Compare the titles, starts and ends against the sheet.
- Open a Monkey's Audio `.ape` file and seek in both directions. Confirm that a CUE sheet referencing APE still applies its start offsets.
- Open 16-, 24- and 32-bit integer and 32-bit float `.wv` files. Check rate, depth and channels in **Signal path**, seek in both directions, and verify APEv2 title/artist/album/genre/year/track fields. Repeat with a hybrid file without its `.wvc` correction file and confirm that it decodes from the `.wv` stream.
- Enable ReplayGain Track and Album with tagged fixtures and compare the displayed gain. Test a positive gain with a peak tag and check that gain is capped to the available headroom. Switch ReplayGain off to restore the direct PCM path.
- Enable **Verify FLAC MD5 during playback** and play a complete FLAC from the beginning. A bad embedded checksum produces a warning. Test with a generated copy before changing any files in a library.

## DSD-to-PCM and native DSD

- Open a tagged `.dsf` and `.dff` file. Check the ID3 title, artist, album, genre, year, duration and **Signal path** (88.2 kHz for DSD64, 96 kHz for DSD48-family files). Seek near the beginning, middle and end; listen for clicks at the start and end of the track.
- Native DSD and DoP are not offered. DoP needs a compatible DAC and an unchanged PCM transport; DSD64 uses a 24-bit, 176.4 kHz transport. The Audient iD4 MKII documentation specifies PCM up to 96 kHz, so this interface cannot verify that path. [dCS DoP explanation](https://dcsltd.fra1.cdn.digitaloceanspaces.com/media/2014/03/Puccini-Player-Manual-v1_4x.pdf) · [Audient specifications](https://docs.audient.com/id4/technical-specifications/)

## Library queries and bulk tagging

- Save a playlist search as **Smart**, change the library and refresh it. Confirm that the query filters the current library, that stable track IDs keep favorites and playback order intact, and that **Make normal** freezes the current rows. Restart and verify persistence.
- Click Artist, Album, Genre and Year facet chips above the track list. Verify that one click adds the field filter, a second click removes only that filter, and `genre:`/`year:` searches work after a cache upgrade.
- Select several files and open **Edit tags…**. Set one field, clear another, and leave the rest on Keep. Re-read the files and confirm that untouched fields still hold their old values. Include a CUE virtual track and verify that it is skipped.

## Last.fm scrobbling

- With a Last.fm test account and its API credentials, connect via **Settings → Integrations → Last.fm**. Restart and confirm the Windows credential restores the account. Verify that connecting alone leaves scrobbling off.
- Enable scrobbling and play a tagged 40-second track. Pause before the 20-second listening threshold, seek ahead, then resume. Verify the scrobble arrives once, after 20 seconds of audio have played. Check that tracks without an artist or tagged title and tracks of 30 seconds or less are skipped.
- Enable **Send Now Playing notifications**. Confirm that restoring a paused track leaves the Last.fm profile alone until you press **Play**. Check that a gapless transition reports the new track.
- Temporarily disconnect the network and play another qualifying track. Confirm the pending count appears. Exit and restart, reconnect the network, and verify that the scrobble arrives once. Disconnect the account and confirm that pending scrobbles are cleared.
- Check the Rust Player icon in Explorer and the running taskbar at 100%, 125%, and 150% scaling. On a setup with mixed DPI, move the window between monitors and check that the window icon stays sharp.

## Duration sort and signal path

- Sort a playlist by **Duration** in both directions. Check that equal lengths retain import order and unreadable or unknown lengths stay at the end. Turn shuffle off and verify Next follows the displayed order. Restart and check that sorting is restored.
- In **Settings → Audio → Signal path**, compare a decoded format to the WASAPI format. Check the resampler with different rates and channel mapping with different channel counts. Play a ReplayGain-tagged track and toggle ReplayGain, then compare the reported gain and direct PCM status. Check that integer output reports dither only when the renderer applies it.

## Audio setup, DSP, and Discord artwork

- On a fresh profile, check that the setup dialog lists Windows endpoints and the reported stereo PCM formats. Select an endpoint, test the quiet tone while stopped, and apply Shared or Exclusive. Repeat the PCM check at 44.1 or 96 kHz. The hash covers the software FileBackend. A digital loopback and aligned sample comparison are required for hardware verification.
- Import an AutoEQ preset, enable a mono WAV impulse, and toggle stereo crossfeed. With a compatible output, check that each active stage is reflected in the signal path and turns off the bit-perfect indicator. Reopen with all effects disabled and verify direct PCM again.
- Enable Discord sharing and Apple Music artwork lookup. Play a tagged track with a local album name that differs from its catalog release, and check that the artwork appears in Discord. If a cover is rejected, set a valid application asset as a fallback. The desktop acknowledgment and rendering in the Discord profile are separate checks.

## Initial Windows performance measurements

Single trial on Windows x64 with an AMD Ryzen 7 5700X. Startup measures time to Windows input idle, and working set is sampled five seconds later. The 10,000-track Rust Player profile used generated WAVs. Playback CPU is process CPU time over six seconds, expressed as a percentage of one logical core.

| Player and test | Input idle | Working set | Playback CPU |
| --- | ---: | ---: | ---: |
| Rust Player, empty profile | 226 ms | 144.4 MiB | — |
| Rust Player, 10,000 tracks | 164 ms | 97.7 MiB | 2.86% |
| Portable foobar2000 2.25.7, empty profile | 139 ms | 44.3 MiB | — |
| Portable foobar2000 2.25.7, 10,000 tracks | — | 192.3 MiB | 2.60% |

Both players ran the same synthetic track with the 10,000-track fixture. The portable foobar2000 profile contained all 10,000 distinct synthetic filenames. Its large-list working set was sampled during playback after import. These single runs have different startup and working-set sampling conditions.

## Saved themes and backgrounds

- Start with an existing settings file that has no saved themes. Change colors or font, create a named theme, apply and restart. Load it, rename it, duplicate it, and delete one copy. Cancel a second edit and verify that the previously applied theme and saved names return.
- Choose and drop a PNG or GIF while the editor is open. Verify fit, opacity, darkening, blur, and removal. Set blur to zero, then drag it quickly while playing music and confirm the newest setting wins. Confirm that dropping audio still imports a track. Copy a background into the profile, move the original, and restart. Move an uncopied background to test the color fallback.
- Enable **Current song cover** and play tracks with different artwork, including a track without artwork. Check that the background changes with each track and falls back to the theme color when artwork is absent or playback is stopped.
- Enable **Animate blur** for a cover and a custom image. Check that each background slowly softens and sharpens, existing GIF frame timing stays intact, and the animation pauses when configured.
- Play audio while comparing a still image and an animated GIF/WebP. Verify variable frame delays, playback while the window stays visible, and that the animation pauses on minimize and on losing focus when selected. Large files should report the limit while audio continues.

The Phase 6 preview played the synthetic WASAPI track in both visible-window trials with 10,000 tracks in the profile. The animated run used a 480 × 270 GIF with 16 frames and 50 ms and 100 ms frame delays. CPU is measured against one logical core. NVIDIA's GPU counter is system-wide and includes other windows and the compositor.

| Background | Working set | Process CPU | System GPU average |
| --- | ---: | ---: | ---: |
| Disabled | 100.2 MiB | 100.30% | 34.2% |
| GIF | 103.6 MiB | 11.28% | 17.5% |

Visible-window CPU and GPU readings varied substantially across runs. These samples confirm playback during the test and do not isolate the incremental cost of the GIF.
