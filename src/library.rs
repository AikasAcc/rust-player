use lofty::{file::TaggedFileExt, prelude::Accessor, tag::ItemKey};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

/// Normalized key used by every path lookup. Components remove separator
/// differences; Windows additionally ignores letter case.
pub(crate) fn path_key(path: &Path) -> String {
    let mut value = String::new();
    for component in path.components() {
        value.push_str(&component.as_os_str().to_string_lossy());
        value.push('|');
    }
    #[cfg(windows)]
    let value = value.to_lowercase();
    value
}

/// Compare paths the way the target filesystem does. On Windows, letter case
/// and separator style do not distinguish a path; on other platforms they do.
pub fn same_path(a: &Path, b: &Path) -> bool {
    path_key(a) == path_key(b)
}

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
    "mp3", "mp1", "mp2", "flac", "wav", "ogg", "m4a", "aac", "alac", "aiff", "aif", "aifc", "caf",
    "mka", "mkv", "ape", "dsf", "dff", "wv", "cue",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TrackId(pub u64);

#[derive(Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub name: String,
    pub tracks: Vec<PathBuf>,
    #[serde(default)]
    pub ids: Vec<TrackId>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub sidebar_position: Option<usize>,
    #[serde(default)]
    pub sort: Sort,
    #[serde(default)]
    pub descending: bool,
    /// Saved foobar-style query. When set, the playlist is recalculated from
    /// the rest of the library instead of being edited track by track.
    #[serde(default)]
    pub smart_query: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TrackInfo {
    #[serde(skip)]
    pub search_key: String,
    #[serde(default)]
    pub missing: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub year: Option<u16>,
    pub disc: Option<u32>,
    pub number: Option<u32>,
    pub duration: Option<Duration>,
}

impl TrackInfo {
    pub fn prepare_search(&mut self, path: &Path) {
        self.search_key = [
            &self.title[..],
            &self.artist,
            &self.album,
            &self.album_artist,
            &self.genre,
            &self.year.map_or_else(String::new, |year| year.to_string()),
            &file_title(path),
        ]
        .join("\0")
        .to_lowercase();
    }
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
        // Lofty cannot probe DSF/DFF; dsd-reader already parses their ID3 tag
        // and the DSD header carries an exact duration.
        if path.extension().is_some_and(|extension| {
            extension.eq_ignore_ascii_case("dsf") || extension.eq_ignore_ascii_case("dff")
        }) {
            if let Some(tag) = crate::audio::dsd::metadata(path) {
                if let Some(title) = tag.title.filter(|title| !title.trim().is_empty()) {
                    info.title = title;
                }
                info.artist = tag.artist;
                info.album = tag.album;
                info.album_artist = if tag.album_artist.is_empty() {
                    info.artist.clone()
                } else {
                    tag.album_artist
                };
                info.genre = tag.genre;
                info.year = tag.year;
                info.disc = tag.disc;
                info.number = tag.number;
            }
            info.duration = crate::audio::container_duration(path);
            return info;
        }
        // libwavpack reads APEv2 tags itself; lofty does not probe `.wv`.
        if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("wv"))
        {
            if let Some(tag) = crate::audio::wavpack::metadata(path) {
                if let Some(title) = tag.title.filter(|title| !title.trim().is_empty()) {
                    info.title = title;
                }
                info.artist = tag.artist;
                info.album = tag.album;
                info.album_artist = if tag.album_artist.is_empty() {
                    info.artist.clone()
                } else {
                    tag.album_artist
                };
                info.genre = tag.genre;
                info.year = tag.year;
                info.disc = tag.disc;
                info.number = tag.number;
            }
            info.duration = crate::audio::container_duration(path);
            return info;
        }
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
                info.genre = tag
                    .get_string(ItemKey::Genre)
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                info.year = tag.date().map(|date| date.year).filter(|&year| year > 0);
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

