//! Single background writer for the small JSON state files.
//!
//! The UI thread only serializes a snapshot and sends the bytes here. This
//! thread owns the atomic write (`tempfile` + `fsync` + `persist`), so the
//! blocking disk work never stalls a frame. Several snapshots for the same
//! path are coalesced: only the newest one reaches the disk.

use crate::library::save_bytes;
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread::JoinHandle,
};

struct Snapshot {
    path: PathBuf,
    bytes: Vec<u8>,
}

pub struct SaveWorker {
    sender: Option<Sender<Snapshot>>,
    errors: Receiver<String>,
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
        let (failures, errors) = mpsc::channel();
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
                        if let Err(error) = save_bytes(&snapshot.bytes, &snapshot.path) {
                            let _ = failures.send(format!(
                                "Could not save {}: {error}",
                                snapshot.path.display()
                            ));
                        }
                    }
                }
            })
            .expect("spawn save worker");
        Self {
            sender: Some(sender),
            errors,
            handle: Some(handle),
        }
    }

    /// Queue an atomic write. The bytes must already be a complete snapshot.
    pub fn enqueue(&self, path: PathBuf, bytes: Vec<u8>) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(Snapshot { path, bytes });
        }
    }

    /// Drain write errors reported since the last call.
    pub fn take_errors(&self) -> Vec<String> {
        self.errors.try_iter().collect()
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
        assert!(
            worker
                .take_errors()
                .iter()
                .any(|error| error.contains("Could not save"))
        );
    }
}
