# Rust Player – Changes in the Next Version

This version makes the library, favorites, and playback significantly more robust.
Two small actions have been added to the interface. Your files stay where they are.

---

## Short Summary (for the Release Page)

- **Favorites stay with the right file** – even after songs are reordered, renamed, or moved.
- **The last played song starts reliably again** – at the right position, even after changes to playlists.
- **"Next" selects the right track again** – especially after deleting the currently playing song.
- **Duplicate files stay separate** – the same file in two playlists remains two independent entries.
- **No data loss from corrupted settings** – unreadable files are backed up, not overwritten.
- **Old data is migrated automatically** – existing `library.json` and `playback.json` continue to work.
- **The library starts faster** – the metadata cache is refreshed only when something actually changes, and is cleaned up regularly.
- **No new favorites folder** – favorites remain a playlist; original files are neither copied nor moved.
- **Adjust resampling quality** – four resampler quality levels instead of a fixed setting; the previous sound quality remains the default.
- **Longer FIR filters supported** – now 16,384 instead of 2,048 taps per channel.
- **Write ReplayGain yourself** – one click in Track properties measures the file and saves the tags.
- **Edit tags directly** – title, artist, album, album artist, and track number can be changed.
- **CUE with multiple files** – albums with one audio file per track are now supported as well.
- **Heart icon next to each song** – favorites can now be set and removed line by line in any playlist.
- **Start a playlist with a double-click** – double-clicking a playlist plays it; with shuffle enabled, a random track starts.
- **The UI stays responsive while saving** – settings are written in the background; on exit, everything is fully written.
- **Smooth import and export** – playlist files are read and written in the background.

---

## Detailed Version (for End Users)

### Favorites

Favorites are now firmly linked to the respective song – not just to its file path.

What you'll notice:

- You can reorder, rename, or move songs on disk within a playlist: the favorite star stays with the right song.
- When you add a song from another playlist, the link to the original is preserved. If the original song is removed, the stale favorite disappears too – instead of remaining as a "ghost".
- If a song is removed or moved directly from favorites, the mapping remains correct.
- The favorite star in the header shows the correct state even when you're playing directly from the favorites playlist.
- Every song row now has its own heart between the number and the title. Clicking it sets or removes the favorite – even directly in the favorites playlist. A filled heart is red; an empty one is grayed out.

Important: **no** "Favorites" folder is created. Favorites remain a normal playlist in `library.json`. Your music files are neither copied nor moved.

### Playback and Queue

- On startup, the player remembers the last played song and the position. This now works reliably even if playlists have changed in the meantime.
- Old saved playback data without the new mapping is automatically migrated and updated on first load.
- If the currently playing song is deleted from a playlist, "Next" jumps to the actually next track – no longer accidentally to the previous one.
- The queue ("Up Next") remains consistent even after songs are removed or moved.
- Double-clicking a playlist in the left sidebar – or "Favorites" – starts playback of that playlist. If shuffle is on, a random track starts; otherwise, the first track in the current sort order. A single click still only selects the playlist.

### Playlists and Files

