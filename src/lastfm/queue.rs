use super::Play;
use crate::library::save_json;
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, fs, io, path::Path};

const CAPACITY: usize = 1000;

#[derive(Serialize, Deserialize)]
pub(super) struct Queue {
    owner: String,
    pub items: VecDeque<Play>,
}

impl Queue {
    pub fn empty(owner: &str) -> Self {
        Self {
            owner: owner.to_lowercase(),
            items: VecDeque::new(),
        }
    }

    pub fn read(path: &Path, owner: &str) -> io::Result<Self> {
        let file = match fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::empty(owner)),
            Err(error) => return Err(error),
        };
        let mut pending: Self = serde_json::from_reader(io::BufReader::new(file))?;
        if pending.owner != owner.to_lowercase() {
            return Ok(Self::empty(owner));
        }
        if pending.items.len() > CAPACITY {
            pending.items.drain(..pending.items.len() - CAPACITY);
        }
        Ok(pending)
    }

    pub fn push(&mut self, play: Play) {
        if self.items.len() == CAPACITY {
            self.items.pop_front();
        }
        self.items.push_back(play);
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        save_json(self, path)
    }

    pub fn clear(path: &Path) -> io::Result<()> {
        match fs::remove_file(path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lastfm::Track;

    #[test]
    fn restores_unsubmitted_tracks_for_the_same_account_only() -> io::Result<()> {
        let profile = tempfile::tempdir()?;
        let path = profile.path().join("lastfm-queue.json");
        let mut queue = Queue::read(&path, "Listener")?;
        queue.push(Play {
            track: Track {
                title: "Example track".into(),
                artist: "Example artist".into(),
                album: String::new(),
                album_artist: String::new(),
                recording_id: String::new(),
                track_number: None,
                duration: None,
            },
            timestamp: 1_000,
        });
        queue.save(&path)?;
        assert!(Queue::read(&path, "Other listener")?.items.is_empty());
        let mut restarted = Queue::read(&path, "LISTENER")?;
        assert_eq!(restarted.items.pop_front().unwrap().timestamp, 1_000);
        restarted.save(&path)?;
        assert!(Queue::read(&path, "Listener")?.items.is_empty());
        Queue::clear(&path)?;
        assert!(!path.exists());
        Ok(())
    }
}
