//! Single background writer for the small JSON state files.
//!
//! The UI thread only serializes a snapshot and sends the bytes here. This
//! thread owns the atomic write (`tempfile` + `fsync` + `persist`), so the
//! blocking disk work never stalls a frame. Several snapshots for the same
//! path are coalesced: only the newest one reaches the disk.

use crate::library::save_bytes;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread::JoinHandle,
};

struct Snapshot {
    path: PathBuf,
    bytes: Vec<u8>,
}

/// One completed write, reported in the same order the worker performed it.
/// Successes matter as much as failures: a pending backup of a corrupt file is
/// only retired once its replacement is actually on disk.
#[derive(Debug)]
pub struct SaveReport {
    pub path: PathBuf,
    pub error: Option<String>,
}

/// Tracks which corrupt state files still need a backup and which backups are
/// waiting for a confirmed write. Kept here so the state machine can be tested
/// without constructing the whole UI.
#[derive(Default)]
pub struct BackupTracker {
    needed: HashSet<PathBuf>,
    pending: HashSet<PathBuf>,
}

impl BackupTracker {
    /// Mark a file as corrupt on load. A backup must be made before it is
    /// overwritten.
    pub fn need(&mut self, path: PathBuf) {
        // A backup is already on its way for this file. Re-adding `needed`
        // here would make a later save back up the now-valid replacement.
        if !self.pending.contains(&path) {
            self.needed.insert(path);
        }
    }

    /// Whether the caller should copy the corrupt file now. Returns `false`
    /// while a backup is already pending, so repeated saves cannot pile up
    /// identical copies.
    pub fn should_back_up(&self, path: &Path) -> bool {
        self.needed.contains(path) && !self.pending.contains(path)
    }

    /// The backup copy exists; remember that a write is still outstanding.
    pub fn mark_backed_up(&mut self, path: PathBuf) {
        self.needed.remove(&path);
        self.pending.insert(path);
    }

    /// The replacement was written. The backup is no longer pending.
    pub fn written(&mut self, path: &Path) {
        self.pending.remove(path);
    }

    /// The write failed. The original is still corrupt on disk, so a fresh
    /// backup is required before the next attempt overwrites it.
    pub fn failed(&mut self, path: PathBuf) {
        if self.pending.remove(&path) {
            self.needed.insert(path);
        }
    }
}

pub struct SaveWorker {
    sender: Option<Sender<Snapshot>>,
    reports: Receiver<SaveReport>,
    handle: Option<JoinHandle<()>>,
}

impl Default for SaveWorker {
    fn default() -> Self {
        Self::new()
    }
}

impl SaveWorker {
    pub fn new() -> Self {
        let (sender, incoming) = mpsc::channel();
        let (reports_tx, reports) = mpsc::channel();
        let handle = std::thread::Builder::new()
            .name("rust-player-save".into())
            .spawn(move || {
                while let Ok(first) = incoming.recv() {
                    // Drain everything already queued and keep the newest
                    // snapshot per path. Old snapshots can never be newer
                    // than the state the UI thread holds now.
                    let mut pending: Vec<Snapshot> = vec![first];
                    // The map points into `pending`, so ordering is unchanged
                    // while duplicate lookups become O(1).
                    let mut index = HashMap::new();
                    index.insert(pending[0].path.clone(), 0usize);
                    while let Ok(next) = incoming.try_recv() {
                        if let Some(&position) = index.get(&next.path) {
                            pending[position].bytes = next.bytes;
                        } else {
                            index.insert(next.path.clone(), pending.len());
                            pending.push(next);
                        }
                    }
                    for snapshot in pending {
                        let error = match save_bytes(&snapshot.bytes, &snapshot.path) {
                            Ok(()) => None,
                            Err(error) => Some(format!(
                                "Could not save {}: {error}",
                                snapshot.path.display()
                            )),
                        };
                        let _ = reports_tx.send(SaveReport {
                            path: snapshot.path,
                            error,
                        });
                    }
                }
            })
            .expect("spawn save worker");
        Self {
            sender: Some(sender),
            reports,
            handle: Some(handle),
        }
    }

