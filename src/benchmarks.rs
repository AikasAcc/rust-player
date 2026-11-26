use super::*;
use std::hint::black_box;
use std::time::{SystemTime, UNIX_EPOCH};

struct Row {
    scenario: &'static str,
    size: usize,
    micros: u128,
}

fn record(rows: &mut Vec<Row>, scenario: &'static str, size: usize, micros: u128) {
    println!("{scenario},{size},{micros}");
    rows.push(Row {
        scenario,
        size,
        micros,
    });
}

/// Machine-readable raw data for docs/BENCHMARK-REPORT.md.
fn write_csv(name: &str, rows: &[Row]) {
    let _ = std::fs::create_dir_all("target");
    let mut csv = String::from("scenario,size,micros\n");
    for row in rows {
        csv.push_str(&format!("{},{},{}\n", row.scenario, row.size, row.micros));
    }
    std::fs::write(format!("target/benchmark-{name}.csv"), csv).expect("write benchmark CSV");
}

#[derive(Default)]
struct ProcessUsage {
    user_micros: u128,
    kernel_micros: u128,
    page_faults: u32,
    peak_working_set: usize,
}

#[cfg(windows)]
fn process_usage() -> ProcessUsage {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    fn ticks(time: FILETIME) -> u128 {
        ((u128::from(time.dwHighDateTime) << 32) | u128::from(time.dwLowDateTime)) / 10
    }

    let process = unsafe { GetCurrentProcess() };
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let (user_micros, kernel_micros) =
        if unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) }
            .is_ok()
        {
            (ticks(user), ticks(kernel))
        } else {
            (0, 0)
        };
    let mut memory = PROCESS_MEMORY_COUNTERS {
        cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    let (page_faults, peak_working_set) =
        if unsafe { GetProcessMemoryInfo(process, &mut memory, memory.cb) }.is_ok() {
            (memory.PageFaultCount, memory.PeakWorkingSetSize)
        } else {
            (0, 0)
        };
    ProcessUsage {
        user_micros,
        kernel_micros,
        page_faults,
        peak_working_set,
    }
}

#[cfg(not(windows))]
fn process_usage() -> ProcessUsage {
    ProcessUsage::default()
}

/// Wall/CPU/memory context for one ignored benchmark process. The values are
/// process-wide, so they describe this benchmark run and nothing else.
struct BenchRun {
    name: &'static str,
    started_unix_secs: u64,
    started: Instant,
}

impl BenchRun {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            started_unix_secs: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            started: Instant::now(),
        }
    }

    fn finish(self, rows: &[Row]) {
        let wall_micros = self.started.elapsed().as_micros();
        let usage = process_usage();
        write_csv(self.name, rows);
        let meta = format!(
            "key,value\n\
             started_unix_secs,{}\n\
             wall_micros,{}\n\
             cpu_user_micros,{}\n\
             cpu_kernel_micros,{}\n\
             page_faults,{}\n\
             peak_working_set_bytes,{}\n\
             profile,release\n",
            self.started_unix_secs,
            wall_micros,
            usage.user_micros,
            usage.kernel_micros,
            usage.page_faults,
            usage.peak_working_set,
        );
        std::fs::write(format!("target/benchmark-{}.meta.csv", self.name), meta)
            .expect("write benchmark metadata CSV");
    }
}

#[test]
#[ignore = "Run individually with --ignored --nocapture --test-threads=1"]
fn library_search_scale() {
    let run = BenchRun::new("search");
    let mut rows = Vec::new();
    for n in [1000, 10000, 50000, 100000] {
        let mut list = Library::default().playlists.remove(0);
        let mut tracks: Vec<_> = (0..n)
            .map(|i| {
                list.tracks.push(PathBuf::from(format!(
                    "Music/Album {}/Track {i}.flac",
                    i / 12
                )));
                TrackInfo {
                    title: format!("Track {i}"),
                    artist: format!("Artist {}", i % 100),
                    album: format!("Album {}", i / 12),
                    album_artist: format!("Artist {}", i % 100),
                    ..Default::default()
                }
            })
            .collect();
        let start = Instant::now();
        for (p, t) in list.tracks.iter().zip(&mut tracks) {
            t.prepare_search(p);
        }
        let view = PlaylistView::build(tracks, Sort::Album, false);
        record(&mut rows, "build", n, start.elapsed().as_micros());
        for _ in 0..7 {
            let start = Instant::now();
            let result = SearchResults::new(0, black_box("not present"), &list, Some(&view));
            assert_eq!(result.count, 0);
            record(&mut rows, "search", n, start.elapsed().as_micros());
        }
    }
    run.finish(&rows);
}

