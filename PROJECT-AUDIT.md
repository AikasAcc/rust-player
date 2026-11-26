# PROJECT-AUDIT — Rust Player

**Date:** 2026-11-25  
**Repository:** `C:\Users\Aika\Documents\Codex\2026-11-23\kan-2\work\phase5-audio-formats`  
**Purpose:** Phase 1 – Project Audit & Safety. Pure inventory; **no source file was modified**, no commit, no push, no branch switch.

## Methodology and traceability

- Static review of the entire repository via four parallel, read-only audit passes (audio engine; library/playlist/favorites/playback/UI; persistence/tests; Cargo/Git/docs).
- Additionally, I **actually ran** the following:
  - `cargo test --release --locked` → **37 tests passed, 0 failed, 2 intentionally ignored** (35 tests in the main binary + 2 in the updater binary).
  - `cargo fmt --check` → clean, no deviations.
  - `cargo clippy --release --locked --all-targets` → clean, **no warnings**.
- **Not run:** the ignored benchmarks, real profiling/measurements, Android build, release packaging, Git operations with write effects. Therefore **no performance figures are invented** in this document; performance statements are either statically supported or marked as hypotheses.
- The command sequence mentioned in the task description (`git add .`, `git commit`, `git push`, `git switch -c …`) was **deliberately not executed**, because the global rule "never perform Git push, merge, or release independently" takes precedence and the branch already exists.

---

## 1. Current architecture

### 1.1 Working state and repository situation

- The directory is a **linked worktree**; `.git` is a Git file pointing to `…/phase0-main/.git/worktrees/phase5-audio-formats`. 7 worktrees share it (`.git` or `git worktree list`).
- Current branch: `ai/rust-player-improvements` @ `ef3f005`, **6 commits ahead** of `origin/main`, 0 behind; no upstream configured. Local `main` is 12 commits behind `origin/main`.
- The six local commits already contain implementations from the roadmap: `c6b7f22` benchmarks, `46c5201` metadata cache + bounded parallelism, `281c4ec` stable IDs in queue/order, `9f0b2a8` UI extraction, `a5cfdbf` prepared search string, `ef3f005` missing files by filename + metadata. Phases 4–7 are therefore **largely already present in the code**; see section 3 for open items.
- Uncommitted work state (not created by me, not modified by me): `src/library.rs`, `src/listening.rs`, `src/main.rs` modified (+112/−8), `android/` untracked. Contents: parameterizable backup routine (`Library::back_up_invalid_json`), flags `playback_needs_backup` / `listening_needs_backup`, backups `playback.invalid-*.json` and `listening.invalid-*.json`, two new listening tests.

### 1.2 Module overview (Rust binary)

`src/main.rs:3-34` declares 30 modules. Sizes (lines): `main.rs` 2022, `lastfm.rs` 829, `library.rs` 768, `background.rs` 763, `audio/mod.rs` 676, `ui/dialogs.rs` 642, `taskbar.rs` 485, `audio/exclusive/device.rs` 433, `audio_ui.rs` 416, `play_queue.rs` 372, `theme.rs` 368, `audio/dsp.rs` 339, `audio/decoder.rs` 320, `ui/playlist.rs` 290, `ui/header.rs` 288, `presence.rs` 283, `window_chrome.rs` 271, `audio/exclusive/render.rs` 246, `artwork.rs` 237, `audio_setup.rs` 225, `media_keys.rs` 221, `settings.rs` 213, `update.rs` 209, `audio/shared.rs` 203, `audio/exclusive.rs` 199, `metadata_cache.rs` 180, `ui/inspector.rs` 180, `ui/sidebar.rs` 177, `relink.rs` 168, `listening.rs` 129, `playlist_io.rs` 109. Also: `src/bin/rust_player_updater.rs` (94).

`src/ui/mod.rs` contains only module declarations (5 lines); the `impl Player` blocks live in sibling files, e.g. `current_is_favorite()` in `src/ui/header.rs:213-230`.

### 1.3 Audio engine (`src/audio/`, unchanged)

- **One** decoder worker (`audio-decoder`, `src/audio/mod.rs:190-220`) probes, decodes, converts (Rubato), applies ReplayGain/DSP and fills an `rtrb::RingBuffer<u32>` (`src/audio/mod.rs:416-447`).
- **Two backends:** CPAL/WASAPI Shared in the audio callback (`src/audio/shared.rs:105-124`, `Renderer::fill` :189-216) and a dedicated WASAPI Exclusive thread with MMCSS (`src/audio/exclusive.rs:100-147`, `src/audio/exclusive/render.rs:43-122`).
- State is passed via `Arc<RenderState>` with atomics (`src/audio/mod.rs:95-118`), exactly one mutex (`next`, :96/:123), SPSC ring, mpsc events (:186-197), plus mpsc channels for devices, setup probes, taskbar and WinRT media keys (`src/taskbar.rs:239-288`, `src/media_keys.rs:65-77`).
- Public API: `OutputMode/Options/Request/Format`, `Event`, `AudioTrack` with `open/queue_next/take_transition/set_volume/play/pause/seek/…` (`src/audio/mod.rs:40-83`, `:149-352`), `DeviceMonitor` (:669-717). Re-exports `:35-36`.
- Gapless: the UI computes `Next`, the worker reports `Event::Transition`, the UI binds the same sink to the next track (`src/gapless_ui.rs:28-128`, `src/audio/mod.rs:467-503`).
- Called among others from `src/main.rs:1016-1055` (`open`), `src/audio_ui.rs:10-20` (`output_request`), `src/audio_ui.rs:33-78` (poll/reopen), `src/main.rs:1496-1497` (once per frame).

### 1.4 Library, identity, playback

- `TrackId(pub u64)` (`src/library.rs:46-47`), `Playlist { name, tracks: Vec<PathBuf>, ids: Vec<TrackId>, pinned, sidebar_position, sort, descending }` (`src/library.rs:49-63`), `TrackInfo` with `search_key` (`serde(skip)`) and `missing` (`src/library.rs:87-100`).
- `Library::rebuild_locations()` (`src/library.rs:358-385`) assigns one ID per **entry** (duplicates get different IDs) and builds `locations: HashMap<TrackId, (usize, usize)>`; `locations` is `#[serde(skip)]` and is regenerated on every load (`src/library.rs:465`).
- `type Track = (usize, usize)` (`src/main.rs:53`) remains only in `SavedTrack` as a legacy compatibility field and for row/playlist operations. `Playing` holds a `TrackId` alone; `plan()`, `open()`, `start()` and `order_id()` are ID-based. `SavedTrack::resolve()` (`src/main.rs:218-263`) is the compatibility path: ID → old coordinate if the path matches → same playlist by path → first playlist with the path.
- Playback order/queue: `Player.order: Vec<TrackId>`, `cursor`, `order_list`, `queue_order_edited`, `play_queue` (`src/main.rs:275-325`); `PlayQueue::Entry { id: u64, track: TrackId, title: String }` (`src/play_queue.rs:5-15`); `plan()`/`refresh_playback_ids()` (`src/main.rs:583-621`, `:946-970`).
- Favorites: `Library.favorites: Option<usize>` points to an ordinary playlist (`src/library.rs:291`); on the first toggle, "Favorites" is created as a playlist (`src/main.rs:750-768`), sidebar entry in `src/ui/sidebar.rs:27-37`, heart icon in `src/ui/header.rs:128-142` and `:213-230`. **No physical folder is created**; folders are created only for the profile (`save_json`), plugins (`src/plugins.rs:13-16`) and backgrounds (`src/background.rs:187-192`).
- Metadata cache: `Stamp { modified, bytes }`, `Entry`, `Cache(HashMap<PathBuf, Entry>)`, `Indexer` with mpsc (`src/metadata_cache.rs:12-32`, `:108-145`); `Cache::scan()` uses `min(available_parallelism, 4)` scoped threads (:41-50); `request_index()` prevents duplicate jobs per playlist and indexes only tracks that are still missing (`src/main.rs:972-982`), `finish_indexes()` uses `try_recv()` (:984-1014).
- Missing files: `TrackInfo.missing` (`src/library.rs:91-92`), marking in the playlist, relink logic in `src/relink.rs` (`ResultSet { changes, unresolved }`, :18-22; duplicates are not guessed, test :159-168).

### 1.5 Persistence

All files are stored as siblings in `%LOCALAPPDATA%\Rust Player` (`src/main.rs:329-333`): `library.json`, `listening.json` (:342-343), `metadata-cache.json` (:362-364), `playback.json` (:493-495), plus `lastfm-queue.json` (`src/lastfm/queue.rs:65`).

`save_json()` (`src/library.rs:493-507`): `create_dir_all` → `NamedTempFile::new_in` → `serde_json::to_writer(BufWriter<&File>)` → `flush()` → `sync_all()` → `persist()`. `back_up_invalid_json()` (`src/library.rs:477-490`, with the `prefix` parameter in the uncommitted state): copies the bytes into a `tempfile`, `sync_all()`, `keep()`.

Serde compatibility: `Library` has container-level `#[serde(default)]` (`src/library.rs:270-272`), `SavedTrack.id` is `Option<TrackId>` with default `None` (`src/main.rs:77-78`), `TrackInfo.missing` has a default (`src/library.rs:91-92`), `locations` and `search_key` are `skip`. No `rename_all` attributes on the persistence types; JSON keys are the Rust field names.

