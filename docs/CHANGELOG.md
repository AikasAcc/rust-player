# Rust Player v0.6.0-rc1 – What's New

This is a **preview** (release candidate). The current stable version remains **v0.5.0**. Please report any problems before this becomes a full release.

## In Short

The player starts faster, shows titles and cover art without long waits, saves your data more safely, and plays more audio files. The sound itself is unchanged.

## More Audio Files Work

Newly supported:

- **MP1 and MP2** – older MPEG audio files
- **ADPCM in WAV and CAF files** – older recordings
- **MKA and MKV** – audio containers. Even if they contain a video track, the player finds the audio track.

Still **not** supported: WavPack, Monkey's Audio, DSD, ASIO, VST plugins, and UPnP.

## Playback Runs More Smoothly

- The player no longer checks for a new song on every small work step. Under heavy load, this could cause brief dropouts.
- If a transition between two songs gets stuck, the player gives up after five seconds and reports it instead of waiting forever.
- **The sound is unchanged** – this is only about stability.

## Titles and Cover Art Appear Immediately

- Previously, the whole file had to be read before the title appeared. Now you see the title immediately; cover art and details follow shortly after.
- When you skip through several songs quickly, the display stays on the right song.
- The **Track properties** window opens immediately. Anything you type in the meantime is not overwritten by late worker results.

## Your Data Is Better Protected

- Settings, playlists, and listening history are saved in the background. On closing, the player waits briefly until everything is actually on disk.
- After saving, the folder itself is also flushed. This ensures the new file survives a power failure.
- If a file is corrupted, the player creates a backup copy and rebuilds the data. Old backups are **never deleted automatically**. A not-yet-existing queue file is not treated as corrupted.
- A damaged CUE block inside a FLAC file is reported in the log folder instead of being silently treated as an album without a sheet.
- Updates are downloaded with low memory usage: the installer is checked while it is being written to disk.

## Small Usability Improvements

- A heart in every song row – favorites directly in the list.
- Playlist search supports field filters, quoted phrases, boolean operators and negation.
- Deleting or relinking a source track refreshes Favorites immediately; scrolling cannot run into an outdated row.
- Double-clicking a playlist or Favorites starts playback.
- Drag tracks to reorder "Date added" playlists; select several tracks for queue, favorite, or remove actions.
- Undo/redo playlist edits with `Ctrl`+`Z` / `Ctrl`+`Shift`+`Z` (or **Tools**).
- More global shortcuts: `Space`, `N`, `P`, `S`, `M`, `F`, `Ctrl`+`S`, `Ctrl`+`←`/`→`.
- M3U export with relative paths for portable playlists.
- Explicitly queued tracks are restored after a restart.
- Save album art as `cover.png` beside the audio file; existing covers are kept.
- Love/unlove the current track on Last.fm from the now-playing panel.
- GitHub button under **Tools** and under **Settings → About**.

## Sound and Tags

- Under **Settings → Audio** there are four quality levels. **"High" is unchanged** – if you change nothing, you'll hear no difference.
- Correction files (FIR) can be twice as long.
- **ReplayGain** can be measured in the background. Only the loudness information is written to the file; the music remains untouched.
- In the track properties window, you can change title, artist, album, album artist, and track number.
- External CUE files can reference multiple audio files.
- CUE sheets embedded in FLAC files are now split into individual tracks.

## Diagnostics for Troubleshooting

- `scripts/run-benchmarks.ps1` repeats the four ignored benchmarks and archives raw data with CPU and memory metadata.
- `scripts/run-frame-log.ps1` records one CSV row per UI frame only while it runs and summarizes p50/p95/p99/max after the player closes. It never stops a player that is already open.

## What Stays the Same

- Existing shortcuts, playlists, favorites, listening history, and settings remain valid; the listed shortcuts and editing actions are additions.
- No music files are moved, renamed, or deleted.
- Existing `library.json`, `playback.json`, and listening history are reused automatically.

## Please Check in the Pre-Release

1. Two songs with the same sample rate and bit depth one after the other: is the transition free of clicks?
2. Skip forward and backward during playback: no stuck or incorrect positions?
3. Load a `.mp1`/`.mp2`, an ADPCM WAV, and a `.mka` file.
4. Open a FLAC with an embedded CUE sheet: are the individual titles and track starts correct?

## Installation

Download the ZIP, extract it, and start `Rust-Player.exe`. There is no installer. The SHA-256 checksum is provided alongside it as a `.sha256` file.