#[derive(Clone)]
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
    #[serde(skip)]
    pub locations: HashMap<TrackId, crate::Track>,
    /// First source entry per normalized path, excluding Favorites. Derived
    /// from `playlists` by `rebuild_locations()`, never serialized.
    #[serde(skip)]
    path_sources: HashMap<String, TrackId>,
    /// Favorites rows per normalized path. Derived the same way.
    #[serde(skip)]
    favorite_paths: HashMap<String, Vec<usize>>,
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
    #[serde(default)]
    library_desk_theme_seen: bool,
    pub font: Option<PathBuf>,
    pub background: crate::background::Settings,
    pub named_themes: Vec<crate::theme::NamedTheme>,
    pub plugins: Vec<crate::plugins::PluginConfig>,
    pub favorites: Option<usize>,
    /// Source TrackId for each entry of the Favorites playlist.
    /// `None` marks a legacy entry that is still matched by path.
    #[serde(default)]
    pub favorite_ids: Vec<Option<TrackId>>,
    pub volume: f32,
    pub muted: bool,
    pub volume_before_mute: f32,
    pub check_for_updates: bool,
    pub track_artwork: bool,
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
            locations: Default::default(),
            path_sources: HashMap::new(),
            favorite_paths: HashMap::new(),
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
            library_desk_theme_seen: true,
            font: None,
            plugins: Vec::new(),
            favorites: None,
            favorite_ids: Vec::new(),
            volume: 0.7,
            muted: false,
            volume_before_mute: 0.7,
            check_for_updates: true,
            track_artwork: false,
            playlists: vec![Playlist {
                ids: Vec::new(),
                name: "Default".into(),
                tracks: Vec::new(),
                pinned: false,
                sidebar_position: None,
                sort: Sort::Album,
                descending: false,
                smart_query: None,
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
    pub fn rebuild_locations(&mut self) {
        let mut locations = HashMap::new();
        let mut path_sources = HashMap::new();
        let mut favorite_paths = HashMap::new();
        let mut all_ids: Vec<Vec<TrackId>> = Vec::with_capacity(self.playlists.len());
        for (list, playlist) in self.playlists.iter().enumerate() {
            let mut ids = Vec::with_capacity(playlist.tracks.len());
            for (index, _) in playlist.tracks.iter().enumerate() {
                let id = playlist
                    .ids
                    .get(index)
                    .copied()
                    .filter(|id| !locations.contains_key(id))
                    .unwrap_or_else(|| {
                        loop {
                            let candidate = TrackId(rand::random());
                            if !locations.contains_key(&candidate)
                                && !all_ids.iter().flatten().any(|used| *used == candidate)
                                && !playlist.ids.contains(&candidate)
                            {
                                break candidate;
                            }
                        }
                    });
                locations.insert(id, (list, index));
                ids.push(id);
            }
            if Some(list) == self.favorites {
                for (index, path) in playlist.tracks.iter().enumerate() {
                    favorite_paths
                        .entry(path_key(path))
                        .or_insert_with(Vec::new)
                        .push(index);
                }
            } else {
                for (index, path) in playlist.tracks.iter().enumerate() {
                    if let Some(&id) = ids.get(index) {
                        // source_id() historically returns the first match.
                        path_sources.entry(path_key(path)).or_insert(id);
                    }
                }
            }
            all_ids.push(ids);
        }
        for (playlist, ids) in self.playlists.iter_mut().zip(all_ids) {
            playlist.ids = ids;
        }
        self.locations = locations;
        self.path_sources = path_sources;
        self.favorite_paths = favorite_paths;
    }

    /// Stable identity of a playlist entry. Never derived from the path.
    pub fn track_id(&self, track: crate::Track) -> Option<TrackId> {
        self.playlists.get(track.0)?.ids.get(track.1).copied()
    }

    /// Current location of a stable ID. Locations are derived and may change.
    pub fn locate(&self, id: TrackId) -> Option<crate::Track> {
        self.locations.get(&id).copied()
    }

    pub fn track_path(&self, id: TrackId) -> Option<&Path> {
        let (list, index) = self.locate(id)?;
        self.playlists
            .get(list)?
            .tracks
            .get(index)
            .map(PathBuf::as_path)
    }
    pub fn toggle_mute(&mut self) {
        if self.muted {
            self.volume = self.volume_before_mute;
        } else {
            self.volume_before_mute = self.volume;
        }
        self.muted = !self.muted;
    }
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

    /// Playlists for the sidebar's PLAYLISTS section. Favorites has its own
    /// LIBRARY entry and must not appear a second time here.
    pub fn sidebar_playlists(&self) -> Vec<usize> {
        self.sidebar_order()
            .into_iter()
            .filter(|&index| Some(index) != self.favorites)
            .collect()
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
        if !data.library_desk_theme_seen {
            if data.theme == crate::theme::Theme::STUDIO {
                data.theme = crate::theme::Theme::LIBRARY_DESK;
            }
            data.library_desk_theme_seen = true;
        }
        data.volume = data.volume.clamp(0.0, 1.0);
        data.volume_before_mute = data.volume_before_mute.clamp(0.0, 1.0);
        if data.playlists.is_empty() {
            data.playlists = Self::default().playlists;
        }
        data.selected = data.selected.min(data.playlists.len() - 1);
        data.favorites = data.favorites.filter(|&index| index < data.playlists.len());
        data.rebuild_locations();
        data.migrate_favorite_ids();
        Ok(data)
    }

    /// Kept for persistence tests; production writes go through `save_worker`.
    #[cfg(test)]
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        save_json(self, path)
    }

    pub fn unique_playlist_name(&self, name: &str) -> String {
        let exists = |candidate: &str| {
            self.playlists
                .iter()
                .any(|list| list.name.eq_ignore_ascii_case(candidate))
        };
        if !exists(name) {
            return name.to_owned();
        }
        (2..)
            .map(|number| format!("{name} ({number})"))
            .find(|candidate| !exists(candidate))
            .unwrap_or_else(|| format!("{name} ({})", self.playlists.len() + 1))
    }

    pub fn favorites_index_or_create(&mut self) -> usize {
        if let Some(index) = self.favorites {
            return index;
        }
        let index = self.playlists.len();
        self.playlists.push(Playlist {
            name: self.unique_playlist_name("Favorites"),
            tracks: Vec::new(),
            ids: Vec::new(),
            pinned: false,
            sidebar_position: None,
            sort: Sort::Album,
            descending: false,
            smart_query: None,
        });
        self.favorites = Some(index);
        index
    }

    fn source_id(&self, path: &Path) -> Option<TrackId> {
        self.path_sources.get(&path_key(path)).copied()
    }

    pub fn migrate_favorite_ids(&mut self) {
        let Some(index) = self.favorites else {
            return;
        };
        if self.favorite_ids.len() >= self.playlists[index].tracks.len() {
            return;
        }
        self.favorite_ids = self.playlists[index]
            .tracks
            .iter()
            .map(|path| self.source_id(path))
            .collect();
    }

    pub fn favorite_id_entries(&self, id: TrackId) -> Vec<usize> {
        self.favorite_ids
            .iter()
            .enumerate()
            .filter_map(|(entry, favorite)| (*favorite == Some(id)).then_some(entry))
            .collect()
    }

    pub fn is_favorite_id(&self, id: TrackId) -> bool {
        self.favorite_ids.contains(&Some(id))
            || self
                .favorites
                .and_then(|index| self.playlists.get(index))
                .is_some_and(|playlist| playlist.ids.contains(&id))
    }

    /// A row is active when it is the playing entry itself or the Favorites
    /// copy of the playing source entry.
    pub fn row_is_playing(&self, list: usize, index: usize, playing: TrackId) -> bool {
        if self.track_id((list, index)) == Some(playing) {
            return true;
        }
        Some(list) == self.favorites
            && self
                .favorite_ids
                .get(index)
                .is_some_and(|id| *id == Some(playing))
    }

    pub fn favorite_entries(&self, path: &Path) -> Vec<usize> {
        let mut entries = self
            .source_id(path)
            .map(|id| self.favorite_id_entries(id))
            .unwrap_or_default();
        if let Some(indexed) = self.favorite_paths.get(&path_key(path)) {
            for &entry in indexed {
                if !entries.contains(&entry) {
                    entries.push(entry);
                }
            }
        }
        entries.sort_unstable();
        entries
    }

    #[cfg(test)]
    pub fn is_favorite(&self, path: &Path) -> bool {
        !self.favorite_entries(path).is_empty()
    }

    pub fn add_favorite(&mut self, path: &Path) {
        let id = self.source_id(path);
        let index = self.favorites_index_or_create();
        self.playlists[index].tracks.push(path.to_path_buf());
        self.favorite_ids.push(id);
        self.rebuild_locations();
    }

    pub fn remove_favorite_entries(&mut self, entries: &[usize]) {
        let Some(index) = self.favorites else {
            return;
        };
        for &entry in entries.iter().rev() {
            if entry < self.playlists[index].tracks.len() {
                self.playlists[index].tracks.remove(entry);
            }
            if entry < self.playlists[index].ids.len() {
                self.playlists[index].ids.remove(entry);
            }
            if entry < self.favorite_ids.len() {
                self.favorite_ids.remove(entry);
            }
        }
        self.rebuild_locations();
    }

    #[cfg(test)]
    pub fn toggle_favorite_path(&mut self, path: &Path) -> bool {
        let entries = self.favorite_entries(path);
        if entries.is_empty() {
            self.add_favorite(path);
            return true;
        }
        self.remove_favorite_entries(&entries);
        false
    }

    pub fn toggle_favorite_id(&mut self, id: TrackId) -> bool {
        let entries = self.favorite_id_entries(id);
        if !entries.is_empty() {
            self.remove_favorite_entries(&entries);
            return false;
        }
        if let Some(entry) = self
            .favorites
            .and_then(|index| self.playlists.get(index))
            .and_then(|playlist| playlist.ids.iter().position(|&entry| entry == id))
        {
            self.remove_favorite_entries(&[entry]);
            return false;
        }
        if let Some(path) = self.track_path(id).map(PathBuf::from) {
            let legacy = self.favorite_entries(&path);
            if !legacy.is_empty() {
                self.remove_favorite_entries(&legacy);
                return false;
            }
            self.add_favorite(&path);
            return true;
        }
        false
    }

    /// Drop favorites whose source no longer exists and refresh changed paths.
    /// Returns whether the Favorites playlist was actually rewritten, so the
    /// caller can invalidate cached rows and index jobs for that view.
    pub fn prune_favorites(&mut self) -> bool {
        let Some(index) = self.favorites else {
            return false;
        };
        let mut kept = Vec::new();
        for (entry, favorite) in self.favorite_ids.iter().enumerate() {
            let Some(id) = *favorite else {
                if let Some(path) = self.playlists[index].tracks.get(entry).cloned() {
                    kept.push((path, None));
                }
                continue;
            };
            if let Some(path) = self.track_path(id).map(PathBuf::from) {
                kept.push((path, Some(id)));
            }
        }
        let unchanged = kept.len() == self.playlists[index].tracks.len()
            && kept
                .iter()
                .zip(&self.playlists[index].tracks)
                .all(|((path, _), old)| same_path(path, old));
        if unchanged {
            return false;
        }
        self.playlists[index].tracks = kept.iter().map(|(path, _)| path.clone()).collect();
        self.playlists[index].ids.clear();
        self.favorite_ids = kept.into_iter().map(|(_, id)| id).collect();
        self.rebuild_locations();
        true
    }

    pub fn back_up_invalid_json(path: &Path, prefix: &str) -> std::io::Result<PathBuf> {
        let mut original = fs::File::open(path)?;
        let directory = path.parent().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "Missing settings folder")
        })?;
        let mut backup = tempfile::Builder::new()
            .prefix(prefix)
            .suffix(".json")
            .tempfile_in(directory)?;
        std::io::copy(&mut original, &mut backup)?;
        backup.as_file().sync_all()?;
        let (_, backup_path) = backup.keep()?;
        sync_directory(directory)?;
        Ok(backup_path)
    }
}

