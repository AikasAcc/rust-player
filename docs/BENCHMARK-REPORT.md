# Benchmark report

Purpose: reproducible, machine-evaluable micro-measurements as the basis for the
UI-thread offload (audit item 6). **Limitation:** no real UI frame time is measured here.
The figures show compute time in the release build, not the perceived behavior of the interface.

## Environment

- Windows, AMD Ryzen 7 5700X (8 cores), 32 GiB RAM
- Rust 1.100.0 (pinned via `rust-toolchain.toml`), release profile from `Cargo.toml`
- Measurement in the test process, `black_box` for the search term, one thread

## Reproduction

```powershell
cargo test --release --locked -- --ignored --nocapture --test-threads=1 benchmarks::library_search_scale
cargo test --release --locked -- --ignored --nocapture --test-threads=1 benchmarks::tag_scan
cargo test --release --locked -- --ignored --nocapture --test-threads=1 benchmarks::library_save_scale
cargo test --release --locked -- --ignored --nocapture --test-threads=1 benchmarks::media_read_scale
```

Each run overwrites its raw data as CSV:

- `target/benchmark-search.csv`
- `target/benchmark-tag_scan.csv`
- `target/benchmark-save.csv`
- `target/benchmark-media.csv`

Columns: `scenario,size,micros`. Raw data from the latest run is in `target/benchmark-save.csv` and `target/benchmark-media.csv`.

## Library search and view construction

All times are in microseconds. The search was run seven times per size; the first
value includes additional cold-cache effects.

| Tracks | Build | Search 1 | Search 2–7 (range) |
|---:|---:|---:|---:|
| 1.000 | 798 | 27 | 12 |
| 10.000 | 7.949 | 248 | 125–139 |
| 50.000 | 44.650 | 1.187 | 514–797 |
| 100.000 | 90.453 | 2.288 | 1.625–2.634 |

Interpretation: construction scales approximately linearly. Even with 100.000
tracks, the search stays in the range of roughly 1,6–2,6 ms when warm. This is not a
reason to optimize the search now; it is the baseline for making a later offload measurable.

## Metadata scan (1.000 files)

Four runs over 1.000 hard-linked PCM WAV files:

| Run | Microseconds |
|---:|---:|
| 1 (cold) | 623.562 |
| 2 | 209.880 |
| 3 | 198.181 |
| 4 | 200.056 |

Interpretation: after the first run, the scan takes about 200 ms for 1.000 files.
The first run is significantly slower because files and cache are not warm yet.
This figure was the reason for running the metadata indexer asynchronously already.
The ReplayGain scan now also runs in the background; the figures here concern
the separate metadata path and are not an audio measurement.

## Persistence path — before and after the offload

`library_save_scale` measures three scenarios per size (three repetitions each):

- `save_json`: old path, serialization + `tempfile` + `fsync` + `persist` **in the calling thread**.
- `serialize`: only `serde_json::to_vec`.
- `ui_path`: serialization + enqueuing in the background writer — what the UI thread still does today. The worker's `fsync` work is **not** counted here.

Baseline (before the offload, first run, warm):

| Tracks | `serialize` | `save_json` (UI thread) |
|---:|---:|---:|
| 1.000 | 49–67 µs | 1,39–1,87 ms |
| 10.000 | 354–491 µs | 2,02–2,84 ms |
| 50.000 | 1,63–1,69 ms | 4,45–4,75 ms |
| 100.000 | 3,79–4,45 ms | 8,25–8,42 ms |

After the offload (latest run, raw data in `target/benchmark-save.csv`):

| Tracks | `ui_path` | `save_json` (comparison in the same run) |
|---:|---:|---:|
| 1.000 | 38–80 µs | 2,03–4,59 ms |
| 10.000 | 392–577 µs | 2,59–7,87 ms |
| 50.000 | 2,41–2,67 ms | 6,49–7,92 ms |
| 100.000 | 4,84–5,41 ms | 11,19–20,33 ms |

**Important context:** absolute disk latency varies greatly between runs
(one intermediate run was 42–89 ms for `save_json` at 100.000 tracks). The cause is
likely antivirus scanning and cache state, not the code. Only values from the same
run are comparable. Within a run, the UI-thread overhead dropped by about a factor
of 8–15 at 100.000 tracks and by more than a factor of 20 at 1.000 tracks.

Interpretation: serialization inevitably remains in the UI thread because it reads
the current state. The expensive and unpredictable part (`fsync`/`persist`) now runs
in the worker. In addition, the worker coalesces multiple snapshots of the same file —
with rapid settings changes, only the latest state reaches disk.

## Track changes — before and after the offload

`media_read_scale` runs 1.000 times against the same tagged PCM WAV file (raw data in
`target/benchmark-media.csv`):

| Scenario | 1.000 runs | per call |
|---|---:|---:|
| `media_placeholder` (used in the UI thread today) | 162 µs | ~0,16 µs |
| `media_read` (formerly synchronous in the UI thread, now a worker) | 165.429 µs | ~165 µs |

Interpretation: tag and cover parsing previously cost around 165 µs **per
track change on the UI thread** — higher for a FLAC with an embedded cover or a
large CUE file. The UI thread now only sets a track snapshot and enqueues the full
read operation on an `mpsc` channel. `finish_media_job()` takes the result, cover
and lyrics over on the UI thread and discards results via a generation if the user
has already selected the next track in the meantime. The properties dialog uses
the same pattern via `property_jobs`, so opening track properties no longer waits
for the tag read.

## Open measurement points

- UI frame time under load (egui), not measured yet. The figures above are compute time, not frame time.
- Playlist file import/export: offloaded via the `mpsc` pattern, but not benchmarked separately (file sizes and CUE share vary too much).
- Memory and CPU profiling during the scan, not measured yet.
- Audio pump loop: no benchmark. Removing `Mutex` + `Next` clone per iteration is only logically tested (`queue_changes_are_observed_once_without_polling_the_mutex`); a listening test is outstanding.
