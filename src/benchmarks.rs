use super::*;
use std::hint::black_box;

#[test]
#[ignore = "Run individually with --ignored --nocapture --test-threads=1"]
fn library_search_scale() {
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
        println!("build,{n},{}", start.elapsed().as_micros());
        for _ in 0..7 {
            let start = Instant::now();
            let result = SearchResults::new(0, black_box("not present"), &list, Some(&view));
            assert_eq!(result.count, 0);
            println!("search,{n},{}", start.elapsed().as_micros());
        }
    }
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
    crate::metadata_cache::measure(&paths, &dir.path().join("cache.json"));
}
