# Rust Player v0.7.0 – What's New

A plain-language description of the current Windows release. Your music files stay where they are, and the default audio settings are unchanged.

v0.7.0 adds opening audio files from Explorer, liquid-glass backgrounds, saved themes and wider font fallback. It also fixes playback, search, favorites, queue, history, lyrics, updater and release-package issues. The published v0.6.1 download is not changed.

**Hardware checks:** digital capture at the DAC input and listening tests are still user-side checks. They are not claimed as completed here.

---

## Short summary for the release page

- **Favorites stay with the right song** – even after reordering, renaming or moving files.
- **The last song starts again** – at the right position, even after playlist changes.
- **“Next” picks the right song** – especially after deleting the song that was playing.
- **The same file in two playlists stays separate** – duplicates do not get mixed up.
- **Damaged settings are backed up** – they are not silently overwritten.
- **Old settings still work** – `library.json` and `playback.json` are read automatically.
- **The library opens faster** – cached tag information is only refreshed when it really changed.
- **No new Favorites folder** – Favorites remain a normal playlist; no files are copied or moved.
- **More file types play** – MP1/MP2, ADPCM, MKA/MKV, Monkey's Audio, WavPack and DSD-to-PCM.
- **Filter chips and smart playlists** make large libraries easier to browse.
- **Titles and covers appear sooner** while the full details load in the background.
- **Tag and ReplayGain edits are possible** without changing the music itself.
- **Settings are written safely** in the background, with a backup if a file is unreadable.
- **Optional sound controls** – four resampler levels, a bit-perfect preset and longer correction filters. Default sound stays the same.
- **Status messages fade out** – short notices tidy themselves up; progress and audio errors stay visible.
- **The filter bar is easier to use** – **All filters** shows every value, the small bar scrolls with the mouse wheel, and clicking an active chip removes just that filter.
- **Open from Windows** – double-click a song in Explorer to open it in the player and start playing; no administrator rights are needed.
- **Liquid glass** – a slow sheen and glass highlight can be added to wallpapers and cover art; your image files are not modified.
- **Themes stay saved** – a theme created with **Save as new** remains in the list, and **Apply** / **Cancel** stay visible at the top of Appearance.
- **More scripts display correctly** – Japanese, Chinese, Cyrillic, Greek, Arabic, Hindi and other titles use Windows font fallbacks instead of empty boxes.
- **Phrase search** – quoted text searches the phrase, and an unfinished `missing:` query no longer hides every song.
- **Playback fixes** – Monkey's Audio volume and channels, ReplayGain for APE/WavPack/DSD, longer FIR filters, gapless seeking and errors, and audio-device loss.
- **The download is complete** – the Windows ZIP includes the updater and license notices; an updater asset and checksum are published for GitHub releases.

---

## What you will notice

### 1. Finding songs is easier

Above the track list you can click chips for **Artist**, **Album**, **Genre** or **Year**. Click again to remove the filter. **All filters** shows the full list instead of only the most common values, and the small bar scrolls sideways with the mouse wheel.

The search box understands plain words and field names such as `artist:`, `album:`, `title:`, `albumartist:`, `genre:`, `year:`, `path:` and `file:`. Quoted phrases, `AND`, `OR`, `NOT` and `-` exclusion work too. **Smart playlists** save a search so you can refresh it later.

### 2. Favorites and playlists behave like a real library

- A heart in every row sets or removes a favorite directly in the list.
- Double-click a playlist or **Favorites** to start it. Shuffle starts a random song; otherwise the first song in the visible order plays.
- Drag rows to reorder “Date added” playlists, select several rows for bulk actions, and undo or redo edits.
- Favorites follow the song when it is renamed or moved. If the original song is removed, the stale favorite disappears.
- The same file can live in several playlists; each entry stays independent.
- M3U/M3U8/PLS can be imported. M3U export can use relative paths, so a playlist can be moved together with the music folder.

### 3. Playback and the “Play next” list are more reliable

- The last song and its position are restored on restart.
- Explicitly queued songs are saved in `queue.json` and restored; entries whose files no longer exist are dropped.
- After deleting the currently playing song, **Next** goes to the actually next song.
- Compatible songs can join without a gap. If a handoff gets stuck, the player gives up after five seconds and shows a warning instead of waiting forever.
- The sound path itself was not changed for these fixes.

### 4. More of your music plays

New support includes **MP1/MP2**, **ADPCM in WAV and CAF**, audio in **MKA/MKV**, **Monkey's Audio (`.ape`)**, **WavPack (`.wv`)** and **DSD in DSF/DFF**. External CUE sheets with one or several audio files and CUE text inside FLAC files are split into separate tracks.