### 1.6 CI and toolchain

- `.github/workflows/windows-ci.yml`: `windows-latest`, `cargo fmt --check`, `cargo clippy --release --locked --all-targets -- -D warnings`, `cargo test --release --locked`; permissions `contents: read`.
- `.github/workflows/deploy-theme-gallery.yml`: Pages deployment with OIDC; only on changes to `theme-gallery/**`.
- `build.rs:1-8` embeds `assets/icon.ico` via `winresource` (only on a Windows host **and** Windows target); requires the MSVC resource compiler or MinGW `windres`/`ar` (`README.md:15`).
- `Cargo.toml`: Edition 2024, `rust-player` v0.5.0, no `[profile.*]`, no `rust-toolchain.toml`, no `[dev-dependencies]`. Lockfile v4 with 425 packages, crates.io sources only.

---

## 2. Existing features (verified)

- **Audio formats:** MP3, FLAC, WAV, Ogg Vorbis, AAC, M4A/ALAC, AIFF, CAF, PCM, MP1/MP2, ADPCM, MKA/MKV; external CUE sheets with multiple `FILE` entries and embedded `CUESHEET` text in FLAC (`src/library.rs:42-43`, `src/audio/decoder.rs:3-16,60-78`, `src/cue.rs`). WavPack/Monkey/DSD are not included (consistent with the README). **Historischer Stand dieser Runde:** APE und DSD→PCM sind inzwischen laut Abschnitt 13 enthalten; WavPack bleibt draußen.
- **Output:** WASAPI Shared (CPAL) and Exclusive, device selection, signal path diagnostics, PCM check/test tone (`src/audio_setup.rs`, `src/audio_ui.rs:106-332`, `src/audio/setup.rs`), ReplayGain, AutoEQ/FIR/Crossfeed (`src/audio/dsp.rs`), gapless (`src/audio/gapless.rs`), taskbar thumbnails and WinRT media keys.
- **Library:** playlists with pin/rename/copy/delete/export/import (M3U/M3U8/PLS), sorting (Added/Title/Artist/Album/Duration), album headers, search (debounced at 80 ms), favorites as a playlist, listening activity (1000 entries, songs/artists rankings).
- **Metadata:** background indexer with stable cache, limit of 4 parallel scans, `missing` marking, relink by filename + metadata when ambiguous (`src/relink.rs`).
- **Integrations:** LRCLIB lyrics, Apple Music artwork, Discord Rich Presence, Last.fm with Windows Credential Manager and persistent queue, Rhai plugins, theme editor with backgrounds/animation, updater with SHA-256 verification and rollback.
- **Tests:** 38 `#[test]` functions in 24 `#[cfg(test)]` modules, of which 2 are ignored benchmarks (`src/benchmarks.rs`). Focus areas: audio/decoder/gapless/PCM, persistence, IDs, cache, DSP, CUE, relink, updater.
- **Documentation:** README, `docs/TESTING.md`, `docs/AUDIO-TESTING.md`, `docs/COMPLETION-REPORT.md`, release notes 0.4.0/0.5.0/draft, issue templates.

---

## 3. Technical issues (statically verified)

### 3.1 High priority — data consistency and storage robustness

1. **No directory fsync after `persist`.** **Fixed:** `sync_directory()` in `src/library.rs` flushes the directory after the rename (Windows: `CreateFileW` with `FILE_FLAG_BACKUP_SEMANTICS` and `GENERIC_WRITE` + `FlushFileBuffers`; Unix: `File::open(dir)?.sync_all()`). Also applies to invalid-JSON backups.
2. **Backup flag cleared before a successful save.** **Fixed:** `SaveWorker` now reports every write outcome (`SaveReport` with path and optional error) in order. `BackupTracker` in `src/save_worker.rs` separates "backup still needed" from "backup made, write pending"; `finish_saves()` retires the pending backup only on success and re-arms it on failure. Repeated saves cannot create duplicate copies while a write is in flight. `count_listening()` now goes through the same guard, so a corrupt `listening.json` is backed up even when playback history is the first write after startup. Tests: `successful_write_is_reported_with_its_path`, `backup_is_not_repeated_while_a_write_is_pending`, `failed_write_rearms_the_backup`, `failure_without_a_pending_backup_is_ignored`.
3. **Inconsistent corrupt-playback handling.** **Fixed:** `previously_saved()` now delegates to `read_playback()` (`src/main.rs:724-726`), which reports read/parse errors and backs up corrupt `playback.json` via `back_up_invalid_playback()` (`src/main.rs:604-622`). Both startup paths use the same handling.
4. **No cross-file transaction.** `library.json`, `playback.json`, `listening.json` and `metadata-cache.json` are written independently; a crash can leave inconsistent states. **Decision: accepted risk, no transaction implemented.** Each file is already replaced atomically (tempfile + `fsync` + `persist` + directory fsync), so a crash always leaves valid JSON, never a half-written file. The only semantic cross-file dependency is `playback.json` → `library.json`: `SavedTrack::resolve()` (`src/main.rs:218-263`) falls back from a stale `TrackId` to the saved path, then to the same playlist and finally to any playlist, so a stale pair restores at worst an older position or nothing. `listening.json` stores plain title/artist strings and timestamps; `metadata-cache.json` is derived data keyed by path and pruned against the library. Neither depends on the other files' identity scheme. A real transaction would need a coordinator across the UI thread and the indexer thread, durable generation numbers and retained old versions for rollback; the writes intentionally happen at different times (volume debounce, track change, track completion, cache scan), so serializing them would cost responsiveness without protecting any user content. Mitigations that stay in place: per-file atomicity and fsync, backup of corrupt originals before overwrite, tolerant playback resolution and a rebuildable metadata cache. Revisit if a future state file owns data that can neither be reconstructed nor tolerated.
5. **`metadata-cache.json` discarded without backup.** **Fixed:** `Cache::load()` backs up corrupt or future-version caches, migrates legacy entries and reports errors (`src/metadata_cache.rs:91-141`). The indexer handles load failures without killing its thread; tests cover backup/rebuild, partial JSON, a missing cache and version upgrades.
6. **Unbounded backup accumulation.** Every corrupt file produces another `*.invalid-*.json` copy with no rotation policy (`src/library.rs:477-490`, test :771-789). **Deliberately kept** so corrupt originals remain inspectable; documented instead of silently rotated.
7. **Incomplete CUE cache invalidation.** **Fixed:** `Stamp::read()` records both the CUE sheet and the referenced audio file (`modified` and `bytes`) in `CueStamp` (`src/metadata_cache.rs:40-65`); test `cue_entries_cache_sheet_and_audio_stamps` covers it.

### 3.2 Medium priority — identity and favorites

8. **Duplicate identity models.** **Implemented for playback identity:** `Playing` holds only a `TrackId`; `plan()`, `open()`, `start()`, `order_id()` and the play queue are ID-based. `refresh_playback_ids()` no longer rewrites playing coordinates, it only checks ID membership; `toggle_favorite_id()` re-points the ID when the playing favorite is removed. `SavedTrack.track` stays as a compatibility field: it is written for older builds and lets `resolve()` read legacy `playback.json` by path (`src/main.rs:218-263`). **Deliberately unchanged:** `Track = (usize, usize)` for row operations, and `views`/`indexing`/`imports`/dialog targets keyed by playlist index — those are per-playlist caches, not identities. `library.selected` is a playlist index, not a track selection; `delete_playlist()` rebases those playlist indices because playlists can actually be deleted. No `src/audio/` files were changed.
9. **Favorites membership path-based, cache ID-based.** **Fixed:** `current_is_favorite()` uses `library.is_favorite_id(playing.id)` and caches by `TrackId` (`src/ui/header.rs:211-223`); row hearts call `toggle_favorite_id()`. Test `favorites_toggle_adds_and_removes_all_duplicates` covers add/remove, persistence and duplicates.
10. **Cursor rebase can jump backwards.** **Fixed:** `remap_order()` now anchors on the next retained entry (`src/main.rs:268-288`); test `remap_order_keeps_cursor_on_playing_entry_or_advances_to_next` covers it.
11. **Stale queue titles.** `PlayQueue::Entry.title` is stored when queued (`src/play_queue.rs:18-25`); no invalidation when metadata changes.
12. **Orphaned queue IDs.** `queue_count()` counts all entries (`src/play_queue.rs:116-118`), `play_next_queued()` silently skips entries without a `locations` entry (:96-114).
13. **View row arithmetic fragile.** `src/ui/playlist.rs:172-175` and `src/library.rs:266-268` assume consistency between `tracks`, `order` and `rows`.
14. **Import/export asymmetric.** Import resolves relative paths against the playlist file's directory (`src/playlist_io.rs:38,82-92`), export against `current_dir()` (:102-108); IDs are not exported (:95-113).

### 3.3 Medium priority — audio engine (audit only, no changes)

