# Rust Player v0.6.1 – What's New

This stable patch release fixes the facet quick filters from **v0.6.0**. Your files stay where they are and the audio path is unchanged.

**Facet filter fixes:** **All filters** next to the facet bar expands every cached Artist, Album, Genre and Year value in a scrollable, wrapped panel. The compact bar scrolls left and right with the mouse wheel. Clicking an active chip again removes that filter, even when two filters are active or a value contains spaces. The published `v0.6.0` release is left unchanged.

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
- **Bit-perfect preset** – one click switches to WASAPI Exclusive and bypasses conversion, DSP, ReplayGain, fades, and software volume.
- **Adjust resampling quality** – four resampler quality levels instead of a fixed setting; the previous sound quality remains the default.
- **Longer FIR filters supported** – now 16,384 instead of 2,048 taps per channel.
- **Write ReplayGain yourself** – one click in Track properties measures the file and saves the tags.
- **Edit tags directly** – title, artist, album, album artist, and track number can be changed.
- **CUE with multiple files** – albums with one audio file per track are now supported as well.
- **CUE inside FLAC** – a sheet stored in the audio file itself is now read and split into tracks.
- **Heart icon next to each song** – favorites can now be set and removed line by line in any playlist.
- **Start a playlist with a double-click** – double-clicking a playlist plays it; with shuffle enabled, a random track starts.
- **The UI stays responsive while saving** – settings are written in the background; on exit, everything is fully written.
- **Smooth import and export** – playlist files are read and written in the background.
- **Portable playlists** – M3U export can now write paths relative to the playlist file.
- **The queue survives a restart** – explicitly queued tracks are stored and validated on load.
- **Playlist editing like a real library** – drag to reorder, select several rows for bulk actions, and undo or redo edits.
- **Album art can be saved locally** – one click writes `cover.png` beside the audio file without replacing an existing cover.
- **Last.fm Love/Unlove** – the current track can be loved or unloved from the now-playing panel.
- **Global keyboard shortcuts** – transport, favorite, shuffle, undo, and redo work without a focused widget.
- **Monkey's Audio (.ape)** – lossless APE files play directly with seeking and CUE offsets.
- **DSF and DFF (DSD)** – DSD is converted to high-rate PCM for ordinary audio devices; ID3 titles and durations are read from the container.
- **WavPack (`.wv`)** – integer, float and hybrid files decode with exact seeking and APEv2 tags; DSD inside WavPack is not supported.
- **Smart playlists** – save a search query as a playlist and refresh it from the library at any time.
- **Facet filters** – click Artist, Album, Genre or Year chips above the track list to filter by metadata.
- **Edit tags for a whole selection** – set or clear title, artist, album, album artist, genre and year for many files at once.
- **Status messages fade out** – one-line notices such as “Audio output changed. Playback paused.” clear themselves after a few seconds. Progress reports and active audio errors stay visible until they finish or recover.

---

## Detailed Version (for End Users)

### Favorites

Favorites are now firmly linked to the respective song – not just to its file path.

What you'll notice:

- You can reorder, rename, or move songs on disk within a playlist: the favorite star stays with the right song.
- When you add a song from another playlist, the link to the original is preserved. If the original song is removed, the stale favorite disappears too – instead of remaining as a "ghost".
- If a song is removed or moved directly from favorites, the mapping remains correct. Removing a song from a source playlist also refreshes the favorites view immediately; a shortened list can no longer cause a crash while scrolling.
- Playlist search now understands fields (`artist:`, `album:`, `title:`, `albumartist:`, `genre:`, `year:`, `path:`, `file:`, `missing:`), quoted phrases, `AND`, `OR`, `NOT` and `-` exclusion. Rows without indexed metadata remain searchable by path.
- The favorite star in the header shows the correct state even when you're playing directly from the favorites playlist.
- Every song row now has its own heart between the number and the title. Clicking it sets or removes the favorite – even directly in the favorites playlist. A filled heart is red; an empty one is grayed out.

Important: **no** "Favorites" folder is created. Favorites remain a normal playlist in `library.json`. Your music files are neither copied nor moved.

### Playback and Queue

