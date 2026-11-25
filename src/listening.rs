use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Listening {
    pub history: Vec<Play>,
    pub songs: HashMap<PathBuf, Song>,
    pub artists: HashMap<String, Artist>,
}

pub struct Rankings {
    pub songs: Vec<PathBuf>,
    pub artists: Vec<String>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Play {
    pub title: String,
    pub artist: String,
    pub time: u64,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Song {
    pub title: String,
    pub artist: String,
    pub plays: u64,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Artist {
    pub name: String,
    pub plays: u64,
}

impl Listening {
    pub fn rankings(&self) -> Rankings {
        let mut songs: Vec<_> = self.songs.iter().collect();
        songs.sort_unstable_by(|(path_a, a), (path_b, b)| {
            b.plays
                .cmp(&a.plays)
                .then_with(|| a.title.cmp(&b.title))
                .then_with(|| a.artist.cmp(&b.artist))
                .then_with(|| path_a.cmp(path_b))
        });
        let mut artists: Vec<_> = self.artists.iter().collect();
        artists.sort_unstable_by(|(key_a, a), (key_b, b)| {
            b.plays
                .cmp(&a.plays)
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| key_a.cmp(key_b))
        });
        Rankings {
            songs: songs.into_iter().map(|(path, _)| path.clone()).collect(),
            artists: artists.into_iter().map(|(key, _)| key.clone()).collect(),
        }
    }

    pub fn load(path: &Path) -> std::io::Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    /// Kept for persistence tests; production writes go through `save_worker`.
    #[cfg(test)]
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        crate::library::save_json(self, path)
    }

    pub fn record(&mut self, path: PathBuf, title: &str, artist: &str) {
        self.history.push(Play {
            title: title.into(),
            artist: artist.into(),
            time: now(),
        });
        if self.history.len() > 1000 {
            self.history.drain(..self.history.len() - 1000);
        }
        let song = self.songs.entry(path).or_insert_with(|| Song {
            title: title.into(),
            artist: artist.into(),
            plays: 0,
        });
        song.title = title.into();
        song.artist = artist.into();
        song.plays += 1;
        if !artist.trim().is_empty() {
            let artist = self
                .artists
                .entry(artist.trim().to_lowercase())
                .or_insert_with(|| Artist {
                    name: artist.trim().into(),
                    plays: 0,
                });
            artist.plays += 1;
        }
    }
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn age(secs: u64) -> String {
    match secs {
        0..60 => "Just now".into(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86400 => format!("{} h ago", secs / 3600),
        _ => format!("{} d ago", secs / 86400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn load_returns_default_for_missing_file() -> std::io::Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("listening.json");
        let loaded = Listening::load(&path)?;
        assert!(loaded.history.is_empty());
        assert!(loaded.songs.is_empty());
        assert!(loaded.artists.is_empty());
        Ok(())
    }

    #[test]
    fn load_reports_invalid_json_for_backup_handling() -> std::io::Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("listening.json");
        fs::write(&path, b"{not json")?;
        assert!(Listening::load(&path).is_err());
        Ok(())
    }

    #[test]
    fn load_applies_defaults_to_partial_json() -> std::io::Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("listening.json");
        fs::write(&path, b"{}")?;
        let loaded = Listening::load(&path)?;
        assert!(loaded.history.is_empty());
        assert!(loaded.songs.is_empty());
        assert!(loaded.artists.is_empty());
        Ok(())
    }

    #[test]
    fn load_applies_defaults_to_partial_nested_entries() -> std::io::Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("listening.json");
        fs::write(
            &path,
            br#"{"history":[{"title":"T"}],"songs":{"x.flac":{"title":"T"}},"artists":{"a":{}}}"#,
        )?;
        let loaded = Listening::load(&path)?;
        assert_eq!(loaded.history[0].artist, "");
        assert_eq!(loaded.history[0].time, 0);
        assert_eq!(loaded.songs[&PathBuf::from("x.flac")].plays, 0);
        assert_eq!(loaded.artists["a"].name, "");
        Ok(())
    }

    #[test]
    fn save_and_load_round_trip_keeps_history_limit() -> std::io::Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("nested").join("listening.json");
        let mut data = Listening::default();
        for index in 0..1001 {
            data.record(
                PathBuf::from(format!("track-{index}.flac")),
                &format!("Title {index}"),
                "Artist",
            );
        }
        assert_eq!(data.history.len(), 1000);
        assert_eq!(data.songs.len(), 1001);

        data.save(&path)?;
        let loaded = Listening::load(&path)?;

        assert_eq!(loaded.history.len(), 1000);
        assert_eq!(loaded.songs.len(), 1001);
        assert_eq!(loaded.artists.len(), 1);
        let rankings = loaded.rankings();
        assert_eq!(rankings.songs.len(), 1001);
        assert_eq!(rankings.artists, ["artist"]);
        Ok(())
    }
}