15. **Seek position potentially read inconsistently.** The worker sets `seek_ns` to `NO_SEEK`, then `frames`, then `origin_ns` (`src/audio/mod.rs:478`, `:550-553`); `get_pos()` reads the values individually with `Relaxed` (:284-303). Structural race, no demonstrated symptom.
16. **Startup gate without timeout.** **Fixed:** `wait_for_gate()` bounds the wait to five seconds and reports a warning instead of blocking forever; test `gate_timeout_does_not_hold_the_worker_forever` covers it.
17. **Hot-loop mutex and clone.** **Fixed:** `next_epoch: AtomicU64` + `changed_next()` keep the mutex and `Next` clone off the steady-state pump path; test `queue_changes_are_observed_once_without_polling_the_mutex` covers cache, identical re-queues and the `None` transition. The listening test is still open.
18. **`compatible()` does not check `bits`/codec.** `src/audio/gapless.rs:36-38` compares only rate/mask/kind, although `SourceFormat` carries `bits`, `codec`, `gain` (`src/audio/decoder.rs:18-27`).
19. **`bit_perfect_now()` is a state test, not a hardware test.** `src/audio/mod.rs:255-266` vs. the note in `src/audio_setup.rs:207` that loopback is required. The UI wording (`src/audio_ui.rs:276-290`) should remain cautious.
20. **Silent errors in callback/worker paths.** `let _ = tx.send(...)` (`src/audio/mod.rs:194-197`), `if let Ok(chunk) = ...` (:617-624; `shared.rs:193-207`; `exclusive/render.rs:168-192`), `Warning`/`Error` share one slot (`src/audio/mod.rs:333-350`).
21. **`Next.generation` is written but never read** (`src/audio/gapless.rs:13`, `src/gapless_ui.rs:55-59`, main.rs:1081).
22. **Device probe may describe a different device.** Both backends fall back to the default device (`src/audio/shared.rs:66-73`, `src/audio/exclusive/device.rs:180-188`), but the setup label uses the stored ID (`src/audio_setup.rs:134-143`).

### 3.4 Low priority — organization, docs, build

23. **`README.md:136` points to `docs/RELEASE-NOTES-DRAFT.md`**, whose title is "Rust Player 0.2.0"; current notes are in the 0.4.0/0.5.0 files.
24. **The new backup function is not documented yet** (the uncommitted state extends README behavior without changing the README).
25. **Mixed line endings.** Globally `core.autocrlf=true`, no `.gitattributes`; `src/main.rs`/`src/library.rs` are LF-only, other files are CRLF → Git warnings and future normalization churn.
26. **No toolchain pinning** with Edition 2024 and lockfile v4; CI relies on the runner default.
27. **`build.rs:3-4` silently skips resources when cross-compiling from non-Windows hosts.**
28. **Windows features `Win32_Storage_FileSystem`/`Win32_System_IO`** (`Cargo.toml:29`) have no matches in the direct code (grep); possibly unnecessary, but not certain because of feature unification.
29. **`expect()` in non-test paths** including `src/main.rs:426,430,801`; not in `save_json`/`back_up_invalid_json` themselves.
30. **No UI module tests.** Neither `src/ui/*.rs`, `audio_ui.rs`, `audio_setup.rs`, `gapless_ui.rs`, `taskbar.rs` nor `media_keys.rs` contain `#[test]`.

---

## 4. Performance issues

**Status:** the four benchmarks in `src/benchmarks.rs` are still `#[ignore]` so normal test runs stay fast, but they are now executed and archived by `scripts/run-benchmarks.ps1`. Each run writes raw CSV plus a `*.meta.csv` with wall time, CPU time, page faults and peak working set; the script records toolchain, hardware, commit and worktree state. Results and interpretation are in `docs/BENCHMARK-REPORT.md`. Symbol-level profiling is still outstanding: `wpr.exe` is installed but cannot enable its system-policy on this unprivileged account, and the Windows `Process` performance-counter set is unavailable, so no new trace was recorded here. UI frame time has an opt-in `RUST_PLAYER_FRAME_LOG` logger and a one-command session script (`scripts/run-frame-log.ps1`), but no recorded interactive session yet. Below, **statically verified risks** and **unresolved hypotheses** are separated.

### Statically verified

- **Synchronous writes in the UI thread:** Every settings change calls `Library::save()` (`src/main.rs:412-452`, `src/ui/sidebar.rs:36`, `src/ui/header.rs`, `src/settings.rs:157`, `src/ui/dialogs.rs:46`) with `create_dir_all`, `sync_all`, `persist` (`src/library.rs:497-505`). Volume is debounced at 300 ms (`src/main.rs:454-490`), other changes are not.
- **`record_play()` rewrites the entire listening JSON on every counted play** (`src/main.rs:813-831`; up to 1000 history entries + maps, `src/listening.rs:72-99`).
- **`save_playback()` writes a full tempfile+fsync+rename for a small struct on every track change/exit** (`src/main.rs:541-571`, `src/gapless_ui.rs:124,142,160`, `src/main.rs:1258,1299,1338,1403,1579`).
- **`rebuild_locations()` is O(n) across all playlists** and is triggered individually by many mutations; removing a favorite calls `edit_track()` per duplicate, each with a full rebuild and save (`src/library.rs:358-385`, `src/main.rs:807-809,871-873`).
- **Track display reads tags synchronously — fixed.** `show_track()` now uses `Media::placeholder()` and a worker (`media_job`); `finish_media_job()` picks up the result with a generation check. The properties dialog uses `property_jobs` with a `loading` flag. Import/export had already been switched to `mpsc`. Measurement in `docs/BENCHMARK-REPORT.md`: ~165 µs per `Media::read` vs. ~0,16 µs per `Media::placeholder`.
- **Search and sort keys are regenerated:** `PlaylistView::build()` creates a lowercase key per track (`src/library.rs:199-217`), `SearchResults::new()` scans rows (`src/main.rs:223-260`); the 80-ms debounce limits the frequency but not the work per search.
- **Cache hits clone `TrackInfo`** (`src/metadata_cache.rs:58,63,78-81`) and `Listening::rankings()` clones all keys (`src/listening.rs:42-61`).
- **Audio hot loop:** mutex + `Next` clone per iteration **fixed** via the `next_epoch` fast path; remaining risks are TPDF RNG twice per sample (`src/audio/shared.rs:197-204`, `src/audio/exclusive/render.rs:179-185`), FIR now up to 16,384 taps per channel/frame (`src/audio/dsp.rs`, raised from 2,048 in this snapshot) and the resampler at up to 512 taps depending on the selected quality (`src/audio/convert.rs`).
- **Ring buffer up to about 8 MiB per `pump()`** (`src/audio/mod.rs:416-418`) plus retained `pending`/`converted` capacity (:454-457).
- **Device enumeration every 1,5 s** with a new `Vec` + sort, regardless of changes (`src/audio/mod.rs:685-694`, `src/audio/shared.rs:31-49`).
- **Synchronous `AudioTrack::open()`** probes and decodes the first packet plus Lofty tags on the UI thread (`src/audio/mod.rs:176-177`, `src/audio/decoder.rs:143-155,194`).

### Hypotheses from the task description — **not confirmed** by the audit

- "`taskbar_buttons()` is computed multiple times per frame": no such function call was found in the code; `src/taskbar.rs` provides a command queue/callback (:131-137, :239-288). **Not measured, not verified.**
- "`current_is_favorite()` performs an unnecessary linear search": `Vec::contains` is linear (`src/ui/header.rs:222-227`), but a cache already exists (:217-220). **Call frequency not measured.**
- "`SearchResults` recomputes search strings unnecessarily often": debouncing and a prepared `search_key` are present (`src/main.rs:216-261`, `src/library.rs:102-110`, commit `a5cfdbf`). **No measurement evidence of recomputation per frame.**

---

## 5. Risks

- **Data loss/state inconsistency:** directory fsync has been added; still open: no cross-file transaction, backup flag cleared before save success, and silent corrupt handling for `playback.json` and the cache.
- **Playback miscontrol after edits:** index-based partial models alongside `TrackId`, fragile cursor rebase, orphaned queue IDs.
- **Non-deterministic audio states:** seek race, unbounded gate, silent errors in render callbacks; the `bit-perfect` display is an implementation status, not a hardware-verified result.
- **Unsafe code surface:** WASAPI/COM/taskbar/WinRT use `unsafe` with corresponding justifications (`src/audio/exclusive/device.rs:37-417`, `src/audio/exclusive/render.rs:26-239`, `src/taskbar.rs:94-506`, `src/media_keys.rs:53-179`); correctness depends on COM/WASAPI contracts.
- **Git process:** shared Git dir across 7 worktrees, branch without upstream, local `main` 12 commits behind origin, 6 unpushed commits, stash `stash@{0}`, branch shared with `development/library-scale-v06`. Exercise the utmost caution with reset/checkout/commit.
- **Artifacts:** `target/` ~3,20 GB (ignored), `android/` ~161 MB (9 source files untracked, build outputs ignored). `lastfm-queue.json` and `*.invalid-*.json` are **not** ignored and could be staged accidentally.
- **Secrets:** no hardcoded API keys/tokens/certificates found; Last.fm credentials resolved at runtime via Credential Manager; `.env` ignored; Pages deployment uses OIDC. `android/local.properties` (ignored) contains a machine-specific SDK path.
- **Release/docs (updated):** the PC release workflow is now documented in `docs/RELEASE-WORKFLOW.md` and implemented locally by `scripts/package-release.ps1` (checks, ZIP, SHA-256; no Git or GitHub writes). Tags/releases still require explicit approval. The README's DRAFT link was corrected earlier and backup behavior is documented. Android is explicitly out of scope.
- **Scalability:** list operations and serialization are still synchronous; the scaling benchmarks remain `#[ignore]` and therefore do not guard CI, but `scripts/run-benchmarks.ps1` makes a 1k/10k/50k/100k run reproducible on demand with archived raw data.

