use lofty::{file::TaggedFileExt, prelude::Accessor, tag::ItemKey};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

pub fn timed_lyrics(text: &str) -> Vec<(Duration, String)> {
    let mut lyrics = Vec::new();
    for line in text.lines() {
        let mut rest = line.trim_start_matches('\u{feff}').trim_start();
        let mut timestamps = Vec::new();
        while let Some((stamp, remaining)) =
            rest.strip_prefix('[').and_then(|line| line.split_once(']'))
        {
            let Some((minutes, seconds)) = stamp.split_once(':') else {
                break;
            };
            let (Ok(minutes), Ok(seconds)) = (minutes.parse::<u32>(), seconds.parse::<f64>())
            else {
                break;
            };
            if !(0.0..60.0).contains(&seconds) {
                break;
            }
            timestamps.push(Duration::from_secs_f64(f64::from(minutes) * 60.0 + seconds));
            rest = remaining;
        }
        lyrics.extend(
            timestamps
                .into_iter()
                .map(|time| (time, rest.trim().to_owned())),
        );
    }
    lyrics.sort_by_key(|(time, _)| *time);
    lyrics
}

pub const AUDIO: &[&str] = &[
    "mp3", "flac", "wav", "ogg", "m4a", "aac", "alac", "aiff", "aif", "aifc", "caf", "cue",
];

#[derive(Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub name: String,
    pub tracks: Vec<PathBuf>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub sidebar_position: Option<usize>,
    #[serde(default)]
    pub sort: Sort,
    #[serde(default)]
    pub descending: bool,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sort {
    #[default]
    Added,
    Title,
    Artist,
    Album,
    Duration,
}

impl Sort {
    pub fn label(self) -> &'static str {
        match self {
            Self::Added => "Date added",
            Self::Title => "Title",
            Self::Artist => "Artist",
            Self::Album => "Album",
            Self::Duration => "Duration",
        }
    }
}

#[derive(Default)]
pub struct TrackInfo {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub disc: Option<u32>,
    pub number: Option<u32>,
    pub duration: Option<Duration>,
}

impl TrackInfo {
    pub fn read(path: &Path) -> Self {
        if let Ok(Some(track)) = crate::cue::resolve(path) {
            return Self {
                title: track.title,
                artist: track.artist.clone(),
                album: track.album,
                album_artist: track.artist,
                number: Some(track.number),
                duration: track
                    .end
                    .map(|end| crate::cue::duration(end - track.start))
                    .or_else(|| {
                        crate::audio::container_duration(&track.audio)
                            .and_then(|end| end.checked_sub(crate::cue::duration(track.start)))
                    })
                    .filter(|duration| !duration.is_zero()),
                ..Self::default()
            };
        }
        let mut info = Self {
            title: file_title(path),
            ..Self::default()
        };
        let options = lofty::config::ParseOptions::new().read_cover_art(false);
        let tagged = lofty::probe::Probe::open(path)
            .and_then(|probe| Ok(probe.guess_file_type()?))
            .and_then(|probe| probe.options(options).read());
        if let Ok(file) = tagged {
            use lofty::file::AudioFile;
            let duration = file.properties().duration();
            info.duration = (!duration.is_zero()).then_some(duration);
            if let Some(tag) = file.primary_tag().or_else(|| file.first_tag()) {
                if let Some(title) = tag.title().filter(|title| !title.trim().is_empty()) {
                    info.title = title.into_owned();
                }
                info.artist = tag.artist().unwrap_or_default().trim().to_owned();
                info.album = tag.album().unwrap_or_default().trim().to_owned();
                info.album_artist = tag
                    .get_string(ItemKey::AlbumArtist)
                    .unwrap_or(&info.artist)
                    .trim()
                    .to_owned();
                info.disc = tag.disk().filter(|&disc| disc > 0);
                info.number = tag.track().filter(|&number| number > 0);
            }
        }
        if info.duration.is_none() {
            info.duration = crate::audio::container_duration(path);
        }
        info
    }

