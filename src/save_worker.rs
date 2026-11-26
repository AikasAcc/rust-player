//! Single background writer for the small JSON state files.
//!
//! The UI thread only serializes a snapshot and sends the bytes here. This
//! thread owns the atomic write (`tempfile` + `fsync` + `persist`), so the
//! blocking disk work never stalls a frame. Several snapshots for the same
//! path are coalesced: only the newest one reaches the disk.

use crate::library::save_bytes;
use std::{
    collections::HashSet,
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
        self.needed.insert(path);
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
                    while let Ok(next) = incoming.try_recv() {
                        if let Some(queued) = pending
                            .iter_mut()
                            .find(|snapshot| snapshot.path == next.path)
                        {
                            queued.bytes = next.bytes;
                        } else {
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

    /// Finish everything still queued, then stop the thread. Called on exit
    /// so graceful shutdown never drops a pending write.
    pub fn shutdown(&mut self) {
        self.sender = None;
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contents(path: &PathBuf) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn shutdown_writes_a_pending_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut worker = SaveWorker::new();
        worker.enqueue(path.clone(), b"first".to_vec());
        worker.shutdown();
        assert_eq!(contents(&path), "first");
    }

    #[test]
    fn last_snapshot_for_a_path_wins() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut worker = SaveWorker::new();
        worker.enqueue(path.clone(), b"old".to_vec());
        worker.enqueue(path.clone(), b"new".to_vec());
        worker.enqueue(path.clone(), b"final".to_vec());
        worker.shutdown();
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
        worker.shutdown();
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
        worker.shutdown();
        assert!(worker.take_reports().iter().any(|report| {
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
        worker.shutdown();
        let reports = worker.take_reports();
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
}