- On startup, the player remembers the last played song and the position. This now works reliably even if playlists have changed in the meantime.
- Old saved playback data without the new mapping is automatically migrated and updated on first load.
- If the currently playing song is deleted from a playlist, "Next" jumps to the actually next track – no longer accidentally to the previous one.
- The queue ("Up Next") remains consistent even after songs are removed or moved.
- Tracks explicitly added with "Add to Play Queue" are stored in `queue.json` and restored on the next start. Entries whose tracks no longer exist are discarded; the playlist-derived part of the queue is still rebuilt from the current library.
- Double-clicking a playlist in the left sidebar – or "Favorites" – starts playback of that playlist. If shuffle is on, a random track starts; otherwise, the first track in the current sort order. A single click still only selects the playlist.

### Playlists and Files

- The same file can still be in multiple playlists. Each entry remains independent and is not "mixed" with the other.
- Duplicates with the same file name in different folders remain separate.
- CUE tracks continue to be treated as separate entries.
- On Windows, paths are recognized regardless of case and of `/` or `\`. A renamed folder therefore no longer leads to "disappeared" favorites or incorrectly mapped entries.
- The playlist context menu now offers **Export (absolute paths)** and **Export (relative paths)**. Relative M3U files can be moved together with the music folder. CUE virtual tracks are exported as their sheet so a re-import expands the whole disc.
- **Smart** in the sidebar creates a playlist from a saved query; **Edit query**, **Refresh** and **Make normal** are available in its context menu. A smart playlist is materialized from the current library, so sorting, playback order, favorites and undo/redo keep working with stable track IDs. Refreshing does not remove or rewrite files.
- A facet bar above the track list shows the most frequent Artist, Album, Genre and Year values. Clicking a chip adds the matching field filter to the playlist search; clicking it again removes only that filter. Genre and Year are indexed from file tags on import, so old caches are rescanned once.

### Playlist Editing

- In "Date added" order, rows can be dragged to a new position. The order is saved with the playlist; other sort modes stay read-only because their order is derived from metadata.
- Click selects a row, `Ctrl`/`Cmd`+click toggles a row, and `Shift`+click selects a range. The context menu then applies "Add to Play Queue", "Toggle favorite", and "Remove from playlist" to every selected row.
- `Ctrl`+`Z` undoes a track edit; `Ctrl`+`Shift`+`Z` or `Ctrl`+`Y` redoes it. Undo and redo are also available under **Tools**. History is cleared when playlists themselves are added, deleted, or moved, so snapshots never point at the wrong playlist.

### Keyboard Shortcuts

- `Space` play/pause, `N` next, `P` previous, `S` stop, `M` mute, `F` favorite, `Ctrl`+`S` shuffle.
- `Ctrl`/`Cmd`+`←`/`→` previous/next, `Ctrl`+`Z` undo, `Ctrl`+`Shift`+`Z` / `Ctrl`+`Y` redo.
- Shortcuts are ignored while a text field or another widget has keyboard focus, so typing in the search box never controls playback.

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
- **Apply bit-perfect preset** switches to WASAPI Exclusive and turns off format conversion, Shared fallback, software volume, transition fades, ReplayGain, downmix, and all DSP. If the device cannot accept the source format, playback fails instead of silently processing it. The signal-path table shows every remaining stage. True hardware bit-perfect status still requires a digital capture at the DAC input; the new [hardware verification workflow](HARDWARE-VERIFICATION.md) documents what Shared loopback, the iD4 mk2 virtual loopback and analogue captures can and cannot prove.
- FIR correction files may now contain up to 16,384 taps per channel (previously 2,048). At 48 kHz, that's just over 340 ms.
- In Track properties, "Scan and write ReplayGain" measures the loudness according to ITU-R BS.1770-4 and writes `REPLAYGAIN_TRACK_GAIN` and `REPLAYGAIN_TRACK_PEAK` to the file. Only tags are changed; the music itself is untouched. The scan runs in the background, the UI stays responsive, and playback continues.
- Also in Track properties, you can change title, artist, album, album artist, and track number and write them back with "Save tags". Empty text fields remove the old value; the track number must be a positive number. CUE entries are virtual tracks and cannot be edited from the properties dialog; edit their source audio file instead.
- External CUE files may now reference multiple audio files. The last track of a file plays to the end of that file.
- A CUE sheet embedded in a FLAC file is now recognized too. Its tracks appear as separate entries, with the titles and start positions from the sheet. Nothing is written to the audio file. A damaged comment block no longer looks like "no sheet"; the reason is written to the log folder.
- New file formats: MP1 and MP2 (MPEG Layer I/II), audio in MKA/MKV, ADPCM in WAV and CAF, Monkey's Audio (`.ape`), WavPack (`.wv`), and DSD in DSF/DFF containers. For MKV files with a video track, the first decodable audio track is selected.
- DSD files are not native-DSD playback: the 1-bit stream is converted to 32/64-bit float PCM at up to 96 kHz (DSD64 → 88.2 kHz) with a linear-phase FIR cascade. This makes DSF/DFF playable on normal PCM devices; it cannot be bit-perfect DSD and DoP is not offered. The iD4 mk2 only accepts up to 96 kHz anyway, so DSD64-DoP (176.4 kHz) is not an option on that hardware.
- WavPack files now decode through libwavpack. Integer and normalized float streams, exact seeking, CUE offsets and APEv2 title/artist/album/genre/year/track fields are supported. Hybrid files play from the `.wv` stream alone; the optional `.wvc` correction file is not read, so hybrid files that need it decode as lossy. DSD stored in a WavPack container is not supported. Source builds now need CMake and LLVM/libclang; `.cargo/config.toml` pins the CMake policy level for the bundled sources.
- With a multi-row selection, **Edit tags…** opens a bulk dialog. Each field is independently Keep, Set or Clear; genre and year are supported too. Writing happens on a worker thread, CUE virtual tracks are skipped, and the metadata index is refreshed afterwards. Existing values in fields left on Keep are untouched.
- The audio worker no longer queries and clones the next playback item via a lock on every iteration. It keeps track of the last state and reacts only to actual changes. This reduces locking and allocations in the real-time path; the sound itself is unchanged.
- Double-click the cover in the now-playing panel to open the artwork viewer. **Save next to track** writes `cover.png` in the audio file's folder; an existing `cover.png` is never overwritten.
- When connected to Last.fm and sharing is enabled, the now-playing panel offers **♥ Last.fm** and **♡ Last.fm** to love or unlove the current song. A late click is ignored if the track changed in the meantime.

### Compatibility

- Existing `library.json`, `playback.json`, and listening history remain valid and are reused automatically.
- No manual steps are required; a backup of your settings is still never a bad idea.
- Existing keyboard shortcuts and basic operation are unchanged; additions include the heart per row, double-clicking playlists, the shortcuts listed above, and the new editing actions.
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
- `library.json`, `playback.json`, and `listening.json` are still written atomically (temporary file + `fsync` + replace), but from a background thread. The UI thread only serializes the snapshot; multiple snapshots of the same file are coalesced through an index. On exit, the UI waits for completion (`finish()`), which also reports whether the worker survived.
- Playlist import and export use worker threads with `mpsc`; only library changes and status messages happen on the UI thread.
- `Media::placeholder()` returns only the file title (including CUE resolution); the full `Media::read()` runs in `media_job` on a worker. `finish_media_job()` takes over the cover texture, lyrics, and notification of plugins/media keys, then starts cover and lyrics lookup. A generation counter discards stale results on rapid track changes. `property_jobs` does the same for the properties dialog; `loading` prevents a later worker result from overwriting user changes.
- Before/after measurements are in `docs/BENCHMARK-REPORT.md`. With 100,000 tracks, the work in the UI thread drops from around 9–15 ms to around 5 ms; the variation in disk latency is disclosed in the report.
- `scripts/run-benchmarks.ps1` runs the four ignored benchmarks in one step and archives environment, logs, raw CSV and process metadata (wall/CPU time, page faults, peak working set) under `target/benchmark-runs/<timestamp>/`. UI frame timing is opt-in via `RUST_PLAYER_FRAME_LOG=<csv path>`; without the variable there is no file I/O per frame. `scripts/run-frame-log.ps1` runs a complete session and reports p50/p95/p99/max, but refuses to stop an already running player.
- Embedded CUE: `src/cue.rs` reads the FLAC Vorbis-comment block directly because lofty's generic `Tag` does not expose the `CUESHEET` key. Tracks are addressed as virtual `audio.flac/<n>` paths and share the same resolver, cache stamps, tag redirect and decoder offsets as external sheets. A malformed or truncated block returns an error that is appended to the log under `%LOCALAPPDATA%\Rust Player\logs`; "no Vorbis block at all" remains a normal, silent case.
- Favorites and source lookups use two derived, non-serialized indexes (`path_sources`, `favorite_paths`), rebuilt together with `locations`. Case-insensitive path comparison on Windows is centralized in `path_key()`.
- An end-to-end test drives `SaveWorker` plus `BackupTracker` against real files: a corrupt file is backed up, the replacement is written atomically, and only the confirmed write retires the backup.
- The updater streams the installer into a temporary file while computing SHA-256 in 64 KiB chunks (Windows CNG). Both the download worker and the updater binary share the same `digest::copy_verified()` implementation, so verification cannot drift; the whole archive is never held in memory.
- M3U relative export uses a lexical path helper; no filesystem access is needed for the playlist itself. Different Windows drives or UNC shares fall back to absolute paths. Import normalizes paths lexically before checking supported extensions.
- `PlayQueue` is serialized with stable row IDs. `load()` repairs `next_id` from the largest stored ID and prunes tracks absent from `library.locations`; every queue-changing action enqueues a save through `SaveWorker`. A missing `queue.json` is a normal first start and does not create a backup; only an unreadable or invalid file does.
- Artwork sidecars are PNG bytes written to a temporary file and committed with `persist_noclobber()`. The cover viewer keeps the source track path only for the currently displayed artwork.
- Last.fm Love/Unlove reuses the signed POST path of the existing scrobble client (`track.love` / `track.unlove`). Generation and track ID are checked by the worker so rapid track changes cannot cross-wire a request.
- Playlist undo stores a bounded (64 entries) snapshot of tracks, IDs, metadata, sort, and Favorites IDs. Redo snapshots are taken before applying an undo. Track drag/drop uses remove + insert with the drop target corrected for the still-present source row; metadata, Favorites, locations, and playback identity are refreshed from the same snapshot.
- APE uses `ape-decoder` (MIT/Apache-2.0) with no unsafe code; blocks are converted from the file's sample width to the output sample type, and CUE start offsets are applied by the shared `Source` seek path.
- Smart playlists store a query in `Playlist.smart_query` (`#[serde(default)]`, so old `library.json` files stay valid). Refresh computes matching paths from the metadata index and writes them into the playlist as ordinary rows: existing `TrackId`s, order, favorites, undo/redo and M3U export keep working. An empty query matches the whole library.
- Facets are derived from indexed `TrackInfo` (`genre`, `year` added); the metadata cache is now version 3 and discards version-2 entries once so new fields are populated. Query additions are `genre:` and `year:`; the chip bar caches per playlist and is invalidated together with search results.
- Bulk tagging uses a separate `tagging::BatchEdit` where each field is `None` (keep), `Some("")` (clear) or a value (set). `write_batch()` writes only selected `ItemKey`s; year goes to `RecordingDate`/TDRC so it round-trips with the reader. The worker reports progress and errors, and `refresh_metadata()` updates views and indexes when all files are done.
- DSD uses `dsd-reader` (MIT/Apache-2.0) to normalise DSF (planar, LSB-first) and DFF (interleaved, MSB-first) to MSB-first interleaved bytes. `dsd-pcm` (MIT/Apache-2.0) decimates them with a linear-phase FIR cascade to 88.2/96 kHz floats; group delay is compensated and `finish()` flushes the tail. Seeking rebuilds the container reader and resets filter state because the decimator has no random-access API. ID3 tags come from `dsd-reader`; lofty cannot probe DSF/DFF headers. DSD stays a PCM conversion, not native DSD/DoP.