    pub fn track_number(&self) -> String {
        match (self.disc, self.number) {
            (Some(disc), Some(number)) => format!("{disc}.{number:02}"),
            (None, Some(number)) => format!("{number:02}"),
            _ => String::new(),
        }
    }
}

pub enum PlaylistRow {
    Album(String),
    Track(usize),
}

pub struct PlaylistView {
    pub tracks: Vec<TrackInfo>,
    pub order: Vec<usize>,
    pub rows: Vec<PlaylistRow>,
}

impl PlaylistView {
    pub fn build(tracks: Vec<TrackInfo>, sort: Sort, descending: bool) -> Self {
        let mut order: Vec<_> = (0..tracks.len()).collect();
        if sort == Sort::Duration {
            order.sort_by(
                |&left, &right| match (tracks[left].duration, tracks[right].duration) {
                    (Some(a), Some(b)) if descending => b.cmp(&a),
                    (Some(a), Some(b)) => a.cmp(&b),
                    (a, b) => a.is_none().cmp(&b.is_none()),
                },
            );
        } else if sort != Sort::Added {
            let keys: Vec<_> = tracks
                .iter()
                .map(|track| {
                    let key = match sort {
                        Sort::Artist => &track.artist,
                        Sort::Album => &track.album,
                        _ => &track.title,
                    };
                    (
                        key.to_lowercase(),
                        if sort == Sort::Album {
                            track.album_artist.to_lowercase()
                        } else {
                            String::new()
                        },
                        track.title.to_lowercase(),
                    )
                })
                .collect();
            order.sort_by(|&left, &right| {
                let (a, artist_a, title_a) = &keys[left];
                let (b, artist_b, title_b) = &keys[right];
                let directed = |o: std::cmp::Ordering| if descending { o.reverse() } else { o };
                let primary = a.is_empty().cmp(&b.is_empty()).then(directed(a.cmp(b)));
                if sort == Sort::Album && !a.is_empty() && !b.is_empty() {
                    let l = &tracks[left];
                    let r = &tracks[right];
                    primary
                        .then(artist_a.cmp(artist_b))
                        .then(l.disc.unwrap_or(1).cmp(&r.disc.unwrap_or(1)))
                        .then(
                            l.number
                                .unwrap_or(u32::MAX)
                                .cmp(&r.number.unwrap_or(u32::MAX)),
                        )
                        .then(title_a.cmp(title_b))
                } else {
                    primary.then(directed(title_a.cmp(title_b)))
                }
            });
        }
        let mut rows = Vec::with_capacity(order.len());
        let mut previous: Option<(&str, &str)> = None;
        for &index in &order {
            let track = &tracks[index];
            let album = (track.album.as_str(), track.album_artist.as_str());
            let same = previous.is_some_and(|(name, artist)| {
                name.eq_ignore_ascii_case(album.0) && artist.eq_ignore_ascii_case(album.1)
            });
            if !album.0.is_empty() && !same {
                let label = if album.1.is_empty() {
                    album.0.to_owned()
                } else {
                    format!("{} · {}", album.1, album.0)
                };
                rows.push(PlaylistRow::Album(label));
            }
            previous = Some(album);
            rows.push(PlaylistRow::Track(index));
        }
        Self {
            tracks,
            order,
            rows,
        }
    }

    pub fn row_count(&self, total_tracks: usize) -> usize {
        self.rows.len() + total_tracks.saturating_sub(self.tracks.len())
    }
}
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Library {
    pub media_keys: bool,
    pub audio_setup_done: bool,
    pub restore_last_track: bool,
    pub restart_track_on_previous: bool,
    pub gapless: bool,
    pub repeat: crate::gapless_ui::Repeat,
    pub audio_device: Option<String>,
    pub audio_output: crate::audio::OutputOptions,
    pub audio_dsp: crate::audio::dsp::Settings,
    pub theme: crate::theme::Theme,
    pub font: Option<PathBuf>,
    pub background: crate::background::Settings,
    pub named_themes: Vec<crate::theme::NamedTheme>,
    pub plugins: Vec<crate::plugins::PluginConfig>,
    pub favorites: Option<usize>,
    pub volume: f32,
    pub playlists: Vec<Playlist>,
    pub selected: usize,
    pub shuffle: bool,
    pub discord_enabled: bool,
    pub lastfm_enabled: bool,
    pub lastfm_now_playing: bool,
    pub discord_id: String,
    pub online_artwork: bool,
    pub online_lyrics: bool,
    pub fallback_asset: String,
    pub artwork_urls: HashMap<PathBuf, String>,
}

