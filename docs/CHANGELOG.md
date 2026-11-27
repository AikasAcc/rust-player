# Rust Player – Change Log

Plain-language notes for Windows 10 and 11 users. This file tells you what changed, why you might care, and what did **not** change.

## How to read this file

- **Added** – something new you can use.
- **Fixed** – something that did not work correctly before.
- **Changed** – something works differently now.
- **Still not included** – a feature that is often asked for but is not in this build.

Unless a line says “edit tags” or “ReplayGain”, Rust Player never writes to your music files. Your songs stay where they are.

## v0.7.0

These changes are included in the v0.7.0 Windows download:

**Finding and organizing music**

- **Added:** A **Settings → General → Open from Windows** section lets you register Rust Player as the default app for the audio files it can play. After that, double-clicking a song in Explorer opens it in the player and starts playing it. The association is per user, needs no administrator rights, and can be removed again with the **Remove association** button.
- **Changed:** The horizontal scroll bar under the filter chips is hidden. You can still scroll the row with the mouse wheel or by dragging it; only the visible bar is gone.
- **Added:** A **Liquid glass** option under the background settings adds a slowly moving refractive sheen and a soft glass highlight to wallpapers and cover art. Glass strength and flow speed are adjustable, and the image files are not modified.
- **Fixed:** The **Apply** and **Cancel** buttons on **Settings → Appearance** are now at the top of the theme editor, so they remain visible while the settings below are scrolled.
- **Fixed:** Tags and file names in Japanese, Chinese, Cyrillic, Greek, Arabic, Hindi and other scripts no longer show as empty boxes. Windows system fonts are used as fallbacks for characters the built-in text fonts do not contain; normal English text looks unchanged.
- **Fixed:** If you change a song title in its tags, the saved copy shown in the **Play next** list now updates as well. Before, that copy could keep the old title. The order of the list and your files do not change.
- **Fixed:** A plain search in quotes (for example `"best of"`) now looks for that phrase instead of including the quote marks in the search.
- **Fixed:** An unfinished `missing:` search (for example `missing:tru`) is ignored instead of hiding all your songs.
- **Fixed:** Favorites that were out of step with the Favorites playlist are rebuilt when the library opens. Before, a song could show a filled heart without appearing in Favorites.
- **Fixed:** Playlist cover images are refreshed after a tag or cover change. Before, an old cover could stay on screen until the player restarted.

**Playing music**

- **Fixed:** Monkey's Audio (`.ape`) files now play at the correct volume and channel layout. Before, quiet or multi-channel samples could be wrong.
- **Fixed:** ReplayGain loudness tags are now applied to Monkey's Audio, WavPack and DSD files, just like other formats.
- **Fixed:** Longer FIR correction filters now work across audio packet boundaries. Before, a filter could be cut off early.
- **Fixed:** Seeking while a gapless song change is in progress now stays on the song you still see on screen.
- **Fixed:** A decode problem during a gapless hand-off now shows an error instead of quietly skipping the song.
- **Fixed:** Losing an audio device no longer freezes the player; the problem is shown in the status area.
- **Fixed:** If exclusive audio setup takes too long, the player now closes the worker cleanly instead of leaving it running.

**Saving your data**

- **Fixed:** Saved themes stay in the list after you close and reopen Settings, and the last chosen theme is shown again. Before, a theme created with **Save as new** disappeared unless you pressed **Apply** first.
- **Fixed:** A repeated save request no longer creates a second backup while one is already being written.
- **Fixed:** A damaged queue with extreme IDs is repaired when it loads, so new **Play next** entries always get usable numbers.
- **Fixed:** The same song counted twice in listening history because Windows spelled its path differently is now merged into one entry.
- **Fixed:** Saving lyrics you edited next to a song now replaces the old file instead of reporting “already exists”.
- **Fixed:** A failed temporary-file cleanup no longer marks a successful update as failed.
- **Fixed:** Clicking the update button twice no longer hands the same downloaded file to the updater twice.

**Release package**

