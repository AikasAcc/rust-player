# Rust Player

<img width="1120" height="680" alt="image" src="https://github.com/user-attachments/assets/7756e401-8275-47af-87b2-ae6a769be733" />

A standalone Windows x64 audio player written in Rust with egui, Symphonia, and WASAPI. It supports local music libraries, editable playlists, album artwork, synchronized lyrics, themes, and script plugins.

## Quick start

Rust Player targets Windows 10 and 11 on x64. Download the Windows ZIP from [GitHub Releases](https://github.com/AikasAcc/rust-player/releases), extract it, and run `Rust-Player.exe`. The executable is unsigned, so Windows SmartScreen or antivirus software may flag it. The release includes a SHA-256 checksum for verifying the ZIP.

To build from source, run `cargo build --release --locked` with a current Rust toolchain. Use `cargo run --locked` for a development run.

Building from source requires the Windows SDK resource compiler with the MSVC toolchain, or MinGW-w64 `windres` and `ar` on PATH with the GNU toolchain. The executable embeds the [Rust Player logo](assets/ATTRIBUTION.md) at 16 sizes. The app sets DPI-appropriate small and large window icons.

## Settings and media keys

Open **Settings** from the toolbar. **General** controls keyboard media keys. **Audio** selects the output device and shows the audio path. **Playback** controls startup restore and lets **Previous** restart the current track once playback passes three seconds. **Appearance** opens the theme and font editor. **Integrations** provides access to lyrics, Discord, Last.fm, and plugins.

Windows System Media Transport Controls handle **Play/Pause**, **Previous** and **Next** while Rust Player is the active Windows media session, including when its window is minimized. Windows can show the track title, artist, album and local cover in its media controls. If registration fails, **General** reports whether the keys are available while Rust Player has focus.

Open **Settings → About** to view the local crash log folder or copy format and output diagnostics. **Report a bug** opens the GitHub issue chooser. Paste the diagnostics yourself if they help describe the issue.

If `library.json` cannot be read, Rust Player saves a copy as `library.invalid-*.json` next to the original before saving new settings. If the backup fails, Rust Player leaves the original file in place.

## Audio output

The player decodes MP3, FLAC, WAV, Ogg Vorbis, AAC, M4A/ALAC, AIFF, and supported audio inside CAF. WavPack, Monkey's Audio and DSD are currently unsupported. CAF and AIFF containers can also hold encodings the player cannot decode.

Open or drop an external single-file CUE sheet to add its tracks with their `INDEX 01` boundaries and titles. The CUE sheet and its source file stay on disk. Rust Player can re-import its own M3U8 exports containing these entries. Other players can open the original CUE sheet.

**Settings → Audio** includes ReplayGain by track or album, preamp and peak-based clipping protection. ReplayGain runs before resampling and disables the bit-perfect indicator. You can also disable downmix and enable FLAC verification against the embedded MD5 during continuous playback. A seek or a CUE segment does not verify the entire FLAC.

**Gapless playback** is on by default under **Settings → Playback**. The player prepares the next item in the active playback order. Tracks with the same decoded sample format, sample rate and channel layout can share the active stream and keep the resampler running through the join. Format changes reopen the output for the next title. The **Repeat** setting can repeat the current track or the playlist.

WAV and FLAC have no encoder delay. MP3 uses delay and padding metadata when available. The current AAC/MP4 decoder does not expose reliable trimming for encoder delay and padding, so a gapless AAC/M4A join is not guaranteed.

Choose **Windows default** or a specific endpoint in **Settings → Audio**. A saved device is identified by its Windows endpoint ID. If it is unavailable, the player uses the Windows default and retains your preference. A change of output pauses the current track and restores its position. Press **Play** to continue.

**WASAPI Shared** is the default. In **Settings → Audio**, select **WASAPI Exclusive** to request the track's original rate and channel layout directly from the device. Exclusive bypasses software volume by default. Use your audio device to adjust listening volume. Format conversion, fallback to Shared, software gain and transition fades are separate opt-in settings.

**Settings → Audio → Signal path** shows each processing stage, its status, and its PCM format, from the decoder through ReplayGain, channel mapping, resampling, volume and dither to WASAPI and the selected device. It also shows buffer size and underruns. The player reports **Bit-perfect PCM at the Windows driver input** during direct Exclusive playback with the original samples, rate and channel layout. Lossless integer padding preserves the sample values. A digital capture is needed to verify the hardware path.

**Audio setup and PCM check** lists the shared mix format and driver-reported stereo Exclusive rates and bit depths. You can play a quiet half-second test tone, apply the selected endpoint and mode, or verify the generated PCM against the bytes written by the software FileBackend. The PCM hash tests the software path. See [hardware loopback testing](docs/TESTING.md#exclusive-playback) for end-to-end verification.

**Settings → Audio → DSP** offers AutoEQ parametric profile import, mono or stereo WAV FIR convolution, and stereo crossfeed. Effects are off by default. FIR is limited to 2,048 samples per channel at the output rate. Active DSP changes the PCM and disables the bit-perfect indicator.

The decoder writes to a bounded ring buffer on a worker thread. The output callback reads the buffer without file access, locks, or allocation. The shared backend uses CPAL's Windows MMCSS audio priority. The Null backend exercises the same pipeline in `cargo test --release --locked`. See [Audio testing](docs/AUDIO-TESTING.md) for the test procedure and dependency notes.

## Playlists and playback

The file picker accepts the formats listed under [Audio output](#audio-output). Click **+** next to **Playlists** and enter a name. Drop audio files and folders anywhere in the window to add them to the selected playlist. Folder import scans subfolders. You can also use **Add files** and **Add folder**. Double-click a track to play it. The shuffle button randomizes the remaining order without repeating tracks within that run. Playlists and settings are saved in `%LOCALAPPDATA%\Rust Player\library.json`.

The **Now playing** panel reads title, artist, album, cover and lyrics from available tags. If a cover is missing, put a file such as `cover.jpg` or `folder.png` next to the track. For lyrics, put a same-name `.lrc` or `.txt` file next to the track. Timestamped LRC lyrics highlight and follow the current line.

Use **Sort by** to choose **Title**, **Artist**, **Album**, or **Duration**. For duration, choose **Shortest first** or **Longest first**. Tracks with unknown durations stay at the end. Each playlist remembers its sort. **Date added** restores the original order. Playback follows the displayed order when shuffle is off.

The playlist shows album headers and columns for the playing marker, disc/track number, title/artist, and duration. **Sort by → Album** gathers matching albums and orders their tracks by disc and track number. Other sort modes show headers wherever the album changes in that order.

Hover the Rust Player icon on the Windows taskbar to use **Previous**, **Play/Pause**, **Next**, and **Shuffle** in the thumbnail preview.

Right-click a playlist for **Pin**, **Rename**, **Copy**, **Delete**, and **Export**. The **Favorites** playlist appears first, followed by playlists you have pinned. Drag a playlist within its pinned or unpinned group to change its position. Right-click a pinned playlist and choose **Unpin** to restore its original position. Copy makes another playlist inside Rust Player. Export creates a UTF-8 M3U8 with absolute file paths in the playlist's stored order. Use **Import…** to read local tracks from M3U, M3U8, or PLS files. Relative paths are based on the imported playlist's folder. Remote streams are skipped.

Right-click a track to remove it from the playlist. In **Date added** order, the same menu lets you move tracks up or down. These actions edit playlist entries and leave audio files on disk. The player restores your saved volume on startup.

Press the star in the playback controls to add or remove the current track from **Favorites**. Use **Listening activity** to see your latest 1,000 play starts, most played songs, and most played artists. Play counts and history are stored locally. Pausing and resuming the same loaded track in the same session keeps its existing play count.

## Lyrics search

Use **Search LRCLIB** for the current track, or enable **Find missing lyrics online** to search automatically when local lyrics are unavailable. Online search sends the track title and artist to [LRCLIB](https://lrclib.net/docs). A matching timed result scrolls during playback. **Save lyrics** writes a same-name `.lrc` or `.txt` file beside the track without overwriting an existing file.

The local-first lyrics workflow takes inspiration from [OpenLyrics](https://github.com/jacquesh/foo_openlyrics).

## Themes

Open **Theme** in the toolbar to preview a preset or edit the background, panel and button, text, and accent colors. **Choose font…** previews a local TTF or OTF as the interface font. **Apply** saves the theme and font path for the next start. **Cancel** restores the previously applied theme. If a font file becomes unavailable, the player uses its default font.

Use **Saved themes** to save your current colors, font and playlist background under a name, then load, update, rename, duplicate or delete it. Changes in the editor take effect permanently with **Apply**.

Choose or drop an image while the Theme Editor is open. The playlist supports PNG, JPEG, WebP, GIF, BMP, TIFF and ICO, with Cover, Contain, Stretch, Tile and Center, as well as opacity, darkening and blur. Backgrounds and blur are off by default. GIF and animated WebP play at up to 30 FPS and can pause when the window loses focus. The player falls back to the theme color if the file is missing or invalid. Select **Copy the image into the player profile on Apply** to keep a local copy independent of your original file.

Choose **Current song cover** as your background source to follow the artwork shown in **Now playing**. Tracks without artwork use the theme color. **Animate blur** gently cycles between the set blur and a softer view of the same image. It works with image files and song covers and uses the same pause-on-focus-loss setting.

## Plugins

Open **Plugins**, choose **Add plugin…**, and select a local `.rhai` file. **Open Plugin Folder** opens the default folder for that picker in Explorer. Tick the plugin's checkbox to enable it. **Reload** picks up edits to the script. Try [track-summary.rhai](examples/track-summary.rhai), then load a song and look under **Now playing → Plugin details**.

The v1 plugin hook is `fn on_track(title, artist, album)`. It receives three strings when a track is loaded and returns a short line of plain text. Rhai scripts run with execution and memory-related limits, without module imports or host functions for file and network access. Plugins use the selected file path and are disabled when first added.

## Discord Rich Presence

1. Open the Discord desktop client, then click **Discord** in Rust Player.
2. Enter the numeric Application ID for your app in the [Discord Developer Portal](https://discord.com/developers/applications).
3. Enable **Share what I’m listening to**. While playback is active, the player sends the title, artist and playback time to Discord. The album is shown in the artwork tooltip when available. Pausing or stopping clears the activity.
4. Enable **Look up artwork via Apple Music** to find public artwork. This sends track metadata to Apple's catalog. The lookup prefers an album match and can fall back to the same song and artist from another release. You can also save a public HTTPS image URL for the current track or a default asset from your Discord application. Embedded and folder images are displayed locally.

**Refresh presence** sends the current playback state and retries an unsuccessful catalog lookup. The Discord window shows the lookup result and whether the desktop client accepted the artwork.

[Discord artwork support](https://discord.com/developers/docs/social-sdk/classdiscordpp_1_1ActivityAssets.html) · [Apple Search API](https://performance-partners.apple.com/search-api)

## Last.fm scrobbling

1. Create a [Last.fm API account](https://www.last.fm/api/account/create) to get an API key and shared secret.
2. Open **Settings → Integrations → Last.fm**, enter both values, and select **Connect to Last.fm**.
3. Approve the connection in your browser, return to Rust Player, and select **Finish connection**.
4. Turn on **Scrobble to Last.fm**. Optionally enable **Send Now Playing notifications** to publish the current track when playback starts. Both are off by default.

The player scrobbles tagged tracks longer than 30 seconds after half the track or four minutes of audio have played, whichever comes first. It sends artist, title, duration, and playback start time, along with album, album artist, track number, and MusicBrainz recording ID when available. Windows Credential Manager stores the API credentials and session. Pending scrobbles are stored in `lastfm-queue.json` in the player profile and retried after temporary errors, including after a restart. Use **Disconnect** to forget the saved connection and clear pending scrobbles. [Last.fm scrobbling rules](https://www.last.fm/api/scrobbling)

## Playback and local data

Click or drag the progress bar to seek. Scroll over the **Volume** control to adjust the volume. On restart, the last track and playback position are restored in a paused state.

Rust Player stores playlists, playback position, listening activity, themes, fonts, plugin paths, and integration settings under `%LOCALAPPDATA%\Rust Player`. Audio files stay at their original paths. Apple Music artwork lookup, LRCLIB search, Discord sharing, and Last.fm scrobbling are separate features you can enable in the interface.

## Known limits and troubleshooting

| Symptom | What to check |
| --- | --- |
| Exclusive playback fails | Check the selected device's rates and channel layout, Windows exclusive-access settings, and whether another app owns the endpoint. Use Shared if you need both applications to play concurrently. |
| The bit-perfect indicator is missing | Check **Signal path** for the actual output mode, resampling, channel mapping, ReplayGain, software gain, DSP, transition fades and underruns. Use the [PCM and DAC checks](docs/TESTING.md#exclusive-playback) to verify the relevant part of the path. |
| A transition has a short gap | Check **Gapless playback** and the formats on both sides of the join. Rate or format changes require reopening the output. AAC/MP4 gapless trimming depends on the available metadata and decoder. |
| Windows media keys control another app | Enable **Settings → General → Use keyboard media keys** and make Rust Player the active Windows media session by starting playback. |
| Discord shows a placeholder | Enable artwork lookup or supply a public HTTPS cover URL or application asset. Check the lookup and RPC status in the Discord dialog. |
| A background falls back to the theme color | Open **Theme Editor** to check the file path and decoder message. Background files are limited to 64 MiB. An animation can contain up to 600 frames and 48 MiB of decoded image data after resizing. |

WavPack, Monkey's Audio, DSD/DoP, ASIO output, and video backgrounds are outside this build. CUE support is for external single-file sheets. See the [manual regression checklist](docs/TESTING.md), [release notes draft](docs/RELEASE-NOTES-DRAFT.md), and [completion report](docs/COMPLETION-REPORT.md).

## License

Rust Player is available under the [MIT License](LICENSE). Windows release archives include the third-party dependency notices.