impl Default for Library {
    fn default() -> Self {
        Self {
            media_keys: true,
            audio_setup_done: false,
            restore_last_track: true,
            restart_track_on_previous: false,
            gapless: true,
            repeat: crate::gapless_ui::Repeat::Off,
            audio_device: None,
            audio_output: crate::audio::OutputOptions::default(),
            audio_dsp: crate::audio::dsp::Settings::default(),
            background: Default::default(),
            named_themes: Vec::new(),
            theme: crate::theme::Theme::default(),
            font: None,
            plugins: Vec::new(),
            favorites: None,
            volume: 0.7,
            playlists: vec![Playlist {
                name: "Default".into(),
                tracks: Vec::new(),
                pinned: false,
                sidebar_position: None,
                sort: Sort::Added,
                descending: false,
            }],
            selected: 0,
            shuffle: false,
            discord_enabled: false,
            lastfm_enabled: false,
            lastfm_now_playing: false,
            discord_id: String::new(),
            online_artwork: false,
            online_lyrics: false,
            fallback_asset: String::new(),
            artwork_urls: HashMap::new(),
        }
    }
}

impl Library {
    pub fn sidebar_order(&self) -> Vec<usize> {
        let mut order: Vec<_> = (0..self.playlists.len()).collect();
        order.sort_by_key(|&index| {
            let playlist = &self.playlists[index];
            (
                Some(index) != self.favorites,
                !playlist.pinned,
                playlist.sidebar_position.unwrap_or(usize::MAX),
            )
        });
        order
    }

    pub fn can_move_playlist(&self, from: usize, to: usize) -> bool {
        from != to
            && Some(from) != self.favorites
            && Some(to) != self.favorites
            && self
                .playlists
                .get(from)
                .zip(self.playlists.get(to))
                .is_some_and(|(source, target)| source.pinned == target.pinned)
    }

    pub fn move_playlist(&mut self, from: usize, to: usize, after: bool) -> bool {
        if !self.can_move_playlist(from, to) {
            return false;
        }
        let mut order = self.sidebar_order();
        let source = order
            .iter()
            .position(|&index| index == from)
            .expect("source in sidebar");
        let target = order
            .iter()
            .position(|&index| index == to)
            .expect("target in sidebar");
        let destination = target + usize::from(after) - usize::from(source < target);
        if source == destination {
            return false;
        }
        order.remove(source);
        order.insert(destination, from);
        for (position, index) in order.into_iter().enumerate() {
            self.playlists[index].sidebar_position = Some(position);
        }
        true
    }

    pub fn load(path: &Path) -> std::io::Result<Self> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error),
        };
        let mut data: Self = serde_json::from_slice(&bytes)?;
        data.volume = data.volume.clamp(0.0, 1.0);
        if data.playlists.is_empty() {
            data.playlists = Self::default().playlists;
        }
        data.selected = data.selected.min(data.playlists.len() - 1);
        data.favorites = data.favorites.filter(|&index| index < data.playlists.len());
        Ok(data)
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        save_json(self, path)
    }

    pub fn back_up_invalid_file(path: &Path) -> std::io::Result<PathBuf> {
        let mut original = fs::File::open(path)?;
        let directory = path.parent().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "Missing settings folder")
        })?;
        let mut backup = tempfile::Builder::new()
            .prefix("library.invalid-")
            .suffix(".json")
            .tempfile_in(directory)?;
        std::io::copy(&mut original, &mut backup)?;
        backup.as_file().sync_all()?;
        let (_, backup_path) = backup.keep()?;
        Ok(backup_path)
    }
}

