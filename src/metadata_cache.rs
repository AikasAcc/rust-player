use crate::library::{TrackInfo, save_json};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::SystemTime,
};

#[derive(PartialEq, Eq, Serialize, Deserialize)]
struct Stamp {
    modified: SystemTime,
    bytes: u64,
}
impl Stamp {
    fn read(p: &Path) -> Option<Self> {
        let m = fs::metadata(p).ok()?;
        Some(Self {
            modified: m.modified().ok()?,
            bytes: m.len(),
        })
    }
}
#[derive(Serialize, Deserialize)]
struct Entry {
    stamp: Stamp,
    info: TrackInfo,
}
#[derive(Default, Serialize, Deserialize)]
struct Cache(HashMap<PathBuf, Entry>);
impl Cache {
    fn load(file: &Path) -> Self {
        fs::File::open(file)
            .ok()
            .and_then(|f| serde_json::from_reader(std::io::BufReader::new(f)).ok())
            .unwrap_or_default()
    }

    fn scan(&mut self, paths: &[PathBuf]) -> (Vec<TrackInfo>, bool) {
        let workers = thread::available_parallelism()
            .map_or(1, usize::from)
            .min(4);
        let cache = &self.0;
        let chunks = thread::scope(|s| {
            paths
                .chunks(paths.len().div_ceil(workers).max(1))
                .map(|chunk| {
                    s.spawn(move || {
                        chunk
                            .iter()
                            .map(|p| {
                                let stamp = Stamp::read(p);
                                if let Some(entry) = cache.get(p)
                                    && Some(&entry.stamp) == stamp.as_ref()
                                {
                                    return (entry.info.clone(), None);
                                }
                                let mut info = TrackInfo::read(p);
                                // Virtual CUE entries depend on the sheet and its referenced audio.
                                info.missing = if let Some((sheet, _)) = crate::cue::reference(p) {
                                    !sheet.is_file()
                                        || crate::cue::resolve(p)
                                            .ok()
                                            .flatten()
                                            .is_none_or(|t| !t.audio.is_file())
                                } else {
                                    !p.is_file()
                                };
                                let update = stamp.filter(|_| !info.missing).map(|stamp| Entry {
                                    stamp,
                                    info: info.clone(),
                                });
                                (info, update)
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|j| j.join().expect("metadata worker"))
                .collect::<Vec<_>>()
        });
        let mut dirty = false;
        let tracks = paths
            .iter()
            .zip(chunks.into_iter().flatten())
            .map(|(p, (mut info, update))| {
                if let Some(entry) = update {
                    self.0.insert(p.clone(), entry);
                    dirty = true;
                } else if info.missing {
                    dirty |= self.0.remove(p).is_some();
                }
                info.prepare_search(p);
                info
            })
            .collect();
        (tracks, dirty)
    }
}
struct Job {
    paths: Vec<PathBuf>,
    reply: mpsc::Sender<Vec<TrackInfo>>,
    ctx: eframe::egui::Context,
}
pub struct Indexer(mpsc::Sender<Job>);
impl Indexer {
    pub fn new(file: PathBuf) -> Self {
        let (send, recv) = mpsc::channel::<Job>();
        thread::spawn(move || {
            let mut cache = Cache::load(&file);
            let mut dirty = false;
            for job in recv {
                let (tracks, changed) = cache.scan(&job.paths);
                dirty |= changed;
                let _ = job.reply.send(tracks);
                job.ctx.request_repaint();
                if dirty && save_json(&cache, &file).is_ok() {
                    dirty = false;
                }
            }
        });
        Self(send)
    }
    pub fn request(
        &self,
        paths: Vec<PathBuf>,
        ctx: &eframe::egui::Context,
    ) -> mpsc::Receiver<Vec<TrackInfo>> {
        let (reply, rx) = mpsc::channel();
        let _ = self.0.send(Job {
            paths,
            reply,
            ctx: ctx.clone(),
        });
        rx
    }
}
#[cfg(test)]
pub fn measure(paths: &[PathBuf], file: &Path) {
    for _ in 0..4 {
        let now = std::time::Instant::now();
        let mut c = Cache::load(file);
        let (r, changed) = c.scan(paths);
        if changed {
            save_json(&c, file).unwrap();
        }
        assert!(r.iter().all(|t| t.duration.is_some()));
        println!("scan,1000,{}", now.elapsed().as_micros());
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn corrupt_cache_rebuilds_and_validates_size_and_time() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("metadata-cache.json");
        fs::write(&file, "invalid").unwrap();
        let mut cache = Cache::load(&file);
        let p = tmp.path().join("track.wav");
        crate::audio::setup::tone(&p, 48000).unwrap();
        let paths = [p.clone()];
        assert!(cache.scan(&paths).1);
        save_json(&cache, &file).unwrap();
        let mut cache = Cache::load(&file);
        assert!(!cache.scan(&paths).1);
        cache.0.get_mut(&p).unwrap().stamp.modified = SystemTime::UNIX_EPOCH;
        assert!(cache.scan(&paths).1);
        fs::write(&p, b"short").unwrap();
        assert!(cache.scan(&paths).1);
        fs::remove_file(&p).unwrap();
        assert!(cache.scan(&paths).0[0].missing);
    }
}