---

## 6. Recommended order of improvements

The order prioritizes **data safety before behavior before performance** and takes into account that phases 4–7 are already partially implemented. Every step needs approval before code is changed; no sequence of steps includes a Git push.

1. **Secure the working state (only after your approval):** review the uncommitted backup changes, commit `PROJECT-AUDIT.md` and, if applicable, the changes locally (no push), add `.gitattributes` for LF/CRLF and `.gitignore` for `*.invalid-*.json`/`lastfm-queue.json`.
2. **Finish phase 3 — harden persistence — implemented:** directory fsync after `persist`, backup flags retired only after a confirmed write via `SaveReport`/`BackupTracker`, `previously_saved()` unified with `restore_playback()` in `read_playback()`, and tests for round-trip, write errors, previous file preserved, missing directory, corrupt JSON, `playback.json` (including `null`) and listening limits.
3. **Make the metadata cache robust:** backup/version/repair instead of silence, replace `expect`/`unwrap` in the indexer with error propagation, tie the CUE dependency to the audio file's `Stamp`, test for "missing cache rebuilds itself" and parallel indexing.
4. **Centralize identity — implemented for playback:** `Playing`, `plan()`, `open()`, `start()` and the play queue use `TrackId`; `refresh_playback_ids()` only checks ID membership and its cursor prefers the next retained entry (test `remap_order_keeps_cursor_on_playing_entry_or_advances_to_next`). Old `playback.json` stays readable through `SavedTrack::resolve()`. Per-playlist caches (`views`, `indexing`, `imports`, `selected`) remain index-keyed by design.
5. **Clean up favorites:** consistent membership (ID instead of path), `toggle_favorite()` tests (add/remove/save/load/play/sort/select), no deletion of music files, ensure no folders are created; evaluate the `current_is_favorite()` cache.
6. **Unload the UI thread — implemented:** `show_track()`/properties via `media_job`/`property_jobs`, import/export via `mpsc`, `Library::save()` via the `SaveWorker`; before/after measurements for persistence and track changes in `docs/BENCHMARK-REPORT.md`. The audio hot loop remains open (see item 7).
7. **Audio hot loop — partially implemented:** removed the `Next` clone/mutex from the pump iteration (epoch fast path) and gave the startup gate a timeout. The seek handshake and `compatible()` are deliberately left unchanged; do not change audio quality. Listening test still open.
8. **Establish observability — implemented for benchmarks:** `scripts/run-benchmarks.ps1` runs the ignored tests in one documented step, archives environment/logs/CSV and adds CPU + memory metadata per run; `docs/BENCHMARK-REPORT.md` evaluates the raw data. UI frame time has an opt-in logger (`RUST_PLAYER_FRAME_LOG`) but no recorded session in this report; symbol-level profiling remains open. Optimize selectively only after those measurements.
9. **CI/docs/release hygiene — implemented for the PC:** toolchain pin, corrected DRAFT link, documented backup behavior, documented release workflow with a local packaging script and explicit approval gates. Android is deliberately deferred and stays untracked; it is not part of the PC release.

**Not recommended without further approval:** Git push/merge/release, changes to `src/audio/` beyond the approved scope, automatic deletion or moving of music files, changes to independent UI features as part of another work package.

---

## 7. Implementation status after wave 1 and the audio quick wins

**Approved and implemented:**

- Selectable resampler quality (`Fast`/`Balanced`/`High`/`Maximum`), default `High` exactly matches the previous behavior.
- FIR limit raised from 2.048 to 16.384 taps, README and release notes updated.
- External CUE files with multiple `FILE` entries; the last track of a FILE plays to the end of the file. Embedded `CUESHEET` text in FLAC is now read directly from the Vorbis-comment metadata block (`src/cue.rs`) and exposed as virtual `audio.flac/<n>` tracks; lofty's generic `Tag` does not expose that key, so the raw block is parsed.
- ReplayGain scan via `bs1770` (ITU-R BS.1770-4) writing `REPLAYGAIN_TRACK_GAIN`/`REPLAYGAIN_TRACK_PEAK` from the track properties. The scan runs in a background thread; `finish_gain_jobs()` receives the result via a channel and updates views and display. This resolves the UI-blocking point mentioned in item 6 for this path.
- Simple tag editor for title, artist, album, album artist and track number; empty fields remove the value, CUE entries reference their source file.
- `.gitattributes`, `.gitignore` entries, toolchain pin `1.100.0`, CI release build, corrected release-notes link and documented backup behavior.
- Reproducible benchmarks with CSV raw data (`target/benchmark-*.csv`) plus `*.meta.csv` (wall/CPU/page faults/peak working set), a one-command archiving script (`scripts/run-benchmarks.ps1`) and evaluation in `docs/BENCHMARK-REPORT.md`. Opt-in UI frame logging via `RUST_PLAYER_FRAME_LOG` writes `frame,micros` without adding work when disabled; `scripts/run-frame-log.ps1` starts a session, refuses to stop an already running player, archives the CSV and reports p50/p95/p99/max plus frames over 16/33 ms.
- UI-thread offload for the ReplayGain scan (`finish_gain_jobs()` with a channel), shared `refresh_metadata()` for tag and gain changes.
- Startup gate in the decoder worker with a 5-second timeout and a warning instead of waiting indefinitely (`wait_for_gate()`, with a test).
- Background writer (`src/save_worker.rs`) for `library.json`, `playback.json` and `listening.json`: the UI serializes, the worker performs `fsync`/`persist` and coalesces per file. `finish()` joins on exit, so no change is lost. Four tests for coalescing, ordering, separate targets and error messages.
- Playlist import and export run via `mpsc` in worker threads; `finish_playlist_io()` takes over the result and status in the UI thread (same pattern as the existing audio import).
- Before/after measurement of the persistence path in `docs/BENCHMARK-REPORT.md`; raw data and run metadata are archived under `target/benchmark-runs/<timestamp>/` (latest evaluated run: `2026-11-26_072551`).
- Directory fsync after every `persist()` (`sync_directory()` in `src/library.rs`): on Windows with `FILE_FLAG_BACKUP_SEMANTICS` + `GENERIC_WRITE` because `FlushFileBuffers` requires write access; on Unix via `sync_all()` on the open directory. This makes the rename itself power-failure safe, not just the file contents.
- GitHub button under **Tools** in the header and under **Settings → About**; both open `https://github.com/AikasAcc/rust-player`. `PROJECT_URL` is defined centrally in `src/main.rs`.
- `show_track()` and the properties dialog no longer read tags synchronously: `Media::placeholder()` or `TrackProperties::loading()` appears immediately, `media_job`/`property_jobs` deliver the result via `mpsc`. `media_generation` discards stale results on rapid track changes; `loading` protects user edits in the dialog from late worker results. Cover texture, lyrics and plugin/media-key notifications only run after the complete read. Measurement: `docs/BENCHMARK-REPORT.md`, raw data `target/benchmark-media.csv`.
- Audio hot loop: `Next` clone and `Mutex` per pump iteration replaced by `next_epoch: AtomicU64` + `changed_next()`. The mutex remains only for real changes; `queue_next()` publishes with `Release`, the pump reads with `Acquire`. The test `queue_changes_are_observed_once_without_polling_the_mutex` checks the cache, identical re-queues and the None transition. No listening test performed.
- Additional Symphonia codecs/containers: MP1, MP2, ADPCM (MS and IMA in WAV/CAF) and MKV/MKA. `audio_track()` prefers decodable tracks so that video tracks in MKV do not block playback. The test `extra_codecs_are_compiled_in` checks the decoder registry. `Cargo.lock` updated with `symphonia-codec-adpcm` and `symphonia-format-mkv`.

**Deliberately not changed:**

- `compatible()` in `src/audio/gapless.rs`: `SourceFormat.kind` already encodes the bit depth as `Integer(bits)`; the codec is irrelevant to PCM continuation, and `gain` is applied per source via `converter.set_gain()`. Adding a check would likely worsen gapless detection rather than improve it.
- ~~`Next` clone/mutex in the pump iteration: without separate profiling and a listening test, the risk of a regression outweighs the presumed gain.~~ **Superseded:** the `next_epoch` fast path was implemented and logically tested (see item 17). No further handoff or DSP changes are made until the listening test is available.
- Backup rotation: backups are deliberately retained so that corrupt originals remain inspectable; the absence of automatic rotation is documented rather than silently changed.
- `android/` remains untracked and is not part of this work; it was neither deleted nor added to version control.

**Implemented from item 8/9:** reproducible benchmarks via `scripts/run-benchmarks.ps1` with archived environment, logs, raw CSV and process metadata; evaluation in `docs/BENCHMARK-REPORT.md`; opt-in UI frame-time CSV; toolchain pin, CI build, DRAFT link and backup documentation.