- **Fixed:** The Windows download now includes the updater program and the license notices. The updater asset and its checksum are also produced for GitHub releases.

## Latest stable version: v0.7.0

This release adds Explorer double-click, liquid-glass backgrounds, saved themes, wider font fallback and search fixes, plus codec, DSP, persistence and release-package fixes. The default audio settings are unchanged. Automated build, lint and regression checks passed. Hardware capture and listening checks remain user-side checks and are not claimed here.

## Earlier stable release: v0.6.1

This was the previous ready-to-use release for Windows 10/11 x64. It fixed the quick filters from v0.6.0. The published v0.6.0 release is left unchanged. The sound path is unchanged; automated build, lint and regression checks passed.

### In one minute

- The library starts faster and shows titles and cover art sooner.
- More audio file types work.
- Favorites, playlists and the “Play next” list are more reliable.
- Settings are saved more safely, and damaged files are backed up.
- You can filter and search your music like in a real music library.
- If you change nothing in the audio settings, the sound is exactly as before.

## Finding music

- **Filter chips** above the track list let you click **Artist**, **Album**, **Genre** or **Year**. Click the same chip again to remove that filter.
- **All filters** opens the full list of values instead of only the most common ones. The small filter bar can be scrolled sideways with the mouse wheel.
- The playlist search understands plain words as well as `artist:`, `album:`, `title:`, `albumartist:`, `genre:`, `year:`, `path:`, `file:` and `missing:`. Quoted phrases, `AND`, `OR`, `NOT` and a minus sign in front of a word also work.
- **Smart playlists** save a search. You can refresh them later; sorting, favorites and undo/redo still work.
- You can sort by title, artist, album or duration. Each playlist remembers its own sort order.
- Every song row has a heart, so favorites can be set directly in any playlist.

## Playlists and playback

- Double-click a playlist (or **Favorites**) to play it. With shuffle on, a random song starts; otherwise the first song in the visible order plays.
- You can drag songs to reorder “Date added” playlists, select several rows for queue/favorite/remove actions, and undo or redo edits with `Ctrl`+`Z` / `Ctrl`+`Shift`+`Z`.
- The same file can be in several playlists. Each entry stays independent, even if the file name is the same in different folders.
- Favorites stay with the right song after files are reordered, renamed or moved. A favorite whose original song was removed disappears instead of becoming a “ghost”.
- The **Play next** list is restored after a restart. If you delete the song that is currently playing, **Next** now picks the actually next song.
- Tracks added explicitly to the queue are stored in `queue.json`. Old, missing entries are discarded on load.
- Favorites are still a normal playlist. No extra “Favorites” folder is created and no music files are copied or moved.
- M3U/M3U8/PLS playlists can be imported. M3U export can use relative paths, so the playlist and music folder can be moved together.
- Gapless transitions avoid locks in the active work loop, and a stuck handoff gives up after five seconds with a warning instead of waiting forever. The sound itself is unchanged.

## More file types

Now supported:

- **MP1 and MP2** – older MPEG audio files.
- **ADPCM in WAV and CAF** – older recordings.
- **MKA and MKV** – audio inside these containers; in MKV with video, the player picks a playable audio track.
- **Monkey's Audio (`.ape`)** – lossless files with seeking and CUE offsets.
- **WavPack (`.wv`)** – integer, float and hybrid files with seeking and title/artist/album/genre/year/track tags. Hybrid files play from the `.wv` part; the optional `.wvc` correction file is not used.
- **DSF and DFF (DSD)** – converted to high-rate PCM (DSD64 becomes 88.2 kHz) so normal audio devices can play them. This is **not** native DSD or DoP.
- **CUE sheets** – external sheets with one or several audio files, plus CUE text embedded in FLAC files. Each CUE track appears as its own entry.

Still **not** included: SACD-ISO, native DSD/DoP, DSD inside WavPack, ASIO, VST plugins, UPnP/DLNA, video backgrounds and binary FLAC CUEDATA blocks. AAC/MP4 gapless joins are not guaranteed because reliable delay metadata is not available.