fn save_bench_library(n: usize) -> Library {
    let mut library = Library::default();
    let mut playlist = library.playlists.remove(0);
    for i in 0..n {
        playlist.tracks.push(PathBuf::from(format!(
            "Music/Album {}/Track {i}.flac",
            i / 12
        )));
        playlist.ids.push(TrackId(i as u64 + 1));
    }
    library.playlists.push(playlist);
    library
}

/// Cost breakdown for the persistence path that Audit 6 wants to move off the
/// UI thread: serialize, atomic write (fsync + persist), and both together.
#[test]
#[ignore = "Run individually with --ignored --nocapture --test-threads=1"]
fn library_save_scale() {
    let run = BenchRun::new("save");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.json");
    let mut rows = Vec::new();
    for n in [1000, 10000, 50000, 100000] {
        let library = save_bench_library(n);
        let bytes = serde_json::to_vec(&library).unwrap();
        for _ in 0..3 {
            let start = Instant::now();
            let snapshot = serde_json::to_vec(black_box(&library)).unwrap();
            record(&mut rows, "serialize", n, start.elapsed().as_micros());
            black_box(snapshot);

            let start = Instant::now();
            library::save_bytes(black_box(&bytes), &path).unwrap();
            record(&mut rows, "write_only", n, start.elapsed().as_micros());

            let start = Instant::now();
            library::save_json(black_box(&library), &path).unwrap();
            record(&mut rows, "save_json", n, start.elapsed().as_micros());
        }

        // New UI path: serialize and queue; the worker's fsync is not counted.
        let mut worker = save_worker::SaveWorker::new();
        let worker_path = dir.path().join("worker.json");
        for _ in 0..3 {
            let start = Instant::now();
            let bytes = serde_json::to_vec(black_box(&library)).unwrap();
            worker.enqueue(worker_path.clone(), bytes);
            record(&mut rows, "ui_path", n, start.elapsed().as_micros());
        }
        worker.shutdown();
    }
    run.finish(&rows);
}

#[test]
#[ignore = "Creates 1000 temporary PCM WAV files"]
fn tag_scan() {
    let run = BenchRun::new("tag_scan");
    let dir = tempfile::tempdir().unwrap();
    let seed = dir.path().join("seed.wav");
    crate::audio::setup::tone(&seed, 48000).unwrap();
    let paths: Vec<_> = (0..1000)
        .map(|i| {
            let p = dir.path().join(format!("{i}.wav"));
            std::fs::hard_link(&seed, &p).unwrap();
            p
        })
        .collect();
    let micros = crate::metadata_cache::measure(&paths, &dir.path().join("cache.json"));
    let rows: Vec<_> = micros
        .into_iter()
        .map(|micros| Row {
            scenario: "tag_scan",
            size: 1000,
            micros,
        })
        .collect();
    for row in &rows {
        println!("{},{},{}", row.scenario, row.size, row.micros);
    }
    run.finish(&rows);
}

/// UI-thread cost of a track change: the old path called `Media::read`
/// synchronously, the new path shows `Media::placeholder` and reads tags on a
/// worker. Both run against the same tagged WAV so the delta is the tag parse.
#[test]
#[ignore = "Run individually with --ignored --nocapture --test-threads=1"]
fn media_read_scale() {
    let run = BenchRun::new("media");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("track.wav");
    crate::audio::setup::tone(&path, 48000).unwrap();
    let mut rows = Vec::new();
    for (scenario, full) in [("media_placeholder", false), ("media_read", true)] {
        let started = Instant::now();
        for _ in 0..1000 {
            let media = if full {
                Media::read(&path)
            } else {
                Media::placeholder(&path)
            };
            black_box(media.title);
        }
        let micros = started.elapsed().as_micros();
        println!("{scenario},1000,{micros}");
        rows.push(Row {
            scenario,
            size: 1000,
            micros,
        });
    }
    run.finish(&rows);
}