- The same file can still be in multiple playlists. Each entry remains independent and is not "mixed" with the other.
- Duplicates with the same file name in different folders remain separate.
- CUE tracks continue to be treated as separate entries.
- On Windows, paths are recognized regardless of case and of `/` or `\`. A renamed folder therefore no longer leads to "disappeared" favorites or incorrectly mapped entries.

### Safer Handling of Your Data

- Settings, library, listening history, and playback state are now saved safely: the new file replaces the old one only after it has been written completely. A crash in the middle does not damage the existing data.
- If a file is still unreadable, the player automatically creates a backup copy (e.g. `library.invalid-…`) and rebuilds the data. The corrupted file is not silently overwritten.
- Older or incomplete files are still loaded and supplemented with sensible default values.
- Writing now happens in the background but remains atomic. On a normal exit, the player waits until all changes are on disk. Write errors are reported in the status bar.
- After the file is replaced, the directory is additionally synchronized. This ensures that even in a power failure the new file survives, rather than the old one or none at all.

### Metadata and CUE

- Cached track information is re-read only when the file has actually changed. This makes startup and browsing faster.
- For CUE files, both the CUE sheet and the associated audio file are checked: if either changes, the track information is updated correctly.
- Removed songs and playlists no longer leave stale entries in the cache.
- A corrupted cache or one created by a newer version is backed up and cleanly rebuilt instead of being misinterpreted.
- When tracks change, the title and UI appear immediately; tags, embedded cover art, and lyrics are loaded in the background. The display stays consistent even when skipping through several tracks quickly.
- Track properties opens immediately and shows "Reading tags…" until the full information is available. Changes you make in the meantime are not overwritten by the later load.

### Audio and Tags

- Under **Settings → Audio** there are now four resampler levels: Fast, Balanced, High, and Maximum. "High" corresponds exactly to the previous setting, so if you change nothing, you'll hear no difference.
- FIR correction files may now contain up to 16,384 taps per channel (previously 2,048). At 48 kHz, that's just over 340 ms.
- In Track properties, "Scan and write ReplayGain" measures the loudness according to ITU-R BS.1770-4 and writes `REPLAYGAIN_TRACK_GAIN` and `REPLAYGAIN_TRACK_PEAK` to the file. Only tags are changed; the music itself is untouched. The scan runs in the background, the UI stays responsive, and playback continues.
- Also in Track properties, you can change title, artist, album, album artist, and track number and write them back with "Save tags". Empty fields remove the old value. For CUE entries, the associated audio file is edited instead.
- External CUE files may now reference multiple audio files. The last track of a file plays to the end of that file.
- New file formats: MP1 and MP2 (MPEG Layer I/II), audio in MKA/MKV, and ADPCM in WAV and CAF. For MKV files with a video track, the first decodable audio track is selected. WavPack, Monkey's Audio, and DSD remain unsupported.
- The audio worker no longer queries and clones the next playback item via a lock on every iteration. It keeps track of the last state and reacts only to actual changes. This reduces locking and allocations in the real-time path; the sound itself is unchanged.

### Compatibility

- Existing `library.json`, `playback.json`, and listening history remain valid and are reused automatically.
- No manual steps are required; a backup of your settings is still never a bad idea.
- Keyboard shortcuts and basic operation are unchanged; the only additions are the heart per row and double-clicking playlists.
- The GitHub button is back: under **Tools** in the header and under **Settings → About**. Both open the project repository.

---

## For the Curious (Brief, Technical)

- Songs receive a stable, random identifier per playlist entry. This identifier – not the path – determines which song is meant.
- Favorites also store the identifier of the original song. The path is retained only for display and as a fallback for old data.
- Playback state and the queue use the same identifiers; index positions are derived from them.
- Saving is atomic via a temporary file; corrupted JSON files are backed up with a prefix. `sync_directory()` also flushes the directory entry after the rename: on Windows via `CreateFileW` with `FILE_FLAG_BACKUP_SEMANTICS` and `GENERIC_WRITE` (a read-only handle would cause `FlushFileBuffers` to refuse), on Unix via `sync_all()` on the open directory.
- The metadata cache is versioned and, as needed, migrated, backed up, or rebuilt.
- The resampler uses 64/128/256/512 taps depending on the level; the default remains 256 taps with cubic interpolation.
- Loudness measurement uses `bs1770` (ITU-R BS.1770-4), writes the ReplayGain 2.0 reference (-18 LUFS) and the sample peak; CUE segments are measured across the existing track boundaries.
- Tag changes are written to the original file via lofty and then updated in all playlist views via the existing metadata indexer.
- The ReplayGain scan runs in a background thread; the result is passed to the UI via a channel.
- A stalled gapless handoff switch releases the decoder after five seconds and reports a warning instead of blocking the worker indefinitely.
- `RenderState` still holds `next` in a `Mutex`, but additionally has a `next_epoch: AtomicU64`. `queue_next()` increments the counter only on a real change; `changed_next()` supplies a cloned `Next` to the pump thread only when the counter changes. The real-time path thus performs an atomic load instead of a lock plus clone on every iteration. A test verifies that identical requests do not trigger a change.
- Symphonia features `mp1`, `mp2`, `adpcm`, and `mkv` are enabled; for MKV, `audio_track()` prefers a track that can actually be decoded over the first non-empty track. `Cargo.lock` contains the two new Symphonia crates.
- The heart in the row uses the same stable track identifier as the header; `toggle_favorite_id()` therefore handles both paths.
- Playlist start picks a random index when shuffle is on and otherwise the first entry in the visible sort order (`playlist_entry()`); the same logic also handles a regular start via "Play".
- `library.json`, `playback.json`, and `listening.json` are still written atomically (temporary file + `fsync` + replace), but from a background thread. The UI thread only serializes the snapshot; multiple snapshots of the same file are coalesced. On exit, the UI waits for completion (`shutdown()`).
- Playlist import and export use worker threads with `mpsc`; only library changes and status messages happen on the UI thread.
- `Media::placeholder()` returns only the file title (including CUE resolution); the full `Media::read()` runs in `media_job` on a worker. `finish_media_job()` takes over the cover texture, lyrics, and notification of plugins/media keys, then starts cover and lyrics lookup. A generation counter discards stale results on rapid track changes. `property_jobs` does the same for the properties dialog; `loading` prevents a later worker result from overwriting user changes.
- Before/after measurements are in `docs/BENCHMARK-REPORT.md`. With 100,000 tracks, the work in the UI thread drops from around 8–20 ms to around 5 ms; the variation in disk latency is disclosed in the report.
