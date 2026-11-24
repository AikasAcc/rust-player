# Rust Player 0.2.0

This release is for Windows x64.

## Playback and audio

- Select WASAPI Shared or Exclusive and inspect the negotiated format, processing stages, buffer and underruns under **Settings → Audio → Signal path**.
- Play compatible tracks gaplessly. Use ReplayGain, AutoEQ imports, WAV FIR convolution and stereo crossfeed as optional processing.
- Import ALAC, AIFF, supported PCM CAF and external single-file CUE sheets. FLAC verification can check the embedded MD5 during uninterrupted playback from the beginning.
- Control playback with media keys and Windows taskbar thumbnails. Restore volume and the last track on restart.

## Library and appearance

- Manage, pin, import and export playlists. Sort by title, artist, album or duration, and use Favorites and listening history.
- Save named themes that include colors, font and playlist background. Use the current song cover, a still image, a GIF or an animated WebP, with fit, opacity, darkening and animated blur controls.
- Read local artwork and synchronized lyrics. Optional integrations provide LRCLIB searches, Discord Rich Presence artwork and Last.fm scrobbling with an offline queue.

## Compatibility

- Exclusive output depends on the Windows endpoint and driver. Device format changes can cause a short pause. Check the [signal-path description](../README.md#audio-output) and [manual test checklist](TESTING.md).
- WavPack, Monkey's Audio, DSD/DoP, ASIO, video backgrounds, and embedded or multi-file CUE are not included in this release.