    /// Queue an atomic write. The bytes must already be a complete snapshot.
    pub fn enqueue(&self, path: PathBuf, bytes: Vec<u8>) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(Snapshot { path, bytes });
        }
    }

    /// Drain write outcomes reported since the last call, in order.
    pub fn take_reports(&self) -> Vec<SaveReport> {
        self.reports.try_iter().collect()
    }

    /// Finish everything still queued, then stop the thread. Returns the write
    /// reports and whether the worker panicked; callers that exit must persist
    /// both instead of dropping them with the window.
    pub fn finish(&mut self) -> (Vec<SaveReport>, bool) {
        self.sender = None;
        let panicked = self
            .handle
            .take()
            .is_some_and(|handle| handle.join().is_err());
        (self.take_reports(), panicked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contents(path: &PathBuf) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn finish_writes_a_pending_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut worker = SaveWorker::new();
        worker.enqueue(path.clone(), b"first".to_vec());
        let _ = worker.finish();
        assert_eq!(contents(&path), "first");
    }

    #[test]
    fn finish_returns_the_final_write_report() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut worker = SaveWorker::new();
        worker.enqueue(path.clone(), b"final".to_vec());
        let (reports, panicked) = worker.finish();
        assert!(!panicked);
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].path, path);
        assert!(reports[0].error.is_none());
        assert_eq!(contents(&path), "final");
    }

    #[test]
    fn last_snapshot_for_a_path_wins() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut worker = SaveWorker::new();
        worker.enqueue(path.clone(), b"old".to_vec());
        worker.enqueue(path.clone(), b"new".to_vec());
        worker.enqueue(path.clone(), b"final".to_vec());
        let _ = worker.finish();
        assert_eq!(contents(&path), "final");
    }

    #[test]
    fn distinct_paths_are_both_written() {
        let dir = tempfile::tempdir().unwrap();
        let library = dir.path().join("library.json");
        let playback = dir.path().join("playback.json");
        let mut worker = SaveWorker::new();
        worker.enqueue(library.clone(), b"library".to_vec());
        worker.enqueue(playback.clone(), b"playback".to_vec());
        let _ = worker.finish();
        assert_eq!(contents(&library), "library");
        assert_eq!(contents(&playback), "playback");
    }

    #[test]
    fn write_errors_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing-parent").join("state.json");
        // A file where the parent directory should be makes persist fail.
        std::fs::write(dir.path().join("missing-parent"), b"block").unwrap();
        let mut worker = SaveWorker::new();
        worker.enqueue(path, b"value".to_vec());
        let (reports, _) = worker.finish();
        assert!(reports.iter().any(|report| {
            report
                .error
                .as_deref()
                .is_some_and(|error| error.contains("Could not save"))
        }));
    }

    #[test]
    fn successful_write_is_reported_with_its_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut worker = SaveWorker::new();
        worker.enqueue(path.clone(), b"value".to_vec());
        let (reports, _) = worker.finish();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].path, path);
        assert!(reports[0].error.is_none());
    }

    #[test]
    fn backup_is_not_repeated_while_a_write_is_pending() {
        let path = PathBuf::from("library.json");
        let mut tracker = BackupTracker::default();
        tracker.need(path.clone());
        assert!(tracker.should_back_up(&path));
        tracker.mark_backed_up(path.clone());
        assert!(!tracker.should_back_up(&path));
        tracker.written(&path);
        assert!(!tracker.should_back_up(&path));
    }

    #[test]
    fn a_second_need_while_a_backup_is_pending_does_not_rearm_it() {
        let path = PathBuf::from("listening.json");
        let mut tracker = BackupTracker::default();
        tracker.need(path.clone());
        tracker.mark_backed_up(path.clone());
        tracker.need(path.clone());
        tracker.written(&path);
        assert!(!tracker.should_back_up(&path));
    }

    #[test]
    fn failed_write_rearms_the_backup() {
        let path = PathBuf::from("playback.json");
        let mut tracker = BackupTracker::default();
        tracker.need(path.clone());
        tracker.mark_backed_up(path.clone());
        tracker.failed(path.clone());
        assert!(tracker.should_back_up(&path));
        tracker.mark_backed_up(path.clone());
        tracker.written(&path);
        assert!(!tracker.should_back_up(&path));
    }

    #[test]
    fn failure_without_a_pending_backup_is_ignored() {
        let path = PathBuf::from("listening.json");
        let mut tracker = BackupTracker::default();
        tracker.failed(path.clone());
        assert!(!tracker.should_back_up(&path));
    }

    /// End-to-end: a corrupt file is backed up, the replacement is written by
    /// the worker, and only the confirmed write retires the pending backup.
    #[test]
    fn a_corrupt_state_file_is_backed_up_then_replaced_atomically() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("library.json");
        let broken = b"{not json";
        std::fs::write(&path, broken)?;

        let mut tracker = BackupTracker::default();
        tracker.need(path.clone());
        assert!(tracker.should_back_up(&path));
        let backup = crate::library::Library::back_up_invalid_json(&path, "library.invalid-")?;
        tracker.mark_backed_up(path.clone());
        assert!(!tracker.should_back_up(&path));
        assert_eq!(std::fs::read(&backup)?, broken);

        let replacement = br#"{"ok":true}"#;
        let mut worker = SaveWorker::new();
        worker.enqueue(path.clone(), replacement.to_vec());
        let (reports, panicked) = worker.finish();
        assert!(!panicked);
        assert_eq!(reports.len(), 1);
        assert!(reports[0].error.is_none(), "{:?}", reports[0].error);
        tracker.written(&path);
        assert!(!tracker.should_back_up(&path));
        assert_eq!(std::fs::read(&path)?, replacement);

        // The atomic write must not leave its temporary file behind, and the
        // backup of the corrupt original must remain untouched.
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "leftover temp files: {leftovers:?}");
        assert_eq!(std::fs::read(&backup)?, broken);
        Ok(())
    }
}