/// Flush the directory entry after a rename. Syncing the temporary file only
/// makes its contents durable; without a directory flush the rename itself can
/// be lost after a power failure. Windows needs a directory handle opened with
/// `FILE_FLAG_BACKUP_SEMANTICS`, while Unix can simply sync an open directory.
fn sync_directory(path: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::{
            Foundation::{CloseHandle, GENERIC_WRITE, HANDLE},
            Storage::FileSystem::{
                CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ,
                FILE_SHARE_WRITE, FlushFileBuffers, OPEN_EXISTING,
            },
        };
        // `encode_wide` keeps unpaired surrogates intact, unlike a lossy
        // conversion through `str`. `FlushFileBuffers` requires GENERIC_WRITE;
        // a read-only directory handle opens but cannot be flushed.
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        let handle = unsafe {
            CreateFileW(
                windows::core::PCWSTR(wide.as_ptr()),
                GENERIC_WRITE.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                HANDLE::default(),
            )
        }?;
        let result = unsafe { FlushFileBuffers(handle) }.map_err(std::io::Error::from);
        unsafe {
            let _ = CloseHandle(handle);
        }
        result
    }
    #[cfg(not(windows))]
    {
        fs::File::open(path)?.sync_all()
    }
}

/// Atomically write already serialized bytes. Kept separate from
/// [`save_json`] so a background writer can do the slow fsync while the UI
/// thread only prepares the snapshot.
pub fn save_bytes(bytes: &[u8], path: &Path) -> std::io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other("Settings path has no parent"))?;
    fs::create_dir_all(dir)?;
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    sync_directory(dir)?;
    Ok(())
}