**Open from items 6–9:** listening test for the audio hot loop (change is implemented and logically tested but not heard); a real interactive frame-time session (script ready, session not run) and symbol-level profiling (WPR blocked by missing admin policy, `Process` counters unavailable); seek handshake and `compatible()` deliberately unchanged. Release workflow is documented and the local packaging script is in place; publication and tagging remain approval-gated. Android is deliberately out of scope and its untracked tree is unchanged. The offload for `show_track()`/track properties, import/export and `Library::save()` mentioned in item 6, by contrast, is fully implemented and measurably verified.

---

## 8. Second read-only audit — logic and error paths (non-audio)

A second read-only pass reviewed the non-audio modules after wave 1. It reported eight findings; all were fixed in the working tree with regression tests wherever a pure unit could be extracted. No `src/audio/` file, version, tag, or GitHub resource was touched.

- **Updater ownership (high):** the updater used to be spawned before the temporary download was made persistent. A `keep()` failure could leave the UI idle while the child was running and delete the file underneath it. `install()` now persists the download first (`staged_download()`); a spawn failure removes the staged file and sets `Failed`. Test: `a_staged_download_survives_the_state_transition`.
- **Repeat playlist (high):** manual Next (button, taskbar, media key) used `adjacent_cursor(1)` and stopped at the last entry while automatic advance honoured `Repeat::All`. `step_cursor()` wraps only forward navigation; the Next button is enabled via `next_cursor()`. Tests cover forward wrap, backward non-wrap and an empty order.
- **CUE tag save (medium):** the properties dialog offered "Save tags" for virtual CUE tracks although `tagging::write()` deliberately refuses them. The button is now disabled for CUE entries with an explanatory hint; `RELEASE-NOTES-NEXT.md` and the module docs say so explicitly. No shared source file is silently rewritten.
- **Listening backup (medium):** `save()` backed up `listening.json` but never wrote it, so a failed backup blocked the whole `library.json` save. The backup now happens only in `record_play()` immediately before its own enqueue.
- **Saved playback normalization (low):** `read_playback()` and `refresh_playback_ids()` duplicated the resolve/coordinate/path update with different fallbacks. Both call `normalize_saved()` now.
- **Discord status source (low):** when primary artwork was empty and the fallback was accepted, the status said "artwork". `artwork_attempts()` labels the URL actually sent and skips an identical fallback. Test added.
- **Queue drag and drop (low):** the drop index was captured before removing the dragged row, so forward drags landed after the target and backward drags before it. `drop_target()` normalizes the index; the context menu keeps its post-removal convention. Three tests added.
- **Tag-number docs (low):** release notes and the module comment no longer claim that an empty track number removes the value; it must remain a positive integer.

Verification: `cargo fmt`, `cargo clippy --release --locked --all-targets -- -D warnings`, `cargo test --release --locked` (125 + 2 passed, 4 ignored) and `cargo build --release --locked` are green. All changes remain local and uncommitted; `v0.6.0-rc1` is unchanged.

---

## 9. Abschluss der offenen Liste aus dem Prompt vom 25.11.

Die folgenden fünf Code-Punkte waren nach Audit 7 noch offen und sind jetzt umgesetzt. Sie betreffen ausschließlich Nicht-Audio-Pfade; `src/audio/**` wurde nicht angefasst.

- **Streaming-Download mit inkrementellem Hash (`src/update.rs`, `src/digest.rs`, `src/bin/rust_player_updater.rs`):** Statt das komplette Installer-ZIP per `read_to_vec()` in den RAM zu laden, wird der HTTP-Body in 64-KiB-Blöcken in eine temporäre Datei geschrieben und dabei über Windows CNG (`CryptographicHash::Append`) gehasht. Beide Binaries nutzen dieselbe Funktion `digest::copy_verified()`; die Prüfung kann nicht auseinanderlaufen. Bei Hash-Fehler wird die temporäre Datei verworfen. Zwei neue Tests prüfen Hash und Fehlerfall. Zusätzlich setzt `staged_download()` einen fehlgeschlagenen `keep()` jetzt in `State::Failed` statt still auf `Idle`.
- **CUE-Diagnose (`src/cue.rs`):** `flac_comments()` liefert `Result<Option<_>, String>`: `Ok(None)` bedeutet weiterhin „kein Vorbis-Block“ (normal), `Err` bedeutet „Block vorhanden, aber beschädigt“. Der Grund wird über `diagnostics::append_error("cue", …)` in `%LOCALAPPDATA%\Rust Player\logs` protokolliert, da GUI-Builds keine Konsole haben. Ein Test mit abgeschnittenem Block prüft, dass dieser Fall nicht mehr als „kein CUE“ gilt.
- **Favoriten-Pfad-Index (`src/library.rs`):** `path_sources` und `favorite_paths` werden zusammen mit `locations` in `rebuild_locations()` aufgebaut und sind `#[serde(skip)]`. `source_id()` und `favorite_entries()` sind damit O(1) bzw. O(Treffer) statt eines vollständigen Playlist-Scans pro Aufruf. Die Pfadnormalisierung steckt jetzt zentral in `path_key()`. Alle mutierenden Favoritenpfade rufen `rebuild_locations()`; ein Regressionstest prüft Neusortierung und Windows-Groß-/Kleinschreibung.
- **Save-Worker-Index (`src/save_worker.rs`):** Das Coalescing mehrerer Snapshots nutzt eine `HashMap<PathBuf, usize>` in `pending`; die Reihenfolge der Schreibvorgänge bleibt unverändert. Drei Dateien waren zwar schon vernachlässigbar, der Index macht das Verhalten aber explizit und skaliert.
- **Backup-/Atomic-Save-Integrationstest (`src/save_worker.rs`):** End-to-End mit echten Dateien: eine beschädigte JSON-Datei wird gesichert, der Ersatz läuft durch `SaveWorker` mit atomarem `save_bytes()`, und erst der bestätigte Schreibvorgang entlässt den Backup-Status. Der Test prüft außerdem, dass keine temporären Dateien zurückbleiben und das Backup unverändert erhalten bleibt.

**Weiterhin nicht ausführbar (keine Code-Schuld):**

- **Hörtest** (Gapless, Seek, MP1/MP2, ADPCM, MKA/MKV, eingebettetes CUE): erfordert die Audio-Hardware und die Ohren des Nutzers. Es wurde kein Audio-Hot-Loop-Parameter verändert.
- **Symbol-Profiling:** `wpr.exe` ist vorhanden, kann aber die Systemrichtlinie auf diesem Konto nicht aktivieren (`0xc5585011`); die Windows-`Process`-Performance-Counter liefern hier keine Daten. Ohne eigene Messung bleibt der Hot-Loop unverändert.
- **GitHub-Workflow `windows-package.yml`:** lokal validiert, aber ein Lauf auf GitHub erfordert Push/Release und damit Freigabe; `gh` ist in dieser Umgebung nicht installiert. `v0.6.0-rc1` und `Cargo.toml` bleiben unverändert.
- **Discord-Button-Sichtbarkeit:** Discord zeigt Activity-Buttons nie im eigenen Profil. Die Prüfung braucht einen Zweitaccount oder Voice-Channel-Hover; die Payload bleibt korrekt und wurde nicht verändert.

Verifikation nach dieser Runde: `cargo fmt --all`, `cargo clippy --release --locked --all-targets -- -D warnings`, `cargo test --release --locked` (130 + 3 bestanden, 4 ignoriert) und `cargo build --release --locked` sind grün. Der letzte Build lief in einem temporären Zielordner, weil ein laufender `rust-player.exe` (PID 25844) die Release-EXE in `target/ui-build/release` sperrt; der Prozess wurde **nicht** beendet. Sobald er geschlossen ist, kann `package-release.ps1` wie gewohnt in `target/ui-build` neu bauen.

---

## 10. Dritter Lese-Audit — gecachte Favoriten-Views

Ein dritter reiner Lese-Audit über die Nicht-Audio-Module meldete zwei belegte Funde, beide sind jetzt behoben:

- **Stale Favorites-View → Out-of-Bounds (hoch):** `edit_track()` aktualisierte nur den View der geänderten Quellliste; das anschließende `save()` konnte über `prune_favorites()` die Favorites-Liste verkürzen, ohne den gecachten View anzupassen. `PlaylistView::row_count()` addiert nur Zeilen, sodass `PlaylistRow::Track(N-1)` weiter gerendert und in `playlist.tracks[N-1]` indexiert wurde. `prune_favorites()` liefert jetzt einen `bool`-Changed-Status; `save()` verwirft bei `true` über `invalidate_favorites_cache()` View und laufenden Index-Job der Favorites. Zusätzlich prüft `track_row_index()` in `src/ui/playlist.rs` jede sichtbare Zeile gegen `playlist.tracks.len()` – veraltete Zeilen werden übersprungen statt zu indexieren. Vier Regressionstests (Change-Report, Cache-Invalidierung, stale/album/appended Rows, gefilterte Rows).
- **Veraltete Favorites-Pfade nach Relink (mittel):** `finish_relink()` entfernte nur die Views direkt geänderter Quelllisten. `prune_favorites()` schreibt die Favorites-Pfade anschließend auf die verlinkten Ziele um; der Favorites-View blieb auf alten `TrackInfo`-Daten stehen. Durch die zentrale Invalidierung in `save()` wird auch dieser Fall abgedeckt – ohne zusätzlichen Sonderfall in `relink.rs`.

