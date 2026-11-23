# Rust Player

<img width="1120" height="680" alt="image" src="https://github.com/user-attachments/assets/e49dc84f-0464-418e-ab9a-be79d1d7a1c4" />

A standalone Windows x64 audio player written in Rust with egui and Rodio. It supports local music libraries, editable playlists, album artwork, synchronized lyrics, themes, and script plugins.

## Quick start

Download the Windows x64 ZIP from **Releases**, extract it, and open `Rust-Player.exe`. To build from source on Windows, run `cargo build --release --locked` with a current Rust toolchain. For a development run, use `cargo run --locked`.

## Playlists and playback

The file picker supports MP3, FLAC, WAV, Ogg Vorbis, M4A, and AAC. Click **+** next to **Playlists** and enter a name. Drop audio files and folders anywhere in the window to add them to the selected playlist. Folder import scans subfolders. You can also use **Add files** and **Add folder**. Double-click a track to play it. The shuffle button randomizes the remaining order without repeating tracks within that run. Playlists and settings are saved in `%LOCALAPPDATA%\Rust Player\library.json`.

The **Now playing** panel reads title, artist, album, cover and lyrics from available tags. If a cover is missing, put a file such as `cover.jpg` or `folder.png` next to the track. For lyrics, put a same-name `.lrc` or `.txt` file next to the track. Timestamped LRC lyrics highlight and follow the current line.

Use **Sort by** to choose **Title**, **Artist** or **Album**, then choose **A–Z** or **Z–A**. Each playlist remembers its sort. Tracks without an artist or album tag go at the end of that sort. **Date added** restores the original order. Playback follows the displayed order when shuffle is off.

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

## Plugins

Open **Plugins**, choose **Add plugin…**, and select a local `.rhai` file. **Open Plugin Folder** opens the default folder for that picker in Explorer. Tick the plugin's checkbox to enable it. **Reload** picks up edits to the script. Try [track-summary.rhai](examples/track-summary.rhai), then load a song and look under **Now playing → Plugin details**.

The v1 plugin hook is `fn on_track(title, artist, album)`. It receives three strings when a track is loaded and returns a short line of plain text. Rhai scripts run with execution and memory-related limits, without module imports or host functions for file and network access. Plugins use the selected file path and are disabled when first added.

## Discord Rich Presence

1. Open the Discord desktop client, then click **Discord** in Rust Player.
2. Enter the numeric Application ID for your app in the [Discord Developer Portal](https://discord.com/developers/applications).
3. Enable **Share what I’m listening to**. While playback is active, the player sends the title, artist and playback time to Discord. The album is shown in the artwork tooltip when available. Pausing or stopping clears the activity.
4. Enable **Look up artwork via Apple Music** to find matching public artwork automatically. This sends the tagged title and artist to Apple's catalog. You can also save a public HTTPS image URL for the current track or set a default Discord asset key. Embedded and folder images are displayed locally.

**Refresh presence** sends the current playback state again. The Discord window reports whether the desktop client confirmed the activity or returned an error.

[Discord artwork support](https://discord.com/developers/docs/social-sdk/classdiscordpp_1_1ActivityAssets.html) · [Apple Search API](https://performance-partners.apple.com/search-api)

## Playback and local data

Click or drag the progress bar to seek. Scroll over the **Volume** control to adjust the volume. On restart, the last track and playback position are restored in a paused state.

Rust Player stores playlists, playback position, listening activity, themes, fonts, plugin paths, and integration settings under `%LOCALAPPDATA%\Rust Player`. Audio files stay at their original paths. Apple Music artwork lookup, LRCLIB search, and Discord sharing are separate features you can enable in the interface.