pub fn save_json(value: &impl Serialize, path: &Path) -> std::io::Result<()> {
    save_bytes(&serde_json::to_vec(value)?, path)
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
    match crate::cue::entries_from_source(&path) {
        Ok(Some(entries)) => tracks.extend(entries),
        Ok(None) => tracks.push(path),
        Err(_) => *skipped += 1,
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
    /// Cheap title-only snapshot for the UI thread. The full tag and cover
    /// read runs on a worker and replaces this once it finishes. CUE entries
    /// resolve to their audio file, matching [`Media::read`].
    pub fn placeholder(path: &Path) -> Self {
        let cue = crate::cue::resolve(path).ok().flatten();
        let audio = cue.as_ref().map_or(path, |track| track.audio.as_path());
        Self {
            title: file_title(audio),
            ..Self::default()
        }
    }

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
        if media.cover.is_none()
            && let Some(dir) = path.parent()
        {
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
    fn mute_restores_saved_level_after_restart() -> std::io::Result<()> {
        let temp = tempfile::tempdir()?;
        let file = temp.path().join("library.json");
        let mut library = Library {
            volume: 0.12345,
            ..Default::default()
        };
        library.toggle_mute();
        library.save(&file)?;
        let mut restored = Library::load(&file)?;
        assert!(restored.muted);
        restored.toggle_mute();
        assert!(!restored.muted);
        assert_eq!(restored.volume, 0.12345);
        Ok(())
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
    fn serialization_failure_preserves_previous_file() -> std::io::Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("library.json");
        Library::default().save(&path)?;
        let before = fs::read(&path)?;
        let invalid = HashMap::from([((1, 2), true)]);
        assert!(save_json(&invalid, &path).is_err());
        assert_eq!(fs::read(&path)?, before);
        Ok(())
    }

    #[test]
    fn backup_retains_original_bytes_and_keeps_prior_backups() -> std::io::Result<()> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("library.json");
        let broken_json = b"{\xFF";
        fs::write(&path, broken_json)?;
        assert!(Library::load(&path).is_err());

        let first = Library::back_up_invalid_json(&path, "library.invalid-")?;
        let second = Library::back_up_invalid_json(&path, "library.invalid-")?;
        assert_ne!(first, second);
        assert_eq!(fs::read(&path)?, broken_json);

        Library::default().save(&path)?;
        assert_eq!(fs::read(first)?, broken_json);
        assert_eq!(fs::read(second)?, broken_json);
        assert!(Library::load(&path).is_ok());
        Ok(())
    }

    #[test]
    fn invalid_json_backup_uses_given_prefix() -> std::io::Result<()> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("playback.json");
        let broken_json = b"not-json";
        fs::write(&path, broken_json)?;

        let backup = Library::back_up_invalid_json(&path, "playback.invalid-")?;
        let name = backup
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        assert!(name.starts_with("playback.invalid-"));
        assert!(name.ends_with(".json"));
        assert_eq!(fs::read(&backup)?, broken_json);
        assert_eq!(fs::read(&path)?, broken_json);
        Ok(())
    }

    #[test]
    fn save_json_creates_missing_parent_directories() -> std::io::Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp
            .path()
            .join("nested")
            .join("settings")
            .join("library.json");
        assert!(!path.parent().unwrap().exists());

        Library::default().save(&path)?;

        assert!(path.is_file());
        assert_eq!(Library::load(&path)?.playlists.len(), 1);
        Ok(())
    }

    #[test]
    fn save_json_reports_error_when_parent_is_a_file() -> std::io::Result<()> {
        let temp = tempfile::tempdir()?;
        let parent = temp.path().join("not-a-directory");
        fs::write(&parent, b"occupied")?;
        let path = parent.join("library.json");

        assert!(Library::default().save(&path).is_err());
        assert!(!path.exists());
        assert_eq!(fs::read(&parent)?, b"occupied");
        Ok(())
    }

    #[test]
    fn save_json_failure_leaves_no_temporary_file() -> std::io::Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("library.json");
        fs::create_dir(&path)?;

        assert!(Library::default().save(&path).is_err());
        assert!(path.is_dir());
        assert_eq!(fs::read_dir(temp.path())?.count(), 1);
        Ok(())
    }

    #[test]
    fn save_bytes_persists_exact_payload_without_leaving_temporaries() -> std::io::Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("nested").join("state.json");
        let payload = br#"{"value":42}"#;

        super::save_bytes(payload, &path)?;

        assert_eq!(fs::read(&path)?, payload);
        assert_eq!(fs::read_dir(path.parent().unwrap())?.count(), 1);
        Ok(())
    }

    #[test]
    fn favorites_create_playlist_without_physical_folder() -> std::io::Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("library.json");
        let mut library = Library::default();
        assert_eq!(library.favorites, None);
        assert!(!library.is_favorite(Path::new("song.flac")));

        let index = library.favorites_index_or_create();
        assert_eq!(library.favorites, Some(index));
        assert_eq!(library.playlists[index].name, "Favorites");
        assert_eq!(library.playlists[index].sort, Sort::Album);
        assert!(!library.playlists[index].pinned);
        assert!(library.playlists[index].sidebar_position.is_none());

        library.save(&path)?;
        let entries: Vec<String> = fs::read_dir(temp.path())?
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries, ["library.json"]);
        Ok(())
    }

    #[test]
    fn favorites_toggle_adds_and_removes_all_duplicates() {
        let mut library = Library::default();
        library.playlists[0].tracks = vec![PathBuf::from("keep.flac"), PathBuf::from("song.flac")];
        library.rebuild_locations();
        let path = Path::new("song.flac");

        assert!(library.toggle_favorite_path(path));
        assert!(library.is_favorite(path));
        assert_eq!(library.favorite_entries(path), [0]);
        let index = library.favorites.unwrap();
        assert_eq!(library.playlists[index].ids.len(), 1);
        let favorite_id = library.playlists[index].ids[0];
        assert_eq!(*library.locations.get(&favorite_id).unwrap(), (index, 0));

        library.playlists[index].tracks.push(path.to_path_buf());
        library.rebuild_locations();
        assert_eq!(library.favorite_entries(path), [0, 1]);

        assert!(!library.toggle_favorite_path(path));
        assert!(!library.is_favorite(path));
        assert!(library.playlists[index].tracks.is_empty());
        assert!(library.playlists[index].ids.is_empty());
        assert_eq!(library.playlists[0].tracks.len(), 2);
        assert_eq!(library.playlists[0].ids.len(), 2);
        assert!(!library.locations.contains_key(&favorite_id));
    }

    #[test]
    fn favorite_path_lookup_follows_reordered_sources() {
        let mut library = Library::default();
        library.playlists[0].tracks = vec![PathBuf::from("a.flac"), PathBuf::from("song.flac")];
        library.rebuild_locations();
        library.toggle_favorite_path(Path::new("song.flac"));
        assert_eq!(library.favorite_entries(Path::new("song.flac")), [0]);
        #[cfg(windows)]
        assert_eq!(library.favorite_entries(Path::new("SONG.FLAC")), [0]);

        library.playlists[0].tracks.swap(0, 1);
        library.playlists[0].ids.swap(0, 1);
        library.rebuild_locations();
        assert_eq!(library.favorite_entries(Path::new("song.flac")), [0]);
        assert_eq!(library.path_sources.len(), 2);
        assert_eq!(library.favorite_paths.len(), 1);
    }

    #[test]
    fn favorites_persist_selection_sort_and_playable_ids() -> std::io::Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("library.json");
        let mut library = Library::default();
        library.playlists[0].tracks = vec![PathBuf::from("song.flac")];
        library.rebuild_locations();
        library.toggle_favorite_path(Path::new("song.flac"));
        let index = library.favorites.unwrap();
        library.playlists[index].sort = Sort::Title;
        library.playlists[index].descending = true;
        library.selected = index;
        library.save(&path)?;

        let loaded = Library::load(&path)?;
        assert_eq!(loaded.favorites, Some(index));
        assert_eq!(loaded.selected, index);
        assert_eq!(loaded.playlists[index].name, "Favorites");
        assert_eq!(loaded.playlists[index].tracks, [PathBuf::from("song.flac")]);
        assert_eq!(loaded.playlists[index].sort, Sort::Title);
        assert!(loaded.playlists[index].descending);
        assert!(loaded.is_favorite(Path::new("song.flac")));

        let id = loaded.playlists[index].ids[0];
        assert_eq!(*loaded.locations.get(&id).unwrap(), (index, 0));

        let infos = vec![
            TrackInfo {
                title: "A".into(),
                ..TrackInfo::default()
            },
            TrackInfo {
                title: "B".into(),
                ..TrackInfo::default()
            },
        ];
        let view = PlaylistView::build(
            infos,
            loaded.playlists[index].sort,
            loaded.playlists[index].descending,
        );
        assert_eq!(view.order, [1, 0]);
        Ok(())
    }

    #[test]
    fn favorites_name_does_not_clobber_existing_playlist() {
        let mut library = Library::default();
        library.playlists.push(Playlist {
            name: "Favorites".into(),
            tracks: Vec::new(),
            ids: Vec::new(),
            pinned: false,
            sidebar_position: None,
            sort: Sort::Added,
            descending: false,
            smart_query: None,
        });

        assert_eq!(library.unique_playlist_name("Favorites"), "Favorites (2)");
        let index = library.favorites_index_or_create();
        assert_eq!(library.playlists[index].name, "Favorites (2)");
        assert_eq!(library.favorites, Some(index));
    }
}