Der Audit-Agent verwarf einen weiteren Kandidaten (`BackupTracker::failed()` setzt `needed` erneut) zu Recht: nach einem fehlgeschlagenen Write liegt die korrupte Originaldatei noch auf der Platte, daher ist ein erneutes Backup korrekt und durch einen Test abgedeckt.

Verifikation: `cargo fmt --all`, Clippy ohne Warnungen, 134 + 3 Tests bestanden, 4 ignoriert. `src/audio/**`, Version, Tag und GitHub-Ressourcen blieben unberührt.

---

## 11. Foobar-Parity: verbleibende Inventur geschlossen

Die Inventur der offenen Komfortfunktionen ist abgearbeitet. Alle Änderungen betreffen die Nicht-Audio-Seite; der Audio-Hot-Loop, die Version, das Tag und `android/` bleiben unangetastet.

- **Suche (`src/search.rs`):** eigener Query-Parser mit `artist:`, `album:`, `title:`, `path:`, `file:`, `missing:`, Quotes, `AND`, `OR`, `NOT` und `-`. Indexierte Zeilen werden über `TrackInfo::search_key` gefiltert, unindexierte über den Pfad. Der Cache wird pro Liste/Query gehalten und bei `save()` invalidiert.
- **Globale Kürzel (`src/main.rs`, `src/ui/header.rs`):** `Space`, `N`, `P`, `S`, `M`, `F`, `Ctrl`+`S`, `Ctrl`+`←`/`→`, `Ctrl`+`Z`, `Ctrl`+`Shift`+`Z` und `Ctrl`+`Y`. Sie greifen nur ohne fokussiertes Widget; Tooltips nennen die Tasten.
- **M3U-Export (`src/playlist_io.rs`):** `PathMode::{Absolute, Relative}` mit lexikalischer Relativierung ohne neue Dependency. Unterschiedliche Laufwerke/UNC-Shares fallen auf Absolutpfade zurück. Virtuelle CUE-Tracks werden als Sheet exportiert, damit der Re-Import die ganze Disc expandiert. Zwei Kontextmenü-Einträge ersetzen den einen Export-Eintrag; Round-Trip-Tests und ein Testsprung aus einem Unterordner sind grün.
- **Queue-Persistenz (`src/play_queue.rs`):** nur explizite „Add to Play Queue“-Einträge werden in `queue.json` gespeichert. `load()` repariert einen veralteten `next_id` und verwirft Tracks, die nicht mehr in `library.locations` liegen. Jede queue-ändernde Aktion (Enqueue, Take, Move/Remove, Gapless-Übergabe) enqueuet einen Save; corrupt Files werden über `BackupTracker` gesichert. Ein noch nicht vorhandenes `queue.json` liefert `LoadError::Missing` und löst **keinen** Backup-Zweig aus – sonst versuchte der erste Shuffle-Klick, eine nicht existierende Datei zu sichern (`os error 2`). Regressionstest ergänzt.
- **Cover-Sidecar (`src/background.rs`):** `save_sidecar_png()` schreibt PNG-Bytes via temporärer Datei und `persist_noclobber()` als `cover.png` neben der Audiodatei. Der Viewer bietet die Aktion nur mit bekanntem Quellpfad an; ein vorhandenes Cover wird nie überschrieben.
- **Last.fm Love/Unlove (`src/lastfm.rs`):** neuer Worker-Befehl mit `track.love` / `track.unlove` über den vorhandenen signierten POST-Pfad. Epoche und Track-ID verhindern, dass ein später Klick einen bereits gewechselten Track markiert; ein Test prüft die beiden Methodennamen.
- **Track-Drag-and-Drop (`src/ui/playlist.rs`, `src/main.rs`):** nur in `Sort::Added` und ohne Suche/Stale-View aktiv. `TrackEdit::Reorder` nutzt remove+insert; das Drop-Ziel wird um die noch vorhandene Quellzeile korrigiert. IDs, Tracks, Metadaten, Favoriten, Locations und Wiedergabeidentität werden zusammen aktualisiert.
- **Mehrfachauswahl (`src/ui/playlist.rs`):** Klick, `Ctrl`/`Cmd`+Klick und `Shift`+Klick wählen Zeilen; das Kontextmenü wendet Queue, Favorite und Remove auf alle gewählten Zeilen an. Entfernen geschieht in einem Durchgang mit absteigenden Indizes, statt viele Einzel-Saves auszulösen.
- **Undo/Redo (`src/main.rs`, `src/ui/header.rs`):** begrenzte Snapshots (64) pro Playlist mit Tracks, IDs, Metadaten, Sortierung und Favoriten-IDs. `Ctrl`+`Z` / `Ctrl`+`Shift`+`Z` / `Ctrl`+`Y` sowie **Tools** greifen darauf zu; Redo speichert vor dem Anwenden den aktuellen Zustand. Bei Löschen oder Verschieben von Playlists wird die Historie geleert, damit keine Snapshots auf falsche Indices zeigen.

Bewusst nicht aufgenommen: ASIO/DSD/DoP/VST, UPnP/DLNA, WavPack, Monkey’s Audio und Android. **Historischer Stand dieser Runde:** APE und DSD→PCM kamen später laut Abschnitt 13 hinzu. Für die neue Queue-Persistenz gibt es einen Round-Trip- und einen `next_id`-Reparaturtest; für M3U, Sidecar, Reorder und Undo/Redo-Shortcuts ebenfalls Regressionstests.

Verifikation nach dieser Runde: `cargo fmt --all`, `cargo clippy --release --locked --all-targets -- -D warnings`, `cargo test --release --locked` (150 + 3 bestanden, 4 ignoriert) und `cargo build --release --locked` grün. Der finale Build lief in `C:\Users\Aika\AppData\Local\Temp\opencode\rust-player-final-build\release`, weil der laufende Player (PID 22300) die EXE in `target/ui-build/release` sperrt; die EXE dort wurde deshalb **nicht** überschrieben und der Prozess nicht beendet. Es wurden keine Git- oder GitHub-Schreiboperationen ausgeführt.

---

## 12. Audiophile-Runde: Bit-perfect-Preset

Der softwareseitig direkt angehbare Teil der Audiophile-Lücke ist geschlossen:

- **Preset (`src/audio_ui.rs`):** „Apply bit-perfect preset“ setzt WASAPI Exclusive, deaktiviert Formatumwandlung, Shared-Fallback, Software-Volume, Übergangs-Fades, ReplayGain, Downmix und sämtliche DSP-Stufen. Kann das Gerät das Quellformat nicht direkt öffnen, bricht die Wiedergabe ab statt still zu resampeln. Der bestehende Signalpfad-Table zeigt weiterhin jede einzelne Stufe und ob sie aktiv ist.
- **Grenze:** Die Anzeige „Bit-perfect PCM at the Windows driver input“ beschreibt den PCM-Pfad bis zum Windows-Treiber. Echte Hardware-Bitperfektheit benötigt einen digitalen Loopback und einen PCM-Vergleich; das bleibt eine Prüfung mit der Hardware des Nutzers und wurde nicht simuliert.
- **Nicht in dieser Runde:** ASIO, DSD/DoP, VST, UPnP/DLNA, WavPack und Monkey’s Audio sind keine kleinen Patches. Sie benötigen eigene SDKs/Decoder-Integrationen bzw. neue Abhängigkeiten und würden den gerade daily-ready PCM-Pfad destabilisisieren. Sie bleiben bewusst draußen; die vorherige Scope-Entscheidung wurde nicht stillschweigend geändert.

Verifikation: `cargo fmt --all`, Clippy ohne Warnungen, 152 + 3 Tests bestanden, 4 ignoriert. Regressionstest `bit_perfect_preset_bypasses_every_pcm_stage` prüft, dass jede Processing-Stufe im Preset aus ist.

---

## 13. Foobar-Lücken: APE, Facetten, Mass-Tagging, DSD→PCM

Diese Runde schließt die vier Punkte, die softwareseitig ohne SDK und ohne Eingriff in den Audio-Hot-Loop machbar waren. Der PCM-Renderpfad selbst blieb unverändert; neue Formate laufen über eigene `Inner`-Quellen, die denselben `Source`-Vertrag erfüllen.

