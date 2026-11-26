# Rust Player v0.6.1 – What's New

This is the stable **v0.6.1** patch release. It succeeds **v0.6.0** without changing the audio signal path. All automated build, lint and regression checks passed. Hardware bit-perfect capture and listening checks are described in `docs/HARDWARE-VERIFICATION.md` and are performed by the maintainer.

**Facet filter fixes:** The facet bar now has an **All filters** button. It expands every cached Artist, Album, Genre and Year value into a scrollable, wrapped panel instead of showing only the six most frequent values of each kind. The compact bar is horizontally scrollable with the mouse wheel (no Shift required). Clicking an already active chip reliably removes its filter, including when two filters are active and the values contain spaces. The published `v0.6.0` release is left unchanged.

## In Short

The player starts faster, shows titles and cover art without long waits, saves your data more safely, and plays more audio files. The sound itself is unchanged.

## More Audio Files Work

Newly supported:

- **MP1 and MP2** – older MPEG audio files
- **ADPCM in WAV and CAF files** – older recordings
- **MKA and MKV** – audio containers. Even if they contain a video track, the player finds the audio track.
- **Monkey's Audio (`.ape`)** – lossless files with seeking and CUE offsets.
- **DSF and DFF (DSD)** – converted to high-rate PCM (DSD64 → 88.2 kHz) so normal audio devices can play them. This is *not* native-DSD/DoP playback.
- **WavPack (`.wv`)** – lossless, hybrid and float files with seeking and APEv2 tags. Hybrid files use the `.wv` stream without an optional `.wvc` correction file; DSD inside WavPack is not covered.

Still **not** supported: SACD-ISO, native DSD/DoP, DSD in WavPack, ASIO, VST plugins, and UPnP/DLNA. Building from source now also needs CMake and LLVM/libclang for libwavpack.

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
- **Smart playlists** – save a query and refresh it from the library; sorting, favorites, undo/redo and stable track IDs still work.
- **Facet chips** above the track list for Artist, Album, Genre and Year, with new `genre:` and `year:` search fields.
- **Bulk tag editing** for a multi-row selection: Keep, Set or Clear title, artist, album, album artist, genre and year.
- **Fading status messages** – transient notices disappear after a short delay; progress and audio-error text remain until the work or failure state ends.
- **Hardware verification guide** – a step-by-step Shared-mode loopback comparison with a sample-alignment script, plus an explicit boundary between software checks, digital captures and listening tests.

## Sound and Tags

- Under **Settings → Audio** there are four quality levels. **"High" is unchanged** – if you change nothing, you'll hear no difference.
- Correction files (FIR) can be twice as long.
- **ReplayGain** can be measured in the background. Only the loudness information is written to the file; the music remains untouched.
- **Apply bit-perfect preset** in Settings → Audio: WASAPI Exclusive, no conversion, no DSP, no ReplayGain, hardware volume only.
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
5. Load a `.ape` file and skip forward/backward.
6. Load a `.dsf` or `.dff` file: are the ID3 title and duration right, and does seeking work? Listen for clicks at the start and end of a track.

## Installation

Download the ZIP, extract it, and start `Rust-Player.exe`. There is no installer. The SHA-256 checksum is provided alongside it as a `.sha256` file.
