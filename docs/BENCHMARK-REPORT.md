# Benchmark report

Purpose: reproducible, machine-evaluable micro-measurements as the basis for the
UI-thread offload (audit item 6). **Limitation:** these are compute times in release
test processes. They are neither UI frame times nor audio measurements.

## Environment

- Windows 11 Pro, AMD Ryzen 7 5700X (8 physical / 16 logical cores), 31.93 GiB RAM
- Rust 1.100.0 (pinned via `rust-toolchain.toml`), release profile from `Cargo.toml`
- One ignored test per process, `--test-threads=1`; inputs pass through `black_box`
- Run `2026-11-26_072551`, commit `f0e05ad...` with a dirty worktree (the
  uncommitted TrackId/benchmark changes). The exact file list and tool versions are
  in `environment.txt` inside the archived run.

## Reproduction

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/run-benchmarks.ps1
```

The script runs all four ignored benchmarks and archives the result under
`target/benchmark-runs/<timestamp>/`:

- `environment.txt`: timestamp, working directory, rustc/cargo versions, commit,
  dirty-file count, CPU, core counts, RAM, OS, profile and thread count
- `<benchmark>.out.log` / `<benchmark>.err.log`: complete test output
- `benchmark-*.csv`: raw rows, columns `scenario,size,micros`
- `benchmark-*.meta.csv`: process context as `key,value` — wall time, CPU user and
  kernel time (Windows `GetProcessTimes`), page faults, peak working set
  (`GetProcessMemoryInfo`), start time and profile

Manual equivalent, if the script is not available:

```powershell
cargo test --release --locked -- --ignored --nocapture --test-threads=1 benchmarks::library_search_scale
cargo test --release --locked -- --ignored --nocapture --test-threads=1 benchmarks::tag_scan
cargo test --release --locked -- --ignored --nocapture --test-threads=1 benchmarks::library_save_scale
cargo test --release --locked -- --ignored --nocapture --test-threads=1 benchmarks::media_read_scale
```

## Process-level context of the latest run

| Benchmark | Wall | CPU user | CPU kernel | Page faults | Peak working set |
|---|---:|---:|---:|---:|---:|
| `library_search_scale` | 0.243 s | 0.219 s | 0.047 s | 32,214 | 66.1 MiB |
| `tag_scan` | 1.519 s | 0.719 s | 1.266 s | 5,622 | 10.9 MiB |
| `library_save_scale` | 0.196 s | 0.063 s | 0.047 s | 36,234 | 28.0 MiB |
| `media_read_scale` | 0.168 s | 0.016 s | 0.156 s | 1,546 | 5.8 MiB |

CPU times are process-wide and quantized to roughly 15.6 ms by Windows; they can
exceed wall time when a benchmark uses worker threads (`tag_scan`). The working set
is the process high-water mark, not a per-scenario allocation. These values provide
context for outliers; the comparable numbers are the microsecond rows below.

## Library search and view construction

All times are in microseconds. The search ran seven times per size; the first value
includes cold-cache effects.

| Tracks | Build | Search 1 | Searches 2–7 (range) |
|---:|---:|---:|---:|
| 1,000 | 628 | 19 | 10–11 |
| 10,000 | 7,429 | 402 | 105–190 |
| 50,000 | 41,419 | 991 | 514–944 |
| 100,000 | 86,071 | 2,094 | 1,914–2,211 |

Interpretation: construction scales approximately linearly. Even with 100,000
tracks, a warm search stays around 2 ms. This is a baseline for measuring a later
offload, not a reason to optimize the search now.

## Metadata scan (1,000 files)

Four passes over 1,000 hard-linked PCM WAV files:

| Pass | Microseconds |
|---:|---:|
| 1 (cold) | 603,025 |
| 2 | 191,309 |
| 3 | 194,258 |
| 4 | 194,824 |

Interpretation: after the first pass, the scan takes about 194 ms for 1,000 files.
The cold pass is dominated by file and cache warming. This was the reason for moving
the metadata indexer to a worker; ReplayGain scanning is now background work too.
These figures concern metadata, not audio decoding.

## Persistence path — before and after the offload

`library_save_scale` measures three scenarios per size (three repetitions each):

- `save_json`: old path, serialization + `tempfile` + `fsync` + `persist` **in the calling thread**.
- `serialize`: only `serde_json::to_vec`.
- `ui_path`: serialization + enqueuing in the background writer — what the UI thread still does today. The worker's `fsync` work is **not** counted here.

Baseline (before the offload, warm):

| Tracks | `serialize` | `save_json` (UI thread) |
|---:|---:|---:|
| 1,000 | 49–67 µs | 1.39–1.87 ms |
| 10,000 | 354–491 µs | 2.02–2.84 ms |
| 50,000 | 1.63–1.69 ms | 4.45–4.75 ms |
| 100,000 | 3.79–4.45 ms | 8.25–8.42 ms |

Latest run (raw data in the archived `benchmark-save.csv`):

| Tracks | `ui_path` | `serialize` | `save_json` (comparison in the same run) |
|---:|---:|---:|---:|
| 1,000 | 47–67 µs | 43–76 µs | 1.42–1.53 ms |
| 10,000 | 315–438 µs | 316–434 µs | 2.12–2.23 ms |
| 50,000 | 1.85–2.12 ms | 1.99–2.09 ms | 4.97–5.43 ms |
| 100,000 | 4.87–5.07 ms | 4.30–4.89 ms | 8.99–15.14 ms |

**Important context:** absolute disk latency varies greatly between runs (the
100,000-track `save_json` line includes a 15.14 ms outlier here). The cause is
likely antivirus scanning and cache state, not the code. Only values from the same
run are comparable. Within this run, UI-thread overhead at 100,000 tracks dropped by
roughly a factor of 2 versus `save_json`; earlier runs showed a factor of 8–15 because
disk latency was higher. The invariant result is that serialization remains and the
unpredictable `fsync`/`persist` work moved to the worker, which also coalesces
snapshots per path.

## Track changes — before and after the offload

`media_read_scale` runs 1,000 times against the same tagged PCM WAV file (raw data in
the archived `benchmark-media.csv`):

| Scenario | 1,000 runs | per call |
|---|---:|---:|
| `media_placeholder` (used in the UI thread today) | 224 µs | ~0.22 µs |
| `media_read` (formerly synchronous in the UI thread, now a worker) | 165,396 µs | ~165 µs |

Interpretation: tag and cover parsing previously cost around 165 µs **per track
change on the UI thread** — higher for a FLAC with an embedded cover or a large CUE
file. The UI thread now only sets a track snapshot and enqueues the full read
operation on an `mpsc` channel. `finish_media_job()` takes the result, cover and
lyrics over on the UI thread and discards results via a generation if the user has
already selected the next track in the meantime. The properties dialog uses the
same pattern via `property_jobs`.

## UI frame time (opt-in)

The app can log one row per `eframe::App::update` call:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/run-frame-log.ps1
```

The script starts the release build with `RUST_PLAYER_FRAME_LOG` set, waits until
the player is closed, then archives `frames.csv`, `environment.txt` and a
`summary.txt` under `target/frame-log-runs/<timestamp>/`. It refuses to start when
a player is already running; it never stops one. The CSV contains `frame,micros`;
buffers are flushed every 120 frames and on exit. Without the environment variable
nothing is opened or formatted. No frame log was recorded for this report: it
requires an interactive session with a real library. Use the script while
reproducing a UI stall and compare p50/p95/p99 against the compute numbers above.

## Open measurement points

- UI frame time under load: the logger and session script exist, but no interactive session was run; no data in this report.
- Playlist file import/export: offloaded via `mpsc`, but not benchmarked separately (file sizes and CUE share vary too much).
- CPU/memory: now captured per benchmark process; symbol-level profiling is still outstanding. `wpr.exe` is present but cannot enable the system-performance policy on this account (error `0xc5585011`), and the Windows `Process` counter set returns no data here, so no new trace could be recorded.
- Audio pump loop: no benchmark. Removing `Mutex` + `Next` clone per iteration is only logically tested (`queue_changes_are_observed_once_without_polling_the_mutex`); a listening test is outstanding.