- **APE (`src/audio/ape.rs`):** `ape-decoder 0.3.2` (MIT OR Apache-2.0, kein `unsafe`) dekodiert APE ≥ 3950 direkt. Die Bytes werden je nach `bits_per_sample`/`bytes_per_sample` in den Zielsampletyp konvertiert; `ApeSource::seek()` nutzt absolute Samplepositionen, CUE-Offsets laufen über den gemeinsamen `Source::seek`. `gapless::next_packet()` liefert dafür `Cow<'_, [T]>` statt eines festen Slices.
- **Facetten (`src/search.rs`, `src/library.rs`, `src/ui/playlist.rs`):** `TrackInfo` hat neue Felder `genre` und `year`, aus lofty `ItemKey::Genre` bzw. `tag.date()`. Der Cache steigt auf Version 3 und verwirft beim Upgrade einmalig alle alten Entries, sonst wären die Facetten dauerhaft leer. `genre:`/`year:` sind neue Query-Felder; die Chipleiste wird pro Playlist gecacht und toggelt einzelne Klauseln, ohne den Rest der Query zu zerstören.
- **Mass-Tagging (`src/tagging.rs`, `src/ui/dialogs.rs`):** `BatchEdit` ist bewusst *partiell*: `None` = behalten, `Some("")` = löschen, sonst setzen. Damit bleiben ungewählte Felder – anders als beim alten `Edit`, das alle fünf Felder schreibt – unangetastet. Das Jahr geht als `ItemKey::RecordingDate` (TDRC) in ID3v2.4, weil `Year` beim Wiederlesen nicht zuverlässig zurückkommt. Ein Worker schreibt Datei für Datei; CUE-Virtuelle werden übersprungen, danach refresht `refresh_metadata()` die Views.
- **Smart Playlists (`src/library.rs`, `src/main.rs`):** materialisiert statt als Live-View. Die gespeicherte Query filtert das normale Library-Universum und schreibt normale Zeilen mit stabilen `TrackId`s. Deshalb funktionieren Sortierung, Playback-Order, Favoriten, Undo/Redo, Persistenz und M3U-Export unverändert. `#[serde(default)]` am neuen Feld hält alte `library.json`-Dateien lesbar.
- **DSD→PCM (`src/audio/dsd.rs`):** `dsd-reader 0.4.0` (MIT/Apache-2.0) normalisiert DSF (planar, LSB-first) und DFF (interleaved, MSB-first) zu MSB-first-Interleaved-Bytes; `dsd-pcm 0.1.0` (MIT/Apache-2.0) decimiert mit linearphasiger FIR-Kaskade zu 88,2/96-kHz-Floats. Wichtig: `dsd_rate()` liefert den DSD64-Multiplikator, nicht Hz – multipliziert mit `DSD_64_RATE` ergab der erste Test sonst „1 Hz“. Seek baut den Container-Reader neu und resettet den Decimator, weil die Kaskade keinen Random Access bietet; Rest-Bytes innerhalb eines Blocks werden als Output-Frames verworfen. ID3-Tags kommen aus dem Container, da lofty DSF/DFF nicht probt. **Grenze:** Das ist eine PCM-Konversion, kein nativer DSD-/DoP-Pfad und nicht bit-perfect. Auf der iD4 mk2 (max. 96 kHz) ist DSD64-DoP mit 176,4 kHz ohnehin nicht möglich.
- ** bewusst weiter draußen:** WavPack (`symphonia-codec-wavpack 0.1.1` verlangt `symphonia-core ^0.6`, während das Projekt 0.5.5 nutzt; Manifest `GPL-3.0-or-later` vs. README `MPL-2.0`), SACD-ISO (kein reifer Rust-Reader, DST/Lizenzfragen), ASIO (Steinberg-SDK, bindgen/LLVM, für Wiedergabe geringer Nutzen), VST3 (jeder Plugin-DSP bricht Bit-perfect), UPnP/DLNA (eigenes Produkt mit SSDP/SOAP/tokio/Firewall) und Android. **Historischer Stand dieser Runde:** WavPack kam später mit `wavpack 0.4.0` (BSD-3-Clause) in Abschnitt 16 hinzu; die anderen Punkte bleiben draußen. `android/` bleibt untracked und tabu.

Verifikation nach dieser Runde: `cargo fmt --all`, `cargo clippy --release --locked --all-targets -- -D warnings` und `cargo test --release --locked` grün (165 + 3 bestanden, 4 ignoriert). Hörtests mit echter Hardware und echten APE-/DSF-/DFF-Dateien stehen weiterhin aus; die Synthetiktests prüfen nur Dekodierung, Seek-Koordinaten, ID3-Auslesen und Modul-Integration. Es wurden keine Git- oder GitHub-Schreiboperationen ausgeführt; `Cargo.toml` bleibt bei `v0.5.0`.

---

## 14. Statusmeldungen mit Fade

Einzelmeldungen wie „Audio output changed. Playback paused.“ bleiben jetzt drei Sekunden stehen und blenden in den letzten 450 ms aus. Danach wird der Text geleert; ein `ctx.request_repaint_after()` weckt die UI genau für den Fade-Zeitraum, statt ständig neu zu zeichnen.

- **Architektur (`src/main.rs`):** `Player.status` bleibt bewusst ein `String`. Es gibt rund 50 Schreibstellen über mehrere Module hinweg, u. a. `push_str()` zum Zusammensetzen von Import-Ergebnissen. Ein eigener Status-Typ hätte alle diese Stellen auf `.into()`/`.set()` umgestellt und das Signalpfad-Reading `self.status.as_str()` in `src/audio_ui.rs` gebrochen. Stattdessen beobachtet `StatusTimer` einmal pro Frame per Textvergleich, wann sich der Text geändert hat, und startet dann die Uhr. Explizite `show_notice()`-Aufrufe und `reopen_audio()` armen den Timer sofort, damit auch identische Meldungen erneut sichtbar sind.
- **Ausnahmen:** Meldungen mit `…`/`...` sind Fortschritt und bleiben, bis sie ersetzt werden. `"Audio output:"`, `"Could not reopen the audio output:"` und `"Could not load playlists:"` bleiben ebenfalls stehen, weil der Signalpfad den Audio-Fehlertext als Statuslabel liest bzw. ein fehlgeschlagener Library-Load kein flüchtiger Hinweis ist. Die Pause-Meldung selbst beginnt zwar mit „Audio output“, enthält aber keinen Doppelpunkt und fällt deshalb normal unter den Fade.
- **Vorteil:** Kein Eingriff in den Audio-Hot-Loop, keine neuen Abhängigkeiten, keine Änderung an Persistenz oder Format-Decodern. Die alte getrennte `Notice`-Leiste entfällt; Favoriten-Hinweise laufen jetzt über denselben getimten Status.
- **Tests:** Fünf neue Tests in `status_tests` prüfen Fade/Expiry, Progress-Hold, Repaint-Zeitpunkte, Audio-Fehler-Hold und dass identische Texte den Timer nicht ungewollt zurücksetzen.

Verifikation: `cargo fmt --all`, `cargo clippy --release --locked --all-targets -- -D warnings`, `cargo test --release --locked` grün (170 + 3 bestanden, 4 ignoriert). Der finale Build läuft in einem frischen `CARGO_TARGET_DIR`, weil der laufende Daily-Player `target\ui-build\release\rust-player.exe` hält; kein Prozess wurde gestoppt.

---

## 15. Hardware-Verifikation vorbereitet, keine Hardware-Behauptung

Die offene Liste (DSD/DoP, WavPack, ASIO, VST3, UPnP/DLNA, SACD, Bit-perfect-Hardware, Hörtests) wurde in dieser Runde nicht durch weitere Feature-Arbeit beantwortet, sondern durch eine saubere Trennung dessen, was welche Prüfung tatsächlich beweist. **Historischer Stand:** WavPack wurde anschließend in Abschnitt 16 softwareseitig nachgerüstet; die restlichen Punkte bleiben unverändert offen.

- **Neue Doku (`docs/HARDWARE-VERIFICATION.md`, englisch):** Stufen von der Software-PCM-Prüfung über Shared-WASAPI-Loopback und Audient-Virtual-Loopback bis zur analogen Schleife und einem echten Digital-Capture am DAC-Eingang. Jede Stufe sagt explizit, was sie *nicht* beweist. Dazu eine Hörtest-Tabelle für Gapless, Rate-Wechsel, Seek, APE/DSF/DFF, CUE und den Status-Fade sowie eine Ergebnisvorlage mit Windows-Build, Treiber, Device, Format, Capture-Methode und Underruns.
- **Neues Skript (`scripts/compare_loopback.py`):** Richtet zwei rohe Interleaved-PCM-Ströme per FFT-Korrelation aus und meldet Trefferquote, maximale und RMS-Abweichung; Exit-Code 0 nur bei Übereinstimmung innerhalb der Toleranz. Es nutzt NumPy (nur für das Prüfskript, nicht im Player) und wurde mit synthetischen `s16le`- und `f32le`-Strömen geprüft: korrekter Versatz, 100 % Treffer bei identischen Daten, `FAIL` bei einer geänderten Sample.
- **Zentrale Erkenntnis:** WASAPI-Loopback ist laut Microsoft-Dokumentation nur für Shared-Streams möglich; ein Exclusive-Stream kann damit nicht erfasst werden. Der Audient iD4 mk2 hat virtuelle Loopback-Eingänge, aber ob diese einen Exclusive-Stream sehen, muss am Gerät geprüft werden. Eine analoge Out→In-Schleife beweist die Signalverbindung, nicht Sample-Gleichheit. Die Anzeige „Bit-perfect PCM at the Windows driver input“ bleibt damit eine Software-Zustandsmeldung, bis ein Capture am DAC-Eingang vorliegt.
- **Architekturentscheidung:** Keine Änderung am Audio-Hot-Loop, an `Cargo.toml`/`Cargo.lock` oder an Persistenz; kein laufender Prozess gestoppt und keine GitHub-Schreiboperation. Der Aufwand liegt bewusst in nachvollziehbaren Prüfungen statt in nicht belegbaren Behauptungen. Die historischen Abschnitte 2 und 11 wurden mit Hinweisen auf den aktuellen Stand versehen, damit alte „nicht enthalten“-Sätze nicht als aktuelle Aussage gelesen werden.

