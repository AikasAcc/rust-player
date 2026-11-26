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

- **Audio formats:** MP3, FLAC, WAV, Ogg Vorbis, AAC, M4A/ALAC, AIFF, CAF, PCM, MP1/MP2, ADPCM, MKA/MKV; external CUE sheets with multiple `FILE` entries and embedded `CUESHEET` text in FLAC (`src/library.rs:42-43`, `src/audio/decoder.rs:3-16,60-78`, `src/cue.rs`). WavPack/Monkey/DSD are not included (consistent with the README).
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
- Background writer (`src/save_worker.rs`) for `library.json`, `playback.json` and `listening.json`: the UI serializes, the worker performs `fsync`/`persist` and coalesces per file. `shutdown()` joins on exit, so no change is lost. Four tests for coalescing, ordering, separate targets and error messages.
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