DSF/DFF are converted to high-rate PCM (DSD64 → 88.2 kHz) so ordinary audio devices can play them. This is not native DSD or DoP. WavPack hybrid files use only the `.wv` part; the optional `.wvc` correction file is not read. DSD inside WavPack, SACD-ISO, ASIO, VST and UPnP/DLNA are still not included.

### 5. Titles, covers and tags arrive quickly

The title appears at once; full tags, embedded cover art and lyrics follow in the background. Rapid skipping no longer leaves details from an older song on screen. **Track properties** opens immediately and fills in later without overwriting anything you typed.

You can edit title, artist, album, album artist and track number in Track properties. With several songs selected, **Edit tags…** can keep, set or clear those fields plus genre and year. **Scan and write ReplayGain** only writes loudness tags; the music is untouched. **Save next to track** writes `cover.png` beside the song without replacing an existing cover.

### 6. Your data is better protected

- Files are written through a temporary file and only replace the old one when complete.
- Saving happens in the background; on a normal exit the player waits for all writes.
- The folder is flushed after saving so a power failure does not bring back the old file.
- Unreadable files are backed up with names such as `library.invalid-…` and rebuilt. Old backups are never deleted automatically.
- A missing `queue.json` on first start is normal, not corruption.
- Updates are streamed to disk and checked while downloading, so the whole installer is not held in memory.

### 7. Sound options, only if you want them

**Settings → Audio** offers four resampler levels. **High** is exactly the previous quality, so doing nothing keeps the old sound. **Apply bit-perfect preset** requests WASAPI Exclusive with the original format and turns off conversion, DSP, ReplayGain, software volume and fades. FIR correction files can be twice as long.

The bit-perfect indicator describes the software signal path up to the Windows driver input. Proving the hardware path still needs a digital capture on your own device; see [hardware verification](HARDWARE-VERIFICATION.md). A listening test of the audio handoff is also part of the manual checklist.

### 8. Small conveniences

- Global shortcuts: `Space`, `N`, `P`, `S`, `M`, `F`, `Ctrl`+`S`, `Ctrl`+`←`/`→`, `Ctrl`+`Z`, `Ctrl`+`Shift`+`Z` / `Ctrl`+`Y`.
- Shortcuts are ignored while typing in a text field.
- Short notices fade out after a few seconds; progress and active audio errors stay visible.
- **GitHub** is available under **Tools** and **Settings → About**.
- With Last.fm connected, the now-playing panel offers **♥ Last.fm** and **♡ Last.fm**.
- [Manual tests](TESTING.md) and [audio tests](AUDIO-TESTING.md) are documented for anyone who wants to verify the player on their own hardware.

---

## Before you update

- Existing `library.json`, `playback.json` and listening history are reused. A backup of your settings is still a good idea.
- No manual migration step is needed.
- No music files are moved, renamed or deleted.
- Existing shortcuts and basic operation stay the same; the listed features are additions.

## If a word is unfamiliar

| Word | Meaning here |
| --- | --- |
| **Facet / chip** | A small button such as `Artist: Pink Floyd` that filters the list with one click. |
| **Smart playlist** | A playlist that remembers a search instead of a fixed list of songs. |
| **CUE sheet** | A small text file that says where each track starts inside one longer audio file. |
| **ReplayGain** | A stored loudness value so songs play at a similar volume. Writing it changes tags, not the music. |
| **Gapless** | Two compatible songs join without a short silence. |
| **Bit-perfect** | The original samples reach the audio driver unchanged. It does not prove what the DAC or speakers do. |
| **WASAPI Exclusive** | Windows audio mode that asks for the song's original format instead of mixing it with other apps. |
| **M3U / M3U8 / PLS** | Small playlist files that point to your music files. |
| **WavPack / `.wv`** | A lossless audio format. Hybrid files can store extra correction data in `.wvc`, which this build does not use. |
| **DSD / DSF / DFF** | A high-rate 1-bit audio format. This player converts it to PCM; it does not send native DSD to the device. |

## Please check after updating

1. Two compatible songs in a row: is the transition free of clicks?
2. Skip forward and backward during playback: are positions correct?
3. Load a `.mp1`/`.mp2`, an ADPCM WAV and an `.mka` file.
4. Open a FLAC with an embedded CUE sheet: are titles and starts correct?
5. Load a `.ape` file and seek forward and backward.
6. Load a `.dsf` or `.dff` file: check the ID3 title, duration and seeking; listen for clicks at track ends.

## Installation

Download the Windows x64 ZIP, extract it and start `Rust-Player.exe`. There is no installer. A `.sha256` file is supplied so you can check the download. Keep the updater helper next to the player.

## Older notes

[v0.5.0](RELEASE-NOTES-0.5.0.md) · [v0.4.0](RELEASE-NOTES-0.4.0.md) · [early draft](RELEASE-NOTES-DRAFT.md)