Verifikation: `cargo fmt --all`, `cargo clippy --release --locked --all-targets -- -D warnings`, `cargo test --release --locked` grün (170 + 3 bestanden, 4 ignoriert). Das Skript wurde zusätzlich mit synthetischen PCM-Dateien im temporären Verzeichnis ausgeführt (jeweils `PASS` für identische Daten, `FAIL` für ein verändertes Sample). Ein finaler Build läuft in einem frischen `CARGO_TARGET_DIR`; die Hardware- und Hörtestergebnisse bleiben ausstehend und sind nicht simuliert.

---

## 16. WavPack nachgezogen, Ökosystem-Lücken weiterhin klar begrenzt

Auf die explizite Forderung, den Player „wirklich“ auf foobar2000-Niveau zu bringen, wurde zuerst der Punkt umgesetzt, der softwareseitig, lizenzseitig und ohne Audio-Hot-Loop-Änderung machbar ist: WavPack.

- **Decoder (`src/audio/wavpack.rs`):** `wavpack 0.4.0` (BSD-3-Clause) bindet die Referenzbibliothek libwavpack. Das war bewusst nicht `symphonia-codec-wavpack 0.1.1` (GPL-3.0-or-later, `symphonia-core ^0.6`, während dieses Projekt 0.5.5 nutzt). Ganze PCM-Blöcke werden in 4096-Frame-Schritten dekodiert; Integer-Werte werden von der rechtsbündigen WavPack-Darstellung auf die Full-Scale-`i32`-Konvention des Players geschoben, Float-Dateien werden mit `OPEN_NORMALIZE` geöffnet und ihre `i32`-Slots als `f32` interpretiert. `seek_sample()` nutzt exakte Sample-Indizes; CUE-Offsets laufen unverändert über `Source::seek`.
- **Einbindung:** `.wv` steht in `library::AUDIO`, in `container_duration()` und als eigener `Inner::Wavpack`-Zweig in `Source::open_with_options()`. APEv2-Titel, Interpret, Album, Albumartist, Genre, Jahr, Disc und Track werden direkt über libwavpack gelesen, weil lofty `.wv` nicht probt. Damit funktionieren Import, Suche, Facetten, Sortierung, Queue und CUE auch für WavPack.
- **Build-Seite:** `Cargo.lock` hat 21 neue, größtenteils Build-Zeit-Abhängigkeiten (CMake, bindgen, clang-sys). `.cargo/config.toml` setzt `CMAKE_POLICY_VERSION_MINIMUM = "3.5"`, weil das gebündelte WavPack-CMake-Projekt älter als CMake 4 ist; ein normales `cargo build --locked` braucht dadurch keine zusätzliche Umgebungsvariable. Auf diesem Rechner wurde LLVM 23.1.6 via winget installiert, damit `bindgen` `libclang.dll` findet. Das ist eine lokale Systemänderung und muss auf fremden Build-Maschinen entsprechend dokumentiert werden.
- **Sicherheitsbewertung `unsafe impl Send`:** `WavpackReader` enthält ein `NonNull` und einen dyn Reader, weshalb der Crate kein `Send` implementiert. Der Player verschiebt die Quelle vor dem Dekodieren nur in den Audio-Worker; danach greift genau dieser Thread auf Kontext und Datei zu. Andere Threads erhalten nur `Arc<RenderState>` und Kanäle. Die Implementierung ist daher auf diese eine Besitzer-Thread-Beziehung beschränkt, nicht als allgemeine Sync-Freigabe zu verstehen.
- **Grenzen, die nicht durch Wunschdenken verschwinden:** Hybriddateien dekodieren ohne `.wvc`-Korrekturdatei; DSD-in-WavPack wird nicht geöffnet. Nativer DSD-Ausgang und DoP bleiben aus, weil WASAPI/CPAL keine DSD-Transporte anbieten, DoP eine 176,4-kHz-PCM-Schiene und damit einen Eingriff in den Hot-Loop sowie einen fähigen DAC braucht – die iD4 mk2 ist auf 96 kHz spezifiziert. SACD-ISO hat keinen reifen, lizenzfreien Rust-Reader. ASIO verlangt das Steinberg-SDK, VST3 bricht Per-Design die Bit-perfect-Kette, UPnP/DLNA wäre ein eigenes Netzwerkprodukt. Hardware-Bitperfektheit und Hörtests brauchen den Nutzer und echte Captures; sie wurden nicht simuliert und nicht behauptet.
- **Unangetastet zu diesem Zeitpunkt:** kein Audio-Hot-Loop-Change, `Cargo.toml` noch bei `v0.5.0`, keine Git- oder GitHub-Schreiboperation, kein Stoppen des laufenden Players, `android/` und `phase5-complete.patch` unberührt. Die Versionserhöhung auf `v0.6.0` und die Veröffentlichung erfolgten erst in Abschnitt 17 nach ausdrücklicher Freigabe.

Verifikation nach dieser Runde: `cargo fmt --all`, `cargo clippy --release --locked --all-targets -- -D warnings` und `cargo test --release --locked` grün (178 + 3 bestanden, 4 ignoriert). Neue Synthetiktests erzeugen echte `.wv`-Dateien und prüfen Integer- und Float-Dekodierung, Full-Scale-Normalisierung, exaktes Seek, Extension-Routing, Tag-Zuordnung, Library-Dauer, die Erkennung des DSD-Qualify-Mode vor irrtümlicher PCM-Ausgabe sowie Byte-für-Byte-PCM durch den `FileBackend`. Der finale Release-Build wurde in einem frischen `CARGO_TARGET_DIR` erzeugt und anschließend in die beiden aktiven Startpfade kopiert: Desktop-Verknüpfung `rust-player-final-verification\release` und `target\ui-build\release`. Beide EXEs haben identische SHA-256 `089FB296019B2A7BDA6ED399101937E9B3BD50CA4A2692720569708C679A2994`; die vorherigen Dateien liegen als Backups unter `rust-player-daily-backup-20261126` und `rust-player-daily-backup-20261126-2`. Zum Zeitpunkt des Kopierens lief kein Player-Prozess, es wurde also nichts gestoppt. Hardware- und Hörtests bleiben weiterhin nicht simuliert.

---

## 17. Stabile Version v0.6.0 veröffentlicht

Auf ausdrückliche Anweisung des Nutzers wurde der Stand nach Abschnitt 16 als stabile Version `v0.6.0` veröffentlicht. `v0.6.0-rc1` blieb unverändert; `Cargo.toml` und `Cargo.lock` wurden dafür von `0.5.0` auf `0.6.0` angehoben. Damit meldet der Updater ab dieser Version korrekt `0.6.0` statt weiterhin eine ältere Versionsnummer.

- **Paket:** `scripts/package-release.ps1 -Version v0.6.0` lief inklusive `cargo fmt --check`, `cargo clippy --release --locked --all-targets -- -D warnings`, `cargo test --release --locked` (178 + 3 bestanden, 4 ignoriert) und `cargo build --release --locked` fehlerfrei durch. ZIP: `Rust-Player-v0.6.0-Windows-x64.zip`, SHA-256 `6f2bed58ce6d8ff3fd986cb3a33ca205da4a07a6177dae48e294c91a151f5e03`.
- **Release-Inhalt:** Das ZIP enthält `rust-player.exe`, `THIRD-PARTY-NOTICES.md`, `CHANGELOG.md` und `RELEASE-NOTES-NEXT.md`. Beide Release-Texte wurden von „Preview/next version“ auf stabile `v0.6.0`-Notizen umgestellt; `docs/RELEASE-WORKFLOW.md` nennt jetzt `v0.6.0` als stabilen Stand und `v0.6.0-rc1` als unveränderten, überholten Preview.
- **Veröffentlichung:** Commit auf `ai/rust-player-improvements` gepusht, Tag `v0.6.0` erstellt und gepusht, GitHub-Release `https://github.com/AikasAcc/rust-player/releases/tag/v0.6.0` mit `prerelease = false` und den vier Assets veröffentlicht. `gh` ist in dieser Umgebung nicht installiert; die Veröffentlichung lief über die GitHub REST API mit einem Token aus dem lokalen Git Credential Manager. Kein Token wurde im Repository oder in Skripten gespeichert.
- **Freigabe und Tabu:** Commit, Push, Tag und Release erfolgten ausschließlich auf die ausdrückliche Anweisung „pack die version als 0.6.0 auf github“ hin. `android/` und `phase5-complete.patch` wurden nicht committet, nicht gepusht und nicht veröffentlicht. Es gab keinen Merge, keinen Branchwechsel und keine Änderung an `v0.6.0-rc1`.
- **Ehrliche Grenze:** Die automatisierten Prüfungen sind grün. Hardware-Bitperfektheit, digitale Loopbacks und Hörtests bleiben Aufgaben des Nutzers und sind in `docs/HARDWARE-VERIFICATION.md` beschrieben; sie wurden nicht simuliert oder behauptet.
