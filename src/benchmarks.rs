use super::*;
use std::hint::black_box;

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

#[test]
#[ignore = "Run individually with --ignored --nocapture --test-threads=1"]
fn library_search_scale() {
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
    write_csv("search", &rows);
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
    write_csv("save", &rows);
}

#[test]
#[ignore = "Creates 1000 temporary PCM WAV files"]
fn tag_scan() {
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
    write_csv("tag_scan", &rows);
}

/// UI-thread cost of a track change: the old path called `Media::read`
/// synchronously, the new path shows `Media::placeholder` and reads tags on a
/// worker. Both run against the same tagged WAV so the delta is the tag parse.
#[test]
#[ignore = "Run individually with --ignored --nocapture --test-threads=1"]
fn media_read_scale() {
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
    write_csv("media", &rows);
}