pub fn save_json(value: &impl Serialize, path: &Path) -> std::io::Result<()> {
    let dir = path.parent().expect("settings file has a parent");
    fs::create_dir_all(dir)?;
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    serde_json::to_writer(&mut file, value)?;
    file.flush()?;
    file.persist(path)?;
    Ok(())
}

pub fn collect(paths: Vec<PathBuf>) -> (Vec<PathBuf>, usize) {
    let mut tracks = Vec::new();
    let mut skipped = 0;
    for path in paths {
        if path.is_dir() {
            for entry in walkdir::WalkDir::new(path).sort_by_file_name() {
                match entry {
                    Ok(entry) if entry.file_type().is_file() && supported(entry.path()) => {
                        add_path(entry.into_path(), &mut tracks, &mut skipped);
                    }
                    Err(_) => skipped += 1,
                    _ => {}
                }
            }
        } else if path.is_file() && supported(&path) {
            add_path(path, &mut tracks, &mut skipped);
        } else {
            skipped += 1;
        }
    }
    (tracks, skipped)
}

fn add_path(path: PathBuf, tracks: &mut Vec<PathBuf>, skipped: &mut usize) {
    if crate::cue::is_sheet(&path) {
        match crate::cue::entries(&path) {
            Ok(entries) => tracks.extend(entries),
            Err(_) => *skipped += 1,
        }
    } else {
        tracks.push(path);
    }
}

pub fn supported(path: &Path) -> bool {
    crate::cue::reference(path).is_some()
        || path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| AUDIO.iter().any(|kind| ext.eq_ignore_ascii_case(kind)))
}

#[derive(Default)]
pub struct Media {
    pub title: String,
    pub tagged_title: bool,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub recording_id: String,
    pub track_number: Option<u32>,
    pub cover: Option<Vec<u8>>,
    pub lyrics: String,
}

