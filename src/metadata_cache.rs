use crate::library::{Library, TrackInfo, save_json};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs, io,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::SystemTime,
};

const CACHE_VERSION: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct CueStamp {
    sheet_modified: SystemTime,
    sheet_bytes: u64,
    audio_modified: SystemTime,
    audio_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct Stamp {
    modified: SystemTime,
    bytes: u64,
    cue: Option<CueStamp>,
}

impl Default for Stamp {
    fn default() -> Self {
        Self {
            modified: SystemTime::UNIX_EPOCH,
            bytes: 0,
            cue: None,
        }
    }
}

impl Stamp {
    fn read(p: &Path) -> Option<Self> {
        if let Some((sheet, _)) = crate::cue::reference(p) {
            let track = crate::cue::resolve(p).ok().flatten()?;
            let sheet_meta = fs::metadata(sheet).ok()?;
            let audio_meta = fs::metadata(&track.audio).ok()?;
            let modified = sheet_meta.modified().ok()?;
            let audio_modified = audio_meta.modified().ok()?;
            return Some(Self {
                modified,
                bytes: sheet_meta.len(),
                cue: Some(CueStamp {
                    sheet_modified: modified,
                    sheet_bytes: sheet_meta.len(),
                    audio_modified,
                    audio_bytes: audio_meta.len(),
                }),
            });
        }
        let meta = fs::metadata(p).ok()?;
        Some(Self {
            modified: meta.modified().ok()?,
            bytes: meta.len(),
            cue: None,
        })
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Entry {
    stamp: Stamp,
    info: TrackInfo,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Cache {
    version: u32,
    entries: HashMap<PathBuf, Entry>,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            version: CACHE_VERSION,
            entries: HashMap::new(),
        }
    }
}

impl Cache {
    fn invalid(file: &Path, message: impl std::fmt::Display) -> io::Error {
        let detail = match Library::back_up_invalid_json(file, "metadata-cache.invalid-") {
            Ok(backup) => format!("backup at {}", backup.display()),
            Err(error) => format!("backup failed: {error}"),
        };
        io::Error::other(format!("{message}; {detail}"))
    }

    fn load(file: &Path) -> io::Result<(Self, bool)> {
        let bytes = match fs::read(file) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok((Self::default(), false));
            }
            Err(error) => return Err(error),
        };
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|error| Self::invalid(file, error))?;
        if value
            .get("version")
            .and_then(serde_json::Value::as_u64)
            .is_some()
        {
            let cache: Self =
                serde_json::from_value(value).map_err(|error| Self::invalid(file, error))?;
            if cache.version > CACHE_VERSION {
                return Err(Self::invalid(
                    file,
                    format!("unsupported cache version {}", cache.version),
                ));
            }
            let upgraded = cache.version != CACHE_VERSION;
            return Ok((
                Self {
                    version: CACHE_VERSION,
                    ..cache
                },
                upgraded,
            ));
        }
        let entries: HashMap<PathBuf, Entry> =
            serde_json::from_value(value).map_err(|error| Self::invalid(file, error))?;
        Ok((
            Self {
                version: CACHE_VERSION,
                entries,
            },
            true,
        ))
    }

    fn scan(&mut self, paths: &[PathBuf]) -> (Vec<TrackInfo>, bool) {
        let workers = thread::available_parallelism()
            .map_or(1, usize::from)
            .min(4);
        let cache = &self.entries;
        let mut worker_failed = false;
        let chunks = thread::scope(|s| {
            paths
                .chunks(paths.len().div_ceil(workers).max(1))
                .map(|chunk| {
                    let len = chunk.len();
                    let worker = s.spawn(move || {
                        chunk
                            .iter()
                            .map(|p| {
                                let stamp = Stamp::read(p);
                                if let Some(entry) = cache.get(p)
                                    && Some(&entry.stamp) == stamp.as_ref()
                                {
                                    return (entry.info.clone(), None);
                                }
                                if stamp.is_none()
                                    && let Some(entry) = cache.get(p)
                                {
                                    let mut info = entry.info.clone();
                                    info.missing = true;
                                    return (info, None);
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
                    });
                    (len, worker)
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|(len, worker)| match worker.join() {
                    Ok(result) => result,
                    Err(_) => {
                        worker_failed = true;
                        (0..len)
                            .map(|_| (TrackInfo::default(), None::<Entry>))
                            .collect()
                    }
                })
                .collect::<Vec<_>>()
        });
        if worker_failed {
            crate::diagnostics::append_error(
                "metadata-cache",
                "A metadata scan worker stopped unexpectedly.",
            );
        }
        let mut dirty = false;
        let tracks = paths
            .iter()
            .zip(chunks.into_iter().flatten())
            .map(|(p, (mut info, update))| {
                if let Some(entry) = update {
                    self.entries.insert(p.clone(), entry);
                    dirty = true;
                }
                info.prepare_search(p);
                info
            })
            .collect();
        (tracks, dirty)
    }

    fn prune(&mut self, keep: &HashSet<PathBuf>) -> bool {
        let before = self.entries.len();
        self.entries.retain(|path, _| keep.contains(path));
        before != self.entries.len()
    }
}

/// `save()` sends the same path set on many state changes; rebuilding the
/// HashSet and retaining every entry on each of those frames is pure overhead.
fn prune_if_changed(
    cache: &mut Cache,
    last_keep: &mut Option<HashSet<PathBuf>>,
    paths: Vec<PathBuf>,
) -> bool {
    let keep: HashSet<PathBuf> = paths.into_iter().collect();
    if last_keep.as_ref() == Some(&keep) {
        return false;
    }
    let changed = cache.prune(&keep);
    *last_keep = Some(keep);
    changed
}

enum Job {
    Scan {
        paths: Vec<PathBuf>,
        reply: mpsc::Sender<Vec<TrackInfo>>,
        ctx: eframe::egui::Context,
    },
    Paths(Vec<PathBuf>),
}

pub struct Indexer(mpsc::Sender<Job>);
impl Indexer {
    pub fn new(file: PathBuf) -> Self {
        let (send, recv) = mpsc::channel::<Job>();
        thread::spawn(move || {
            let (mut cache, mut dirty) = Cache::load(&file).unwrap_or_else(|error| {
                crate::diagnostics::append_error(
                    "metadata-cache",
                    &format!("Could not load metadata cache: {error}"),
                );
                (Cache::default(), false)
            });
            let mut save_failed = false;
            let mut last_keep = None;
            for job in recv {
                match job {
                    Job::Scan { paths, reply, ctx } => {
                        let (tracks, changed) = cache.scan(&paths);
                        dirty |= changed;
                        let _ = reply.send(tracks);
                        ctx.request_repaint();
                    }
                    Job::Paths(paths) => {
                        dirty |= prune_if_changed(&mut cache, &mut last_keep, paths);
                    }
                }
                if dirty {
                    match save_json(&cache, &file) {
                        Ok(()) => {
                            dirty = false;
                            save_failed = false;
                        }
                        Err(error) if !save_failed => {
                            crate::diagnostics::append_error(
                                "metadata-cache",
                                &format!("Could not save metadata cache: {error}"),
                            );
                            save_failed = true;
                        }
                        Err(_) => {}
                    }
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
        let _ = self.0.send(Job::Scan {
            paths,
            reply,
            ctx: ctx.clone(),
        });
        rx
    }

    pub fn set_paths(&self, paths: Vec<PathBuf>) {
        let _ = self.0.send(Job::Paths(paths));
    }
}
#[cfg(test)]
pub fn measure(paths: &[PathBuf], file: &Path) -> Vec<u128> {
    let mut results = Vec::new();
    for _ in 0..4 {
        let now = std::time::Instant::now();
        let (mut c, _) = Cache::load(file).unwrap_or_default();
        let (r, changed) = c.scan(paths);
        if changed {
            assert!(save_json(&c, file).is_ok());
        }
        assert!(r.iter().all(|t| t.duration.is_some()));
        results.push(now.elapsed().as_micros());
    }
    results
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchanged_library_paths_are_not_pruned_twice() {
        let mut cache = Cache::default();
        let keep = PathBuf::from("keep.flac");
        let drop = PathBuf::from("drop.flac");
        cache.entries.insert(keep.clone(), Entry::default());
        cache.entries.insert(drop.clone(), Entry::default());
        let mut last = None;
        assert!(prune_if_changed(&mut cache, &mut last, vec![keep.clone()]));
        assert_eq!(cache.entries.len(), 1);
        assert!(!prune_if_changed(&mut cache, &mut last, vec![keep]));
        cache.entries.insert(drop, Entry::default());
        assert!(prune_if_changed(
            &mut cache,
            &mut last,
            vec![PathBuf::from("other.flac")]
        ));
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn corrupt_cache_is_backed_up_and_rebuilt() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("metadata-cache.json");
        let broken = b"invalid";
        fs::write(&file, broken).unwrap();

        let (cache, _) = Cache::load(&file).unwrap_or_default();
        assert!(cache.entries.is_empty());
        let backups: Vec<PathBuf> = fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|entry| {
                let entry = entry.unwrap();
                let name = entry.file_name().to_string_lossy().into_owned();
                name.starts_with("metadata-cache.invalid-")
                    .then(|| entry.path())
            })
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read(&backups[0]).unwrap(), broken);
    }

    #[test]
    fn partial_cache_json_uses_serde_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("metadata-cache.json");
        fs::write(&file, b"{}").unwrap();
        let (cache, _) = Cache::load(&file).unwrap();
        assert!(cache.entries.is_empty());
        assert_eq!(cache.version, CACHE_VERSION);
    }

    #[test]
    fn corrupt_cache_rebuilds_and_validates_size_and_time() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("metadata-cache.json");
        fs::write(&file, "invalid").unwrap();
        let (mut cache, _) = Cache::load(&file).unwrap_or_default();
        let p = tmp.path().join("track.wav");
        crate::audio::setup::tone(&p, 48000).unwrap();
        let paths = [p.clone()];
        assert!(cache.scan(&paths).1);
        save_json(&cache, &file).unwrap();
        let (mut cache, _) = Cache::load(&file).unwrap();
        assert!(!cache.scan(&paths).1);
        cache.entries.get_mut(&p).unwrap().stamp.modified = SystemTime::UNIX_EPOCH;
        assert!(cache.scan(&paths).1);
        fs::write(&p, b"short").unwrap();
        assert!(cache.scan(&paths).1);
        fs::remove_file(&p).unwrap();
        assert!(cache.scan(&paths).0[0].missing);
    }

    #[test]
    fn missing_cache_starts_empty_without_backup() -> io::Result<()> {
        let tmp = tempfile::tempdir()?;
        let file = tmp.path().join("metadata-cache.json");
        let (cache, dirty) = Cache::load(&file)?;
        assert!(cache.entries.is_empty());
        assert_eq!(cache.version, CACHE_VERSION);
        assert!(!dirty);
        assert_eq!(fs::read_dir(tmp.path())?.count(), 0);
        Ok(())
    }

    #[test]
    fn legacy_cache_entries_are_migrated() -> io::Result<()> {
        let tmp = tempfile::tempdir()?;
        let file = tmp.path().join("metadata-cache.json");
        let path = tmp.path().join("song.wav");
        crate::audio::setup::tone(&path, 48000)?;
        let mut legacy = HashMap::new();
        legacy.insert(path.clone(), Entry::default());
        save_json(&legacy, &file)?;

        let (cache, dirty) = Cache::load(&file)?;
        assert!(dirty);
        assert_eq!(cache.version, CACHE_VERSION);
        assert_eq!(cache.entries.len(), 1);
        assert!(cache.entries.contains_key(&path));
        Ok(())
    }

    #[test]
    fn future_cache_version_is_backed_up_and_rebuilt() -> io::Result<()> {
        let tmp = tempfile::tempdir()?;
        let file = tmp.path().join("metadata-cache.json");
        fs::write(&file, br#"{"version":999,"entries":{}}"#)?;

        let result = Cache::load(&file);
        assert!(result.is_err());
        let (cache, _) = result.unwrap_or_default();
        assert!(cache.entries.is_empty());
        let backups: Vec<PathBuf> = fs::read_dir(tmp.path())?
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let name = entry.file_name().to_string_lossy().into_owned();
                name.starts_with("metadata-cache.invalid-")
                    .then(|| entry.path())
            })
            .collect();
        assert_eq!(backups.len(), 1);
        Ok(())
    }

    #[test]
    fn cue_entries_cache_sheet_and_audio_stamps() -> io::Result<()> {
        let tmp = tempfile::tempdir()?;
        let sheet = tmp.path().join("disc.cue");
        let audio = tmp.path().join("whole.wav");
        crate::audio::setup::tone(&audio, 48000)?;
        let text = "TITLE \"Album\"\nPERFORMER \"Band\"\nFILE \"whole.wav\" WAVE\nTRACK 01 AUDIO\nTITLE \"First\"\nINDEX 01 00:00:00\n";
        fs::write(&sheet, text)?;
        let track = sheet.join("1");
        let paths = [track.clone()];
        let file = tmp.path().join("metadata-cache.json");

        let (mut cache, _) = Cache::load(&file).unwrap_or_default();
        assert!(cache.scan(&paths).1);
        assert_eq!(cache.entries[&track].info.title, "First");
        save_json(&cache, &file)?;

        let (mut cache, _) = Cache::load(&file)?;
        assert!(!cache.scan(&paths).1);
        assert!(cache.entries[&track].stamp.cue.is_some());

        fs::write(&sheet, text.replace("First", "Changed"))?;
        assert!(cache.scan(&paths).1);
        assert_eq!(cache.scan(&paths).0[0].title, "Changed");

        fs::write(&audio, b"modified")?;
        assert!(cache.scan(&paths).1);

        fs::remove_file(&audio)?;
        assert!(cache.scan(&paths).0[0].missing);
        Ok(())
    }

    #[test]
    fn prune_removes_only_paths_outside_library() {
        let mut cache = Cache::default();
        let keep_path = PathBuf::from("keep.wav");
        let drop_path = PathBuf::from("drop.wav");
        cache.entries.insert(keep_path.clone(), Entry::default());
        cache.entries.insert(drop_path.clone(), Entry::default());
        let keep = HashSet::from([keep_path.clone()]);

        assert!(cache.prune(&keep));
        assert!(cache.entries.contains_key(&keep_path));
        assert!(!cache.entries.contains_key(&drop_path));
        assert!(!cache.prune(&keep));
    }

    #[test]
    fn parallel_scan_preserves_input_order() -> io::Result<()> {
        let tmp = tempfile::tempdir()?;
        let paths: Vec<PathBuf> = (0..12)
            .map(|i| tmp.path().join(format!("track-{i:02}.wav")))
            .collect();
        for path in &paths {
            crate::audio::setup::tone(path, 48000)?;
        }
        let (mut cache, _) =
            Cache::load(&tmp.path().join("metadata-cache.json")).unwrap_or_default();
        let (tracks, changed) = cache.scan(&paths);

        assert!(changed);
        assert_eq!(tracks.len(), paths.len());
        for (track, path) in tracks.iter().zip(&paths) {
            assert_eq!(track.title, path.file_stem().unwrap().to_string_lossy());
            assert!(track.duration.is_some());
        }
        Ok(())
    }
}