Building from source now also needs CMake and LLVM/libclang for WavPack.

## Titles, covers and tags appear faster

- The title appears immediately. Full tags, embedded cover art and lyrics are read in the background and filled in when ready.
- Fast skipping no longer shows details from an older song.
- **Track properties** opens immediately and says “Reading tags…” until the full information arrives. Edits you make in the meantime are not overwritten later.
- In Track properties you can change title, artist, album, album artist and track number.
- With several songs selected, **Edit tags…** can keep, set or clear those fields, plus genre and year. CUE virtual tracks are skipped; edit their source file instead.
- **Scan and write ReplayGain** measures loudness in the background and stores tags only. The music itself is untouched.
- Double-click the cover to open it. **Save next to track** writes `cover.png` beside the song and never replaces an existing cover.
- With Last.fm connected, the now-playing panel offers **♥ Last.fm** and **♡ Last.fm**.

## Settings are safer

- Settings, playlists, listening history and playback state are written in the background. On a normal exit, the player waits until everything is on disk.
- A new file replaces the old one only after it was written completely. A crash during writing does not damage the existing data.
- After saving, the folder itself is flushed, so the new file survives a power failure.
- If a file is unreadable, the player makes a backup such as `library.invalid-…` and rebuilds the data. Old backups are never deleted automatically.
- A missing `queue.json` on first start is normal and does not create a backup.
- A damaged CUE block in a FLAC file is reported in the log folder instead of looking like “no CUE sheet”.
- Updates are downloaded with low memory use: the installer is checked while it is written to disk.

## Sound settings (optional)

- Under **Settings → Audio** there are four resampler levels: Fast, Balanced, High and Maximum. **High is exactly the old setting**, so leaving the setting alone changes nothing.
- **Apply bit-perfect preset** switches to WASAPI Exclusive, no conversion, no DSP, no ReplayGain, no software volume and no fades. If the device cannot accept the original format, playback fails instead of silently processing it.
- FIR correction files can now be twice as long (16,384 instead of 2,048 samples per channel).
- AutoEQ profiles, WAV FIR convolution and stereo crossfeed remain available and off by default.
- [Hardware verification](HARDWARE-VERIFICATION.md) explains what software checks, loopback captures and listening tests can and cannot prove. A digital capture on your device is still needed to claim hardware bit-perfect playback.

## Keyboard shortcuts

`Space` play/pause · `N` next · `P` previous · `S` stop · `M` mute · `F` favorite · `Ctrl`+`S` shuffle · `Ctrl`+`←`/`→` previous/next · `Ctrl`+`Z` undo · `Ctrl`+`Shift`+`Z` or `Ctrl`+`Y` redo. Shortcuts are ignored while typing in a text field.

## For troubleshooting

- `scripts/run-benchmarks.ps1` repeats the four slow tests and saves measurements.
- `scripts/run-frame-log.ps1` records interface frame times while you use the player. It never stops a player that is already open.

## What stays the same

- Existing playlists, favorites, listening history, shortcuts and settings remain valid.
- No music files are moved, renamed or deleted.
- Existing `library.json`, `playback.json` and listening history are reused automatically.

## Please check after installing

1. Two songs with the same sample rate and bit depth one after the other: is the join free of clicks?
2. Skip forward and backward during playback: do positions stay correct?
3. Open a `.mp1`/`.mp2`, an ADPCM WAV and an `.mka` file.
4. Open a FLAC with an embedded CUE sheet: are titles and start positions right?
5. Open a `.ape` file and seek forward and backward.
6. Open a `.dsf` or `.dff` file: are the ID3 title and duration right, and does seeking work?

## Installation

Download the Windows ZIP, extract it and start `Rust-Player.exe`. There is no installer. A `.sha256` file is provided so you can check the download. Keep the updater helper beside the player.

## Older versions

- [v0.5.0 notes](RELEASE-NOTES-0.5.0.md)
- [v0.4.0 notes](RELEASE-NOTES-0.4.0.md)
- [early draft notes](RELEASE-NOTES-DRAFT.md)