pub fn file_title(path: &Path) -> String {
    path.file_stem()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

impl Media {
    pub fn read(path: &Path) -> Self {
        let cue = crate::cue::resolve(path).ok().flatten();
        let path = cue.as_ref().map_or(path, |track| track.audio.as_path());
        let mut media = Self {
            title: file_title(path),
            ..Self::default()
        };
        if let Ok(file) = lofty::read_from_path(path) {
            for tag in file.primary_tag().into_iter().chain(file.tags()) {
                if let Some(title) = tag.title().filter(|text| !text.trim().is_empty()) {
                    media.title = title.into_owned();
                    media.tagged_title = true;
                    break;
                }
            }
            for tag in file.primary_tag().into_iter().chain(file.tags()) {
                if media.artist.is_empty() {
                    media.artist = tag
                        .artist()
                        .map(|text| text.into_owned())
                        .unwrap_or_default();
                }
                if media.album.is_empty() {
                    media.album = tag
                        .album()
                        .map(|text| text.into_owned())
                        .unwrap_or_default();
                }
                if media.album_artist.is_empty() {
                    media.album_artist = tag
                        .get_string(ItemKey::AlbumArtist)
                        .unwrap_or_default()
                        .to_owned();
                }
                if media.recording_id.is_empty() {
                    media.recording_id = tag
                        .get_string(ItemKey::MusicBrainzRecordingId)
                        .unwrap_or_default()
                        .to_owned();
                }
                media.track_number = media.track_number.or_else(|| tag.track());
                if media.lyrics.is_empty() {
                    media.lyrics = tag
                        .get_string(ItemKey::Lyrics)
                        .or_else(|| tag.get_string(ItemKey::UnsyncLyrics))
                        .unwrap_or_default()
                        .to_owned();
                }
                if media.cover.is_none() {
                    media.cover = tag
                        .pictures()
                        .iter()
                        .find(|picture| {
                            picture.pic_type() == lofty::picture::PictureType::CoverFront
                        })
                        .or_else(|| tag.pictures().first())
                        .map(|picture| picture.data().to_vec());
                }
            }
        }
        if media.cover.is_none() {
            if let Some(dir) = path.parent() {
                media.cover = [
                    "cover.jpg",
                    "folder.jpg",
                    "cover.png",
                    "folder.png",
                    "front.jpg",
                ]
                .iter()
                .find_map(|name| fs::read(dir.join(name)).ok());
            }
        }
        if media.lyrics.trim().is_empty() {
            media.lyrics = ["lrc", "txt"]
                .iter()
                .find_map(|ext| fs::read_to_string(path.with_extension(ext)).ok())
                .unwrap_or_default();
        }
        if let Some(track) = cue {
            media.title = track.title;
            media.tagged_title = track.tagged_title;
            media.artist = track.artist;
            media.album = track.album;
            media.album_artist.clear();
            media.recording_id.clear();
            media.track_number = Some(track.number);
            // Embedded and sidecar lyrics belong to the whole disc image.
            media.lyrics.clear();
        }
        media
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_sort_keeps_unknown_tracks_last_and_ties_stable() {
        let tracks = || {
            [Some(241), None, Some(999), Some(241), Some(150), None]
                .into_iter()
                .map(|duration| TrackInfo {
                    duration: duration.map(Duration::from_millis),
                    ..TrackInfo::default()
                })
                .collect()
        };
        assert_eq!(
            PlaylistView::build(tracks(), Sort::Duration, false).order,
            [4, 0, 3, 2, 1, 5]
        );
        assert_eq!(
            PlaylistView::build(tracks(), Sort::Duration, true).order,
            [2, 0, 3, 4, 1, 5]
        );
    }

    #[test]
    fn playback_settings_load_old_libraries_and_persist() -> std::io::Result<()> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("library.json");
        fs::write(
            &path,
            br#"{"volume":0.25,"playlists":[{"name":"Old","tracks":[]}]}"#,
        )?;

        let mut library = Library::load(&path)?;
        assert_eq!(library.playlists[0].name, "Old");
        assert_eq!(library.volume, 0.25);
        assert!(library.media_keys);
        assert!(library.restore_last_track);
        assert!(!library.restart_track_on_previous);
        assert!(!library.background.enabled);
        assert!(library.named_themes.is_empty());

        library.media_keys = false;
        library.restore_last_track = false;
        library.restart_track_on_previous = true;
        library.playlists[0].sort = Sort::Duration;
        library.playlists[0].descending = true;
        library.background.enabled = true;
        library.named_themes.push(crate::theme::NamedTheme {
            name: "Night show".into(),
            font: Some(PathBuf::from("example.ttf")),
            theme: library.theme,
            background: library.background.clone(),
        });
        library.save(&path)?;

        let library = Library::load(&path)?;
        assert_eq!(library.playlists[0].name, "Old");
        assert_eq!(library.volume, 0.25);
        assert!(!library.media_keys);
        assert!(!library.restore_last_track);
        assert!(library.restart_track_on_previous);
        assert!(library.playlists[0].sort == Sort::Duration);
        assert!(library.playlists[0].descending);
        assert!(library.background.enabled);
        assert_eq!(library.named_themes[0].name, "Night show");
        assert_eq!(
            library.named_themes[0].font.as_deref(),
            Some(Path::new("example.ttf"))
        );
        assert!(library.named_themes[0].background.enabled);
        Ok(())
    }

    #[test]
    fn backup_retains_original_bytes_and_keeps_prior_backups() -> std::io::Result<()> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("library.json");
        let broken_json = b"{\xFF";
        fs::write(&path, broken_json)?;
        assert!(Library::load(&path).is_err());

        let first = Library::back_up_invalid_file(&path)?;
        let second = Library::back_up_invalid_file(&path)?;
        assert_ne!(first, second);
        assert_eq!(fs::read(&path)?, broken_json);

        Library::default().save(&path)?;
        assert_eq!(fs::read(first)?, broken_json);
        assert_eq!(fs::read(second)?, broken_json);
        assert!(Library::load(&path).is_ok());
        Ok(())
    }
}
