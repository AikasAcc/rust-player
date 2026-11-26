#![windows_subsystem = "windows"]

mod artwork;
mod audio;
mod audio_setup;
mod audio_ui;
mod background;
#[cfg(test)]
mod benchmarks;
mod cue;
mod diagnostics;
mod digest;
mod fonts;
mod gain;
mod gapless_ui;
mod lastfm;
mod library;
mod listening;
mod lyrics;
mod media_keys;
mod metadata_cache;
mod play_queue;
mod playlist_io;
mod plugins;
mod presence;
mod relink;
mod save_worker;
mod settings;
mod tagging;
mod taskbar;
mod theme;
mod thumbnails;
#[cfg(test)]
mod track_id_tests;
mod ui;
mod update;
mod window_chrome;

use eframe::egui::{self, RichText};
use library::{
    Library, Media, Playlist, PlaylistRow, PlaylistView, Sort, TrackInfo, file_title, same_path,
};
use rand::seq::SliceRandom;
use std::{
    collections::HashMap,
    io::Write,
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};

use library::TrackId;
pub(crate) const PROJECT_URL: &str = "https://github.com/AikasAcc/rust-player";
type Track = (usize, usize);
type ImportResult = (Vec<PathBuf>, usize);
type PlaylistImportResult = Result<(Playlist, usize), String>;
type PlaylistExportResult = (String, Result<(), String>);

struct Playing {
    id: TrackId,
    counted: bool,
    lastfm: lastfm::Listen,
    sink: audio::AudioTrack,
    duration: Option<Duration>,
    media: Media,
    cover: Option<background::Cover>,
    online_cover: Option<background::Cover>,
    online_url: Option<String>,
    lookup: Option<mpsc::Receiver<Result<Option<artwork::Artwork>, &'static str>>>,
    artwork_status: &'static str,
    timed_lyrics: Vec<(Duration, String)>,
    lyrics_lookup: Option<mpsc::Receiver<Result<Option<String>, String>>>,
    lyrics_status: String,
    lyrics_online: bool,
}

impl Playing {
    fn cover(&self) -> Option<&background::Cover> {
        self.cover.as_ref().or(self.online_cover.as_ref())
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct SavedTrack {
    #[serde(default)]
    id: Option<TrackId>,
    track: Track,
    path: PathBuf,
    position: Duration,
}

enum PlaylistDialog {
    Name { target: Option<usize>, name: String },
    Delete(usize),
}

enum TrackEdit {
    Remove,
    Move(usize),
}

enum PlaylistAction {
    TogglePin,
    Rename,
    Copy,
    Delete,
    Export,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum ActivityTab {
    #[default]
    History,
    Songs,
    Artists,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InspectorTab {
    Queue,
    Track,
}

#[derive(Clone, Copy)]
enum HeaderAction {
    Settings,
    Files,
    Folder,
    Theme,
    Plugins,
    Discord,
    GitHub,
}

struct ThemeDraft {
    theme: theme::Theme,
    font: Option<PathBuf>,
    font_error: String,
    background: background::Settings,
    themes: Vec<theme::NamedTheme>,
    selected: Option<usize>,
    name: String,
    theme_error: String,
    copy_background: bool,
}

struct TrackProperties {
    path: PathBuf,
    title: String,
    artist: String,
    album: String,
    album_artist: String,
    track: String,
    duration: String,
    format: String,
    size: String,
    status: String,
    loading: bool,
}

struct CoverViewer {
    cover: background::Cover,
    title: String,
    artist: String,
}

struct Notice {
    message: String,
    started: Instant,
}

impl TrackProperties {
    /// Shown immediately while the tag read runs on a worker thread.
    fn loading(path: PathBuf) -> Self {
        Self {
            path,
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            album_artist: String::new(),
            track: String::new(),
            duration: String::new(),
            format: String::new(),
            size: String::new(),
            status: "Reading tags…".into(),
            loading: true,
        }
    }

    fn read(path: PathBuf) -> Self {
        let info = TrackInfo::read(&path);
        let track = info.track_number();
        let duration = info.duration.map(time).unwrap_or_else(|| "Unknown".into());
        let size = std::fs::metadata(&path)
            .map(|metadata| format_bytes(metadata.len()))
            .unwrap_or_else(|_| "Unavailable".into());
        let format = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.to_ascii_uppercase())
            .unwrap_or_else(|| "Unknown".into());
        Self {
            path,
            title: info.title,
            artist: info.artist,
            album: info.album,
            album_artist: info.album_artist,
            track,
            duration,
            format,
            size,
            status: String::new(),
            loading: false,
        }
    }
}

impl SavedTrack {
    /// Resolve a saved track to its stable identity. A stored ID always wins,
    /// so renames and reorders keep the same track. Old saves without a usable
    /// ID are matched by their original location and path; duplicates keep
    /// their own IDs.
    fn resolve(&self, library: &Library) -> Option<TrackId> {
        if let Some(id) = self.id
            && library.locate(id).is_some()
        {
            return Some(id);
        }
        let (list, _) = self.track;
        library
            .track_id(self.track)
            .filter(|&id| {
                library
                    .track_path(id)
                    .is_some_and(|path| same_path(path, &self.path))
            })
            .or_else(|| {
                library
                    .playlists
                    .get(list)
                    .and_then(|playlist| {
                        playlist
                            .tracks
                            .iter()
                            .position(|path| same_path(path, &self.path))
                            .map(|index| (list, index))
                    })
                    .and_then(|track| library.track_id(track))
            })
            .or_else(|| {
                library
                    .playlists
                    .iter()
                    .enumerate()
                    .find_map(|(list, playlist)| {
                        playlist
                            .tracks
                            .iter()
                            .position(|path| same_path(path, &self.path))
                            .and_then(|index| library.track_id((list, index)))
                    })
            })
    }
}

/// Keep only IDs that still exist and point the cursor at the same entry, or
/// at the next entry when the playing track itself was removed.
fn remap_order(
    order: &mut Vec<TrackId>,
    locations: &HashMap<TrackId, Track>,
    cursor: usize,
) -> usize {
    let mut position = 0usize;
    let mut preceding = 0usize;
    order.retain(|id| {
        let keep = locations.contains_key(id);
        if keep && cursor != usize::MAX && position < cursor {
            preceding += 1;
        }
        position += 1;
        keep
    });
    if order.is_empty() {
        usize::MAX
    } else {
        preceding.min(order.len() - 1)
    }
}

struct SearchResults {
    list: usize,
    query: String,
    rows: Vec<PlaylistRow>,
    count: usize,
}

impl SearchResults {
    fn new(list: usize, query: &str, playlist: &Playlist, view: Option<&PlaylistView>) -> Self {
        let needle = query.to_lowercase();
        let mut rows = Vec::new();
        let mut count = 0;
        let mut indexed = 0;
        if let Some(view) = view {
            indexed = view.tracks.len();
            let mut album = None;
            for row in &view.rows {
                match row {
                    PlaylistRow::Album(label) => album = Some(label),
                    PlaylistRow::Track(index) => {
                        let track = &view.tracks[*index];
                        if !needle.contains('\0') && track.search_key.contains(&needle) {
                            if let Some(label) = album.take() {
                                rows.push(PlaylistRow::Album(label.clone()));
                            }
                            rows.push(PlaylistRow::Track(*index));
                            count += 1;
                        }
                    }
                }
            }
        }
        for (index, path) in playlist.tracks.iter().enumerate().skip(indexed) {
            if file_title(path).to_lowercase().contains(&needle) {
                rows.push(PlaylistRow::Track(index));
                count += 1;
            }
        }
        Self {
            list,
            query: query.into(),
            rows,
            count,
        }
    }
}

#[derive(Default)]
struct Dialogs {
    playlist_dialog: Option<PlaylistDialog>,
    track_properties: Option<TrackProperties>,
    cover_viewer: Option<CoverViewer>,
    activity_open: bool,
    plugins_open: bool,
    settings_page: Option<settings::Page>,
    activity_tab: ActivityTab,
    discord_settings: bool,
}

struct Player {
    relink: Option<mpsc::Receiver<relink::ResultSet>>,
    gain_jobs: HashMap<PathBuf, mpsc::Receiver<Result<gain::Report, String>>>,
    media_job: Option<(u64, mpsc::Receiver<Media>)>,
    media_generation: u64,
    property_jobs: HashMap<PathBuf, mpsc::Receiver<TrackProperties>>,
    cached_favorite: Option<(TrackId, bool)>,
    search_deadline: Option<Instant>,
    dialogs: Dialogs,
    indexer: metadata_cache::Indexer,
    save_worker: save_worker::SaveWorker,
    background: background::Preview,
    setup: audio_setup::Setup,
    audio_devices: Option<audio::DeviceMonitor>,
    audio_retry_after: Option<Instant>,
    gapless_generation: u64,
    audio_release: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    playing: Option<Playing>,
    library: Library,
    library_path: PathBuf,
    backup: save_worker::BackupTracker,
    restore_track: bool,
    volume_changed: bool,
    volume_save_at: Option<Instant>,
    status: String,
    notice: Option<Notice>,
    updater: update::State,
    thumbs: thumbnails::Cache,

    listening: listening::Listening,
    listening_ranks: Option<listening::Rankings>,

    theme_draft: Option<ThemeDraft>,
    plugins: plugins::Host,

    taskbar: Option<taskbar::Taskbar>,
    window_chrome: Option<window_chrome::NativeChrome>,
    media_keys: Result<media_keys::MediaKeys, String>,

    artwork_url: String,
    track_search: String,
    search_results: Option<SearchResults>,
    rpc: presence::Presence,
    lastfm: lastfm::Client,
    order: Vec<TrackId>,
    cursor: usize,
    order_list: Option<usize>,
    queue_order_edited: bool,
    play_queue: play_queue::PlayQueue,
    inspector_tab: InspectorTab,
    imports: Vec<(usize, mpsc::Receiver<ImportResult>)>,
    playlist_imports: Vec<mpsc::Receiver<Vec<PlaylistImportResult>>>,
    playlist_exports: Vec<mpsc::Receiver<PlaylistExportResult>>,
    views: HashMap<usize, PlaylistView>,
    indexing: HashMap<usize, mpsc::Receiver<Vec<TrackInfo>>>,
    /// Opt-in UI frame timing: set RUST_PLAYER_FRAME_LOG to a CSV path.
    frame_log: Option<std::io::BufWriter<std::fs::File>>,
    frame_log_count: usize,
}

impl Player {
    fn new(ctx: &egui::Context) -> Self {
        let library_path = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("Rust Player")
            .join("library.json");
        let (library, library_needs_backup, mut status) = match Library::load(&library_path) {
            Ok(library) => (library, false, String::new()),
            Err(error) => (
                Library::default(),
                true,
                format!("Could not load playlists: {error}"),
            ),
        };
        let (listening, listening_needs_backup) =
            match listening::Listening::load(&library_path.with_file_name("listening.json")) {
                Ok(data) => (data, false),
                Err(error) => {
                    status = format!("Could not load listening activity: {error}");
                    (listening::Listening::default(), true)
                }
            };
        let plugins = plugins::Host::load(&library.plugins);
        let lastfm = lastfm::Client::new(ctx, &library_path, &library);
        let setup = audio_setup::Setup::new(!library_path.exists() && !library.audio_setup_done);
        let mut updater = update::State::default();
        if library.check_for_updates {
            updater.check(ctx, env!("CARGO_PKG_VERSION"));
        }
        let mut backup = save_worker::BackupTracker::default();
        if library_needs_backup {
            backup.need(library_path.clone());
        }
        if listening_needs_backup {
            backup.need(library_path.with_file_name("listening.json"));
        }
        let mut frame_log = std::env::var_os("RUST_PLAYER_FRAME_LOG").and_then(|path| {
            std::fs::File::create(path)
                .ok()
                .map(|file| std::io::BufWriter::with_capacity(64 * 1024, file))
        });
        if let Some(log) = &mut frame_log {
            let _ = writeln!(log, "frame,micros");
        }
        let player = Self {
            relink: None,
            gain_jobs: HashMap::new(),
            media_job: None,
            media_generation: 0,
            property_jobs: HashMap::new(),
            cached_favorite: None,
            search_deadline: None,
            background: background::Preview::default(),
            indexer: metadata_cache::Indexer::new(
                library_path.with_file_name("metadata-cache.json"),
            ),
            save_worker: save_worker::SaveWorker::new(),
            dialogs: Default::default(),
            setup,
            audio_devices: None,
            audio_retry_after: None,
            gapless_generation: 0,
            search_results: None,
            order_list: None,
            queue_order_edited: false,
            play_queue: Default::default(),
            inspector_tab: InspectorTab::Queue,
            audio_release: None,
            playing: None,
            library,
            library_path,
            backup,
            restore_track: true,
            volume_changed: false,
            volume_save_at: None,
            status,
            notice: None,
            updater,
            thumbs: thumbnails::Cache::new(ctx),

            listening,
            listening_ranks: None,

            theme_draft: None,
            plugins,

            taskbar: None,
            window_chrome: None,
            media_keys: Err("Windows media session not initialized".into()),

            artwork_url: String::new(),
            track_search: String::new(),
            rpc: presence::Presence::new(),
            lastfm,
            order: Vec::new(),
            cursor: usize::MAX,
            imports: Vec::new(),
            playlist_imports: Vec::new(),
            playlist_exports: Vec::new(),
            views: HashMap::new(),
            indexing: HashMap::new(),
            frame_log,
            frame_log_count: 0,
        };
        player.sync_cache_paths();
        player
    }

    fn sync_cache_paths(&self) {
        let paths = self
            .library
            .playlists
            .iter()
            .flat_map(|playlist| playlist.tracks.iter().cloned())
            .collect();
        self.indexer.set_paths(paths);
    }

    fn save(&mut self) {
        self.cached_favorite = None;
        self.search_results = None;
        self.volume_save_at = None;
        self.library.prune_favorites();
        self.sync_cache_paths();
        let mut notices = Vec::new();
        if !self.prepare_backup(
            self.library_path.clone(),
            "library.invalid-",
            "settings",
            &mut notices,
        ) {
            return;
        }
        let listening_path = self.library_path.with_file_name("listening.json");
        if !self.prepare_backup(
            listening_path,
            "listening.invalid-",
            "listening activity",
            &mut notices,
        ) {
            return;
        }
        // Serialize on the UI thread, let the worker do the atomic write.
        // The snapshot belongs to the state right now, so a later save simply
        // replaces it in the worker's queue. The backup stays pending until the
        // worker confirms that the replacement reached the disk.
        match serde_json::to_vec(&self.library) {
            Ok(bytes) => {
                self.save_worker.enqueue(self.library_path.clone(), bytes);
                self.volume_changed = false;
                if !notices.is_empty() {
                    self.status = notices.join(" ");
                }
            }
            Err(error) => {
                notices.push(format!("Could not save playlists or settings: {error}"));
                self.status = notices.join(" ");
            }
        }
    }

    /// Copy a corrupt file before its replacement is written. `false` aborts
    /// the save so the original stays untouched for a later retry.
    fn prepare_backup(
        &mut self,
        path: PathBuf,
        prefix: &str,
        label: &str,
        notices: &mut Vec<String>,
    ) -> bool {
        if !self.backup.should_back_up(&path) {
            return true;
        }
        match Library::back_up_invalid_json(&path, prefix) {
            Ok(backup) => {
                self.backup.mark_backed_up(path);
                notices.push(format!(
                    "Saved a copy of unreadable {label} to {}",
                    backup.display()
                ));
                true
            }
            Err(error) => {
                self.status = format!("Could not back up unreadable {label}: {error}");
                false
            }
        }
    }

    fn volume_adjusted(&mut self) {
        self.volume_changed = true;
        self.volume_save_at = Some(Instant::now() + Duration::from_millis(300));
        if let Some(playing) = &self.playing {
            playing.sink.set_volume(self.effective_volume());
        }
    }

    fn effective_volume(&self) -> f32 {
        if !self.library.muted {
            self.library.volume
        } else {
            Default::default()
        }
    }

    fn toggle_mute(&mut self) {
        self.library.toggle_mute();
        self.volume_adjusted();
    }

    fn show_notice(&mut self, message: impl Into<String>) {
        self.notice = Some(Notice {
            message: message.into(),
            started: Instant::now(),
        });
    }

    fn flush_volume(&mut self, ctx: &egui::Context) {
        if let Some(deadline) = self.volume_save_at {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                self.save();
            } else {
                ctx.request_repaint_after(remaining);
            }
        }
    }

    fn read_playback(&mut self) -> Option<SavedTrack> {
        let path = self.library_path.with_file_name("playback.json");
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
            Err(error) => {
                self.status = format!("Could not read saved playback: {error}");
                return None;
            }
        };
        let mut saved: SavedTrack = match serde_json::from_slice(&bytes) {
            Ok(Some(saved)) => saved,
            Ok(None) => return None,
            Err(error) => {
                self.status = format!("Could not read saved playback: {error}");
                self.backup.need(path.clone());
                self.back_up_invalid_playback(&path);
                return None;
            }
        };
        let missing_id = saved.id.is_none();
        let id = saved.resolve(&self.library)?;
        let track = self.library.locate(id)?;
        saved.id = Some(id);
        saved.track = track;
        saved.path = self
            .library
            .track_path(id)
            .map(PathBuf::from)
            .unwrap_or_default();
        if missing_id {
            self.write_playback(Some(saved.clone()));
        }
        Some(saved)
    }

    fn restore_playback(&mut self, ctx: &egui::Context) {
        let Some(saved) = self.read_playback() else {
            return;
        };
        let Some(id) = saved.id else {
            return;
        };
        let Some(track) = self.library.locate(id) else {
            return;
        };
        match self.open(id, ctx, Some(saved.position)) {
            Ok(()) => {
                self.library.selected = track.0;
                self.plan(id);
            }
            Err(error) => self.status = format!("Could not restore last track: {error}"),
        }
    }

    fn back_up_invalid_playback(&mut self, path: &std::path::Path) {
        if !self.backup.should_back_up(path) {
            return;
        }
        match Library::back_up_invalid_json(path, "playback.invalid-") {
            Ok(backup) => {
                self.backup.mark_backed_up(path.to_path_buf());
                self.status = format!(
                    "Saved a copy of unreadable playback state to {}",
                    backup.display()
                );
            }
            Err(error) => {
                self.status = format!("Could not back up unreadable playback state: {error}");
            }
        }
    }

    fn save_playback(&mut self, position: Duration) {
        let Some(playing) = &self.playing else { return };
        let id = playing.id;
        // The coordinates are only written for older builds; identity is the ID.
        let track = self.library.locate(id).unwrap_or((0, 0));
        let saved = SavedTrack {
            id: Some(id),
            track,
            path: self
                .library
                .track_path(id)
                .map(PathBuf::from)
                .unwrap_or_default(),
            position,
        };
        self.write_playback(Some(saved));
    }

    fn write_playback(&mut self, saved: Option<SavedTrack>) {
        let path = self.library_path.with_file_name("playback.json");
        let mut notices = Vec::new();
        if !self.prepare_backup(
            path.clone(),
            "playback.invalid-",
            "playback state",
            &mut notices,
        ) {
            return;
        }
        match serde_json::to_vec(&saved) {
            Ok(bytes) => {
                self.save_worker.enqueue(path, bytes);
                if !notices.is_empty() {
                    self.status = notices.join(" ");
                }
            }
            Err(error) => {
                notices.push(format!("Could not save playback: {error}"));
                self.status = notices.join(" ");
            }
        }
    }

    fn previously_saved(&mut self) -> Option<SavedTrack> {
        self.read_playback()
    }

    fn refresh_playback_ids(&mut self, saved: Option<SavedTrack>) {
        let locations = &self.library.locations;
        self.play_queue.retain(|id| locations.contains_key(&id));
        let cursor = self.cursor;
        self.cursor = remap_order(&mut self.order, locations, cursor);
        self.order_list = self
            .order
            .first()
            .and_then(|id| locations.get(id))
            .map(|loc| loc.0);
        if let Some(playing) = &mut self.playing {
            if locations.contains_key(&playing.id) {
                // The ID survives the reorder; no coordinate rebase is needed.
                let position = playing.sink.get_pos();
                self.save_playback(position);
                return;
            }
            self.observe_lastfm();
            self.lastfm.track_changed();
            self.playing = None;
            self.clear_order();
            self.sync_presence();
            self.write_playback(None);
        } else if let Some(mut saved) = saved {
            if let Some(id) = saved.resolve(&self.library) {
                saved.id = Some(id);
                saved.track = self.library.locate(id).unwrap_or(saved.track);
                saved.path = self
                    .library
                    .track_path(id)
                    .map(PathBuf::from)
                    .unwrap_or(saved.path);
                self.write_playback(Some(saved));
            } else {
                self.write_playback(None);
            }
        }
    }

    fn delete_playlist(&mut self, list: usize) {
        let saved = self.previously_saved();
        let rebase = |index: usize| match index.cmp(&list) {
            std::cmp::Ordering::Less => Some(index),
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(index - 1),
        };
        self.library.playlists.remove(list);
        self.library.favorites = self.library.favorites.and_then(rebase);
        if self.library.playlists.is_empty() {
            self.library.playlists = Library::default().playlists;
        }
        self.library.selected = rebase(self.library.selected)
            .unwrap_or_else(|| list.min(self.library.playlists.len() - 1));
        self.views.clear();
        self.indexing.clear();
        self.imports.retain_mut(|(index, _)| {
            if let Some(new) = rebase(*index) {
                *index = new;
                true
            } else {
                false
            }
        });
        self.library.rebuild_locations();
        self.save();
        self.refresh_playback_ids(saved);
    }

    fn unique_playlist_name(&self, name: &str) -> String {
        self.library.unique_playlist_name(name)
    }

    fn playlist_action(&mut self, index: usize, action: PlaylistAction, ctx: &egui::Context) {
        match action {
            PlaylistAction::TogglePin => {
                let playlist = &mut self.library.playlists[index];
                playlist.pinned = !playlist.pinned;
                self.save();
            }
            PlaylistAction::Rename => {
                self.dialogs.playlist_dialog = Some(PlaylistDialog::Name {
                    target: Some(index),
                    name: self.library.playlists[index].name.clone(),
                });
            }
            PlaylistAction::Copy => {
                let mut playlist = self.library.playlists[index].clone();
                playlist.ids.clear();
                playlist.pinned = false;
                playlist.sidebar_position = None;
                playlist.name = self.unique_playlist_name(&format!("{} (Copy)", playlist.name));
                self.library.playlists.push(playlist);
                self.library.rebuild_locations();
                self.library.selected = self.library.playlists.len() - 1;
                self.save();
            }
            PlaylistAction::Delete => {
                self.dialogs.playlist_dialog = Some(PlaylistDialog::Delete(index))
            }
            PlaylistAction::Export => {
                let playlist = self.library.playlists[index].clone();
                let name = playlist
                    .name
                    .replace(['<', '>', ':', '"', '/', '\\', '|', '?', '*'], "_");
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("M3U8 playlist", &["m3u8"])
                    .set_file_name(format!("{name}.m3u8"))
                    .save_file()
                {
                    let (send, receive) = mpsc::channel();
                    self.playlist_exports.push(receive);
                    let label = playlist.name.clone();
                    let ctx = ctx.clone();
                    std::thread::spawn(move || {
                        let result = playlist_io::export(&playlist, &path)
                            .map_err(|error| error.to_string());
                        let _ = send.send((label, result));
                        ctx.request_repaint();
                    });
                    self.status = "Exporting playlist…".into();
                }
            }
        }
    }

    fn import_playlists(&mut self, ctx: &egui::Context) {
        let Some(paths) = rfd::FileDialog::new()
            .add_filter("Playlist files", playlist_io::EXTENSIONS)
            .pick_files()
        else {
            return;
        };
        let (send, receive) = mpsc::channel();
        self.playlist_imports.push(receive);
        let ctx = ctx.clone();
        self.status = "Importing playlists…".into();
        std::thread::spawn(move || {
            let results = paths
                .into_iter()
                .map(|path| {
                    playlist_io::import(&path)
                        .map_err(|error| format!("{}: {error}", path.display()))
                })
                .collect();
            let _ = send.send(results);
            ctx.request_repaint();
        });
    }

    /// Apply playlist file I/O results on the UI thread. Parsing and writing
    /// happened on worker threads; only library mutation and status stay here.
    fn finish_playlist_io(&mut self) {
        let mut imports = Vec::new();
        self.playlist_imports.retain(|rx| match rx.try_recv() {
            Ok(results) => {
                imports.push(results);
                false
            }
            Err(mpsc::TryRecvError::Disconnected) => false,
            Err(mpsc::TryRecvError::Empty) => true,
        });
        for results in imports {
            let mut imported = 0;
            let mut skipped = 0;
            let mut errors = Vec::new();
            for result in results {
                match result {
                    Ok((mut playlist, ignored)) => {
                        playlist.name = self.unique_playlist_name(&playlist.name);
                        self.library.playlists.push(playlist);
                        self.library.rebuild_locations();
                        self.library.selected = self.library.playlists.len() - 1;
                        imported += 1;
                        skipped += ignored;
                    }
                    Err(error) => errors.push(error),
                }
            }
            self.status = format!("Imported {imported} playlists");
            if skipped > 0 {
                self.status
                    .push_str(&format!(" · Skipped {skipped} unsupported entries"));
            }
            if !errors.is_empty() {
                self.status.push_str(&format!(" · {}", errors.join(" · ")));
            }
            if imported > 0 {
                self.save();
            }
        }

        let mut exports = Vec::new();
        self.playlist_exports.retain(|rx| match rx.try_recv() {
            Ok(result) => {
                exports.push(result);
                false
            }
            Err(mpsc::TryRecvError::Disconnected) => false,
            Err(mpsc::TryRecvError::Empty) => true,
        });
        for (name, result) in exports {
            self.status = match result {
                Ok(()) => format!("Exported “{name}”"),
                Err(error) => format!("Could not export playlist: {error}"),
            };
        }
    }

    fn toggle_favorite(&mut self) {
        if let Some(playing) = &self.playing {
            let id = playing.id;
            self.toggle_favorite_id(id);
        }
    }

    /// Toggle a track by identity, whether it is currently playing or only a row.
    fn toggle_favorite_id(&mut self, id: TrackId) {
        let playing_in_favorites = self
            .playing
            .as_ref()
            .is_some_and(|playing| playing.id == id);
        let path = self
            .library
            .track_path(id)
            .map(|path| path.to_path_buf())
            .unwrap_or_default();
        let index = self.library.favorites_index_or_create();
        let matches = self.library.favorite_entries(&path);
        if matches.is_empty() {
            let notice = format!("Added to {}", self.library.playlists[index].name);
            assert!(self.library.toggle_favorite_id(id));
            self.save();
            self.show_notice(notice);
            return;
        }

        let notice = format!("Removed from {}", self.library.playlists[index].name);
        if playing_in_favorites {
            // The sink keeps playing the file. Point the identity at the same
            // path in another playlist before the favorite entry disappears.
            let replacement = self
                .library
                .playlists
                .iter()
                .enumerate()
                .filter(|(list, _)| *list != index)
                .find_map(|(list, playlist)| {
                    playlist
                        .tracks
                        .iter()
                        .position(|track| same_path(track, &path))
                        .map(|entry| (list, entry))
                })
                .and_then(|track| self.library.track_id(track));
            if let Some(new_id) = replacement {
                self.playing.as_mut().expect("active track").id = new_id;
                self.plan(new_id);
            }
        }
        let saved = self.previously_saved();
        assert!(!self.library.toggle_favorite_id(id));
        self.indexing.remove(&index);
        if let Some(view) = self.views.remove(&index) {
            let mut metadata = view.tracks;
            for entry in matches.iter().rev() {
                if *entry < metadata.len() {
                    metadata.remove(*entry);
                }
            }
            let playlist = &self.library.playlists[index];
            self.views.insert(
                index,
                PlaylistView::build(metadata, playlist.sort, playlist.descending),
            );
        }
        self.save();
        self.refresh_playback_ids(saved);
        self.show_notice(notice);
    }

    fn record_play(&mut self) {
        let Some(playing) = &mut self.playing else {
            return;
        };
        if playing.counted || playing.sink.is_paused() || playing.sink.output.is_none() {
            return;
        }
        playing.counted = true;
        let Some(path) = self
            .library
            .track_path(playing.id)
            .map(|path| path.to_path_buf())
        else {
            return;
        };
        self.listening
            .record(path, &playing.media.title, &playing.media.artist);
        self.listening_ranks = None;
        let path = self.library_path.with_file_name("listening.json");
        let mut notices = Vec::new();
        if !self.prepare_backup(
            path.clone(),
            "listening.invalid-",
            "listening activity",
            &mut notices,
        ) {
            return;
        }
        match serde_json::to_vec(&self.listening) {
            Ok(bytes) => {
                self.save_worker.enqueue(path, bytes);
                if !notices.is_empty() {
                    self.status = notices.join(" ");
                }
            }
            Err(error) => self.status = format!("Could not save listening activity: {error}"),
        }
    }

    fn edit_track(&mut self, (list, index): Track, edit: TrackEdit) {
        let saved = self.previously_saved();
        let playlist = &mut self.library.playlists[list];
        match edit {
            TrackEdit::Remove => {
                playlist.ids.remove(index);
            }
            TrackEdit::Move(other) => playlist.ids.swap(index, other),
        }
        let tracks = &mut playlist.tracks;
        match edit {
            TrackEdit::Remove => {
                tracks.remove(index);
            }
            TrackEdit::Move(other) => tracks.swap(index, other),
        }
        if Some(list) == self.library.favorites {
            match edit {
                TrackEdit::Remove => {
                    if index < self.library.favorite_ids.len() {
                        self.library.favorite_ids.remove(index);
                    }
                }
                TrackEdit::Move(other) => self.library.favorite_ids.swap(index, other),
            }
        }
        self.indexing.remove(&list);
        if let Some(view) = self.views.remove(&list) {
            let mut metadata = view.tracks;
            match edit {
                TrackEdit::Remove if index < metadata.len() => {
                    metadata.remove(index);
                }
                TrackEdit::Move(other) if index.max(other) < metadata.len() => {
                    metadata.swap(index, other)
                }
                TrackEdit::Move(other) if index.min(other) < metadata.len() => {
                    metadata.truncate(index.min(other))
                }
                _ => {}
            }
            let playlist = &self.library.playlists[list];
            self.views.insert(
                list,
                PlaylistView::build(metadata, playlist.sort, playlist.descending),
            );
        }
        self.library.rebuild_locations();
        self.save();
        self.refresh_playback_ids(saved);
        if matches!(edit, TrackEdit::Move(_))
            && !self.library.shuffle
            && self.order_list == Some(list)
            && let Some(id) = self.order_id()
        {
            self.plan(id);
        }
    }

    fn open_containing_folder(&mut self, path: PathBuf) {
        let folder = path.parent().map(PathBuf::from).unwrap_or(path);
        let result = std::process::Command::new("explorer.exe")
            .arg(folder)
            .spawn()
            .map(|_| ());
        if let Err(error) = result {
            self.status = format!("Could not open the containing folder: {error}");
        }
    }

    fn import(&mut self, paths: Vec<PathBuf>, ctx: &egui::Context) {
        if paths.is_empty() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        self.imports.push((self.library.selected, rx));
        self.status = "Importing audio…".into();
        std::thread::spawn(move || {
            let _ = tx.send(library::collect(paths));
            ctx.request_repaint();
        });
    }

    fn finish_imports(&mut self) {
        let mut index = 0;
        while index < self.imports.len() {
            let (list, rx) = &self.imports[index];
            let list = *list;
            match rx.try_recv() {
                Ok((tracks, skipped)) => {
                    self.imports.remove(index);
                    let count = tracks.len();
                    let playlist = &mut self.library.playlists[list];
                    let first_new = playlist.tracks.len();
                    playlist.tracks.extend(tracks);
                    self.library.rebuild_locations();
                    let playlist = &self.library.playlists[list];
                    if self.order_list == Some(list) {
                        let mut extra: Vec<_> =
                            playlist.ids.iter().skip(first_new).copied().collect();
                        if self.library.shuffle {
                            extra.shuffle(&mut rand::rng());
                        }
                        self.order.extend(extra);
                    }
                    self.status = if skipped == 0 {
                        format!("Added {count} tracks")
                    } else {
                        format!(
                            "Added {count} tracks. Skipped {skipped} unsupported or unreadable items."
                        )
                    };
                    self.save();
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.imports.remove(index);
                    self.status = "Import could not be completed.".into();
                }
                Err(mpsc::TryRecvError::Empty) => index += 1,
            }
        }
    }

    /// Build the playback order around a track identity. Coordinates are
    /// looked up once here; no caller has to keep a `(list, index)` in sync
    /// with the library after a reorder.
    fn plan(&mut self, id: TrackId) {
        let Some((list, index)) = self.library.locate(id) else {
            return;
        };
        self.queue_order_edited = false;
        self.order_list = Some(list);
        let ids = &self.library.playlists[list].ids;
        let len = ids.len();
        self.order.clear();
        if self.library.shuffle {
            self.order
                .extend(ids.iter().copied().filter(|&item| item != id));
            self.order.shuffle(&mut rand::rng());
            self.order.insert(0, id);
            self.cursor = 0;
        } else if let Some(view) = self.views.get(&list) {
            self.order.extend(view.order.iter().map(|&i| ids[i]));
            self.order.extend_from_slice(&ids[view.tracks.len()..len]);
            self.cursor = self
                .order
                .iter()
                .position(|&item| item == id)
                .unwrap_or(index);
        } else {
            self.order.extend_from_slice(ids);
            self.cursor = index;
        }
    }

    /// Re-read all playlist views after a tag or ReplayGain change.
    fn refresh_metadata(&mut self, ctx: &egui::Context) {
        for list in 0..self.library.playlists.len() {
            self.views.remove(&list);
            self.indexing.remove(&list);
            self.request_index(list, ctx);
        }
        self.search_results = None;
    }

    /// Collect background ReplayGain scans without blocking the UI thread.
    fn finish_gain_jobs(&mut self, ctx: &egui::Context) {
        let mut finished = Vec::new();
        self.gain_jobs.retain(|path, rx| match rx.try_recv() {
            Ok(result) => {
                finished.push((path.clone(), result));
                false
            }
            Err(mpsc::TryRecvError::Disconnected) => false,
            Err(mpsc::TryRecvError::Empty) => true,
        });
        for (path, result) in finished {
            let status = match result {
                Ok(report) => format!(
                    "Wrote {:+.2} dB (peak {:.6}, {:.1} LUFS)",
                    report.gain_db, report.peak, report.loudness_lufs
                ),
                Err(error) => format!("Scan failed: {error}"),
            };
            if self
                .dialogs
                .track_properties
                .as_ref()
                .is_some_and(|properties| properties.path == path)
                && let Some(properties) = &mut self.dialogs.track_properties
            {
                properties.status.clone_from(&status);
            }
            self.show_notice(&status);
            self.refresh_metadata(ctx);
            ctx.request_repaint();
        }
    }

    /// Report disk failures and retire pending backups on the next frame.
    fn finish_saves(&mut self) {
        for report in self.save_worker.take_reports() {
            match report.error {
                Some(error) => {
                    // The corrupt original is still on disk. Require a fresh
                    // backup before the next attempt can overwrite it.
                    self.backup.failed(report.path);
                    self.status = error;
                }
                None => self.backup.written(&report.path),
            }
        }
    }

    fn request_index(&mut self, list: usize, ctx: &egui::Context) {
        let playlist = &self.library.playlists[list];
        let cached = self.views.get(&list).map_or(0, |view| view.tracks.len());
        if cached >= playlist.tracks.len() || self.indexing.contains_key(&list) {
            return;
        }
        let tracks = playlist.tracks[cached..].to_vec();

        self.indexing
            .insert(list, self.indexer.request(tracks, ctx));
    }

    fn finish_indexes(&mut self) {
        let mut changed = Vec::new();
        self.indexing.retain(|&list, rx| match rx.try_recv() {
            Ok(tracks) => {
                let playlist = &self.library.playlists[list];
                let mut metadata = self
                    .views
                    .remove(&list)
                    .map_or_else(Vec::new, |view| view.tracks);
                metadata.extend(tracks);
                self.views.insert(
                    list,
                    PlaylistView::build(metadata, playlist.sort, playlist.descending),
                );
                changed.push(list);
                false
            }
            Err(mpsc::TryRecvError::Disconnected) => false,
            Err(mpsc::TryRecvError::Empty) => true,
        });
        if !changed.is_empty() {
            self.search_results = None;
        }
        if let Some(id) = self.order_id()
            && !self.queue_order_edited
            && !self.library.shuffle
            && self
                .library
                .locate(id)
                .is_some_and(|track| changed.contains(&track.0))
        {
            self.plan(id);
        }
    }

    /// Playback entry point. Locations are derived and may move, the ID is not.
    fn open(
        &mut self,
        id: TrackId,
        ctx: &egui::Context,
        restore: Option<Duration>,
    ) -> Result<(), String> {
        let track = self
            .library
            .locate(id)
            .ok_or_else(|| "Track is no longer in the library".to_string())?;
        self.setup.stop_tone();
        let path = &self.library.playlists[track.0].tracks[track.1];
        if !relink::exists(path) {
            if let Some(info) = self
                .views
                .get_mut(&track.0)
                .and_then(|v| v.tracks.get_mut(track.1))
            {
                info.missing = true;
            }
            return Err(format!("Missing file: {}", path.display()));
        }
        let sink = audio::AudioTrack::open(
            path,
            self.output_request(),
            restore.unwrap_or_default(),
            self.effective_volume(),
            true,
            ctx,
        )?;
        self.audio_release = Some(sink.release_gate());
        self.audio_retry_after = None;
        if let Some(previous) = &self.playing {
            previous.sink.pause();
        }
        self.observe_lastfm();
        self.show_track(id, sink, ctx);
        if restore.is_none()
            && let Some(playing) = &self.playing
        {
            playing.sink.play();
        }
        Ok(())
    }

    fn show_track(&mut self, id: TrackId, sink: audio::AudioTrack, ctx: &egui::Context) {
        let Some(track) = self.library.locate(id) else {
            return;
        };
        let duration = sink.duration;
        let path = &self.library.playlists[track.0].tracks[track.1];
        // Tags, embedded cover and sidecar artwork are the slow part of a
        // track change. Show a title-only snapshot now; finish_media_job()
        // replaces it as soon as the worker returns.
        let media = Media::placeholder(path);
        self.artwork_url = self
            .library
            .artwork_urls
            .get(path)
            .cloned()
            .unwrap_or_default();
        self.gapless_generation = self.gapless_generation.wrapping_add(1);
        self.media_generation = self.media_generation.wrapping_add(1);
        let generation = self.media_generation;
        let (send, receive) = mpsc::channel();
        self.media_job = Some((generation, receive));
        let read_path = path.clone();
        let paint = ctx.clone();
        std::thread::spawn(move || {
            let media = Media::read(&read_path);
            let _ = send.send(media);
            paint.request_repaint();
        });
        self.lastfm.track_changed();
        self.playing = Some(Playing {
            id,
            counted: false,
            lastfm: lastfm::Listen::default(),
            sink,
            duration,
            media,
            cover: None,
            online_cover: None,
            online_url: None,
            lookup: None,
            artwork_status: "",
            timed_lyrics: Vec::new(),
            lyrics_lookup: None,
            lyrics_status: String::new(),
            lyrics_online: false,
        });
        if let Ok(media_keys) = &mut self.media_keys {
            media_keys.track_changed();
        }
        // Plugins, artwork and online lyrics need the real tags, so they
        // start in finish_media_job() instead of seeing the placeholder.
    }

    fn lookup_lyrics(&mut self, ctx: &egui::Context) {
        let Some(playing) = &mut self.playing else {
            return;
        };
        if playing.lyrics_lookup.is_some() {
            return;
        }
        let title = playing.media.title.clone();
        let artist = playing.media.artist.clone();
        let album = playing.media.album.clone();
        let duration = playing.duration;
        let (tx, rx) = mpsc::channel();
        playing.lyrics_lookup = Some(rx);
        playing.lyrics_status = "Searching LRCLIB…".into();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(lyrics::lookup(&title, &artist, &album, duration));
            ctx.request_repaint();
        });
    }

    fn finish_lyrics(&mut self) {
        let Some(playing) = &mut self.playing else {
            return;
        };
        let Some(rx) = &playing.lyrics_lookup else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Disconnected) => Err("Lyrics search stopped.".into()),
            Err(mpsc::TryRecvError::Empty) => return,
        };
        playing.lyrics_lookup = None;
        playing.lyrics_status = match result {
            Ok(Some(text)) => {
                playing.timed_lyrics = library::timed_lyrics(&text);
                playing.media.lyrics = text;
                playing.lyrics_online = true;
                "Source: LRCLIB".into()
            }
            Ok(None) => "No matching lyrics found.".into(),
            Err(error) => error,
        };
    }

    fn save_lyrics(&mut self) {
        let Some(playing) = &mut self.playing else {
            return;
        };
        let Some(track) = self
            .library
            .track_path(playing.id)
            .map(|path| path.to_path_buf())
        else {
            return;
        };
        let extension = if playing.timed_lyrics.is_empty() {
            "txt"
        } else {
            "lrc"
        };
        playing.lyrics_status =
            match lyrics::save(&track.with_extension(extension), &playing.media.lyrics) {
                Ok(()) => {
                    playing.lyrics_online = false;
                    format!("Saved .{extension} next to the track.")
                }
                Err(error) => format!("Could not save lyrics: {error}"),
            };
    }

    fn lookup_artwork(&mut self, ctx: &egui::Context) {
        let Some(playing) = &mut self.playing else {
            return;
        };
        if !self.library.online_artwork {
            playing.lookup = None;
            playing.online_url = None;
            playing.online_cover = None;
            playing.artwork_status = "";
            return;
        }
        if playing.lookup.is_some()
            || playing.online_url.is_some()
            || playing.media.artist.trim().is_empty()
        {
            return;
        }
        let title = playing.media.title.clone();
        let artist = playing.media.artist.clone();
        let album = playing.media.album.clone();
        let duration = playing.duration;
        let download_image = playing.cover.is_none();
        let ctx = ctx.clone();
        let (tx, rx) = mpsc::channel();
        playing.lookup = Some(rx);
        playing.artwork_status = "Looking up artwork…";
        std::thread::spawn(move || {
            let _ = tx.send(artwork::lookup(
                &title,
                &artist,
                &album,
                duration,
                download_image,
            ));
            ctx.request_repaint();
        });
    }

    fn finish_artwork(&mut self, ctx: &egui::Context) {
        let Some(playing) = &mut self.playing else {
            return;
        };
        let Some(lookup) = &playing.lookup else {
            return;
        };
        match lookup.try_recv() {
            Ok(result) => {
                playing.lookup = None;
                if let Ok(Some(result)) = result {
                    playing.online_url = Some(result.url);
                    playing.artwork_status = "Found Apple Music artwork.";
                    playing.online_cover = result
                        .image
                        .as_deref()
                        .and_then(cover_image)
                        .map(|image| background::Cover::new(ctx, image));
                    self.sync_presence();
                } else {
                    playing.artwork_status = result
                        .err()
                        .unwrap_or("No matching Apple Music artwork found.");
                }
            }
            Err(mpsc::TryRecvError::Disconnected) => playing.lookup = None,
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }

    /// Apply the background tag read to the current track. The generation
    /// makes rapid track changes discard stale results; the cover texture and
    /// network lookups stay on the UI thread.
    /// Open the properties dialog immediately and read tags on a worker.
    fn open_track_properties(&mut self, path: PathBuf, ctx: &egui::Context) {
        let (send, receive) = mpsc::channel();
        self.property_jobs.insert(path.clone(), receive);
        self.dialogs.track_properties = Some(TrackProperties::loading(path.clone()));
        let paint = ctx.clone();
        std::thread::spawn(move || {
            let properties = TrackProperties::read(path);
            let _ = send.send(properties);
            paint.request_repaint();
        });
    }

    /// Fill a still-loading properties dialog with the worker's result.
    fn finish_property_jobs(&mut self) {
        let mut ready = Vec::new();
        self.property_jobs
            .retain(|path, receiver| match receiver.try_recv() {
                Ok(properties) => {
                    ready.push((path.clone(), properties));
                    false
                }
                Err(mpsc::TryRecvError::Disconnected) => false,
                Err(mpsc::TryRecvError::Empty) => true,
            });
        for (path, properties) in ready {
            if self
                .dialogs
                .track_properties
                .as_ref()
                .is_some_and(|open| open.path == path && open.loading)
            {
                self.dialogs.track_properties = Some(properties);
            }
        }
    }

    fn finish_media_job(&mut self, ctx: &egui::Context) {
        let ready = match &self.media_job {
            Some((generation, receiver)) => match receiver.try_recv() {
                Ok(media) => Some((*generation, media)),
                Err(mpsc::TryRecvError::Disconnected) | Err(mpsc::TryRecvError::Empty) => return,
            },
            None => return,
        };
        self.media_job = None;
        let Some((generation, mut media)) = ready else {
            return;
        };
        if generation != self.media_generation {
            return;
        }
        let cover = media
            .cover
            .as_deref()
            .and_then(cover_image)
            .map(|image| background::Cover::new(ctx, image));
        if cover.is_none()
            || media
                .cover
                .as_ref()
                .is_some_and(|data| data.len() > media_keys::MAX_COVER_BYTES)
        {
            media.cover = None;
        }
        let timed_lyrics = library::timed_lyrics(&media.lyrics);
        let Some(playing) = &mut self.playing else {
            return;
        };
        playing.media = media;
        playing.cover = cover;
        playing.timed_lyrics = timed_lyrics;
        if let Some(playing) = &self.playing {
            self.plugins.track_changed(&playing.media);
        }
        if let Ok(media_keys) = &mut self.media_keys {
            media_keys.track_changed();
        }
        self.lookup_artwork(ctx);
        if self.library.online_lyrics
            && self
                .playing
                .as_ref()
                .is_some_and(|p| p.media.lyrics.trim().is_empty())
        {
            self.lookup_lyrics(ctx);
        }
    }

    fn start(&mut self, id: TrackId, ctx: &egui::Context) {
        match self.open(id, ctx, None) {
            Ok(()) => {
                self.plan(id);
                self.refresh_gapless();
                self.status.clear();
                self.save_playback(Duration::ZERO);
                self.sync_presence();
            }
            Err(error) => self.status = format!("Could not play track: {error}"),
        }
    }

    /// Start a whole playlist: a random entry when shuffle is on, otherwise
    /// the first entry in the current sort order.
    fn start_playlist(&mut self, list: usize, ctx: &egui::Context) {
        let len = self.library.playlists[list].tracks.len();
        let order = self.views.get(&list).map(|view| view.order.as_slice());
        if let Some(index) = playlist_entry(len, self.library.shuffle, order)
            && let Some(id) = self.library.track_id((list, index))
        {
            self.start(id, ctx);
        }
    }

    fn step(&mut self, offset: isize, ctx: &egui::Context) {
        let failed = if offset > 0 {
            match self.play_next_queued(ctx) {
                Ok(()) => return,
                Err(warning) => warning,
            }
        } else {
            None
        };
        self.step_playlist(offset, ctx, failed);
    }

    fn step_playlist(&mut self, offset: isize, ctx: &egui::Context, mut failed: Option<String>) {
        let Some(_) = self.order_list else {
            if let Some(warning) = failed {
                self.status = warning;
            }
            if self.playing.as_ref().is_some_and(|p| p.sink.empty()) {
                self.stop();
            }
            return;
        };
        let mut candidate = self.adjacent_cursor(offset);
        while let Some(cursor) = candidate
            && let Some(&id) = self.order.get(cursor)
            && self.library.locate(id).is_some()
        {
            match self.open(id, ctx, None) {
                Ok(()) => {
                    self.cursor = cursor;
                    self.refresh_gapless();
                    self.status = failed.unwrap_or_default();
                    self.save_playback(Duration::ZERO);
                    self.sync_presence();
                    return;
                }
                Err(error) => {
                    self.status = format!("Skipped unreadable track: {error}");
                    failed = Some(self.status.clone());
                }
            }
            candidate = cursor.checked_add_signed(offset);
        }
        if self.playing.as_ref().is_some_and(|p| p.sink.empty()) {
            if let Some(warning) = failed {
                self.status = warning;
            }
            self.stop();
        } else if let Some(warning) = failed {
            self.status = warning;
        }
    }

    fn previous_restarts_track(&self) -> bool {
        self.library.restart_track_on_previous
            && self
                .playing
                .as_ref()
                .is_some_and(|playing| playing.sink.get_pos() > Duration::from_secs(3))
    }

    fn previous(&mut self, ctx: &egui::Context) {
        if self.previous_restarts_track() && self.seek(Duration::ZERO, ctx) {
            return;
        }
        if let Some(anchor) = self.order_id()
            && self.playing.as_ref().is_some_and(|p| p.id != anchor)
        {
            match self.open(anchor, ctx, None) {
                Ok(()) => {
                    self.status.clear();
                    self.save_playback(Duration::ZERO);
                    self.sync_presence();
                    self.refresh_gapless();
                }
                Err(error) => self.status = format!("Could not play previous track: {error}"),
            }
        } else {
            self.step(-1, ctx);
        }
    }

    fn seek(&mut self, target: Duration, ctx: &egui::Context) -> bool {
        let Some(playing) = &self.playing else {
            return false;
        };
        match playing.sink.try_seek(target) {
            Ok(()) => {
                self.status.clear();
                self.sync_presence();
                ctx.request_repaint();
                true
            }
            Err(error) => {
                self.status = format!("Could not seek: {error}");
                false
            }
        }
    }

    fn resume(&mut self, ctx: &egui::Context) {
        if self.playing.as_ref().is_some_and(|p| p.sink.failed) {
            self.reopen_audio(ctx, false);
            return;
        }
        if let Some(playing) = &self.playing {
            playing.sink.play();
            self.record_play();
            self.sync_presence();
        } else {
            let warning = match self.play_next_queued(ctx) {
                Ok(()) => return,
                Err(warning) => warning,
            };
            self.start_playlist(self.library.selected, ctx);
            if let Some(warning) = warning {
                self.status = warning;
            }
        }
    }

    fn stop(&mut self) {
        self.observe_lastfm();
        self.lastfm.track_changed();
        self.save_playback(Duration::ZERO);
        self.playing = None;
        self.clear_order();
        self.sync_presence();
    }

    fn pause(&self) {
        if let Some(playing) = &self.playing {
            playing.sink.pause();
            self.sync_presence();
        }
    }

    fn toggle_shuffle(&mut self) {
        self.library.shuffle = !self.library.shuffle;
        if let Some(id) = self.order_id() {
            self.plan(id);
        }
        self.save();
    }

    fn taskbar_buttons(&self) -> taskbar::Buttons {
        let loaded = self.playing.is_some();
        taskbar::Buttons {
            previous: (loaded
                && (self.adjacent_cursor(-1).is_some()
                    || self.order_id().is_some_and(|anchor| {
                        self.playing.as_ref().is_some_and(|p| p.id != anchor)
                    })))
                || self.previous_restarts_track(),
            can_play: loaded
                || !self.play_queue.is_empty()
                || !self.library.playlists[self.library.selected]
                    .tracks
                    .is_empty(),
            playing: self.playing.as_ref().is_some_and(|p| !p.sink.is_paused()),
            next: !self.play_queue.is_empty() || (loaded && self.adjacent_cursor(1).is_some()),
            shuffle: self.library.shuffle,
        }
    }

    fn sync_presence(&self) {
        let app_id = self.library.discord_id.trim();
        let enabled = self.library.discord_enabled
            && !app_id.is_empty()
            && app_id.bytes().all(|byte| byte.is_ascii_digit());
        let activity = self
            .playing
            .as_ref()
            .filter(|p| !p.sink.is_paused())
            .and_then(|playing| {
                let path = self.library.track_path(playing.id)?;
                let artwork = self
                    .library
                    .artwork_urls
                    .get(path)
                    .map(String::as_str)
                    .filter(|url| presence::public_artwork_url(url))
                    .or(playing.online_url.as_deref())
                    .unwrap_or_default();
                Some(presence::Activity {
                    title: playing.media.title.clone(),
                    artist: playing.media.artist.clone(),
                    album: playing.media.album.clone(),
                    artwork: artwork.into(),
                    fallback_artwork: self.library.fallback_asset.trim().into(),
                    elapsed: playing.sink.get_pos(),
                    duration: playing.duration,
                })
            });
        self.rpc
            .update(enabled.then(|| app_id.to_owned()), activity);
    }
}

impl eframe::App for Player {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        let frame_started = self.frame_log.as_ref().map(|_| Instant::now());
        self.setup.update(ctx);
        self.lastfm.poll(ctx);
        self.updater.poll(ctx);
        if let Some(notice) = &self.notice {
            let elapsed = notice.started.elapsed();
            if elapsed >= Duration::from_secs(3) {
                self.notice = None;
            } else if elapsed < Duration::from_millis(2550) {
                ctx.request_repaint_after(Duration::from_millis(2550) - elapsed);
            } else {
                ctx.request_repaint_after(Duration::from_millis(16));
            }
        }
        if std::mem::take(&mut self.restore_track) && self.library.restore_last_track {
            self.restore_playback(ctx);
        }
        self.update_audio(ctx);
        self.run_transport_commands(ctx);
        let mut dropped: Vec<PathBuf> = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect()
        });
        if self.dialogs.settings_page == Some(settings::Page::Appearance)
            && let Some(draft) = &mut self.theme_draft
        {
            dropped.retain(|path| {
                if !background::supported(path) {
                    return true;
                }
                draft.background.select_file(path.clone());
                false
            });
        }
        self.import(dropped, ctx);
        self.finish_imports();
        self.finish_playlist_io();
        self.finish_property_jobs();
        self.finish_media_job(ctx);
        self.finish_artwork(ctx);
        self.finish_lyrics();
        self.finish_relink();
        self.finish_indexes();
        self.finish_gain_jobs(ctx);
        self.finish_saves();
        self.request_index(self.library.selected, ctx);
        if let Some(playing) = &self.playing
            && let Some((list, _)) = self.library.locate(playing.id)
        {
            self.request_index(list, ctx);
        }
        if self
            .playing
            .as_ref()
            .is_some_and(|playing| playing.sink.empty())
        {
            self.advance_automatic(ctx);
        }
        if self.playing.as_ref().is_some_and(|p| !p.sink.is_paused())
            || self.dialogs.discord_settings
        {
            ctx.request_repaint_after(Duration::from_millis(200));
        }

        self.header(ctx);

        egui::TopBottomPanel::bottom("transport")
            .frame(desk_surface(&ctx.style().visuals))
            .show(ctx, |ui| {
                self.transport(ui, ctx);
                if !self.status.is_empty() {
                    ui.weak(&self.status);
                }
                if let Some(notice) = &self.notice {
                    let fade = ((Duration::from_secs(3).saturating_sub(notice.started.elapsed()))
                        .as_secs_f32()
                        / 0.45)
                        .clamp(0.0, 1.0);
                    ui.colored_label(
                        ui.visuals().weak_text_color().gamma_multiply(fade),
                        &notice.message,
                    );
                }
            });

        let requested_sort = self.sidebar(ctx);

        self.inspector(ctx);

        self.playlist_panel(ctx, requested_sort);

        self.dialogs(ctx);
        self.refresh_gapless();
        self.flush_volume(ctx);
        self.refresh_windows_controls();
        if let (Some(log), Some(started)) = (&mut self.frame_log, frame_started) {
            let _ = writeln!(
                log,
                "{},{}",
                self.frame_log_count,
                started.elapsed().as_micros()
            );
            self.frame_log_count += 1;
            if self.frame_log_count.is_multiple_of(120) {
                let _ = log.flush();
            }
        }
    }

    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        self.observe_lastfm();
        if self.volume_changed {
            self.save();
        }
        if let Some(playing) = &self.playing {
            self.save_playback(playing.sink.get_pos());
        }
        // Drain and join: graceful exit must never lose a queued snapshot.
        self.save_worker.shutdown();
        if let Some(log) = &mut self.frame_log {
            let _ = log.flush();
        }
    }
}

fn desk_sections(ui: &mut egui::Ui, height: f32, center_width: f32) -> [egui::Ui; 3] {
    let (row, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );
    let edge = (row.width() - center_width).max(0.0) * 0.5;
    let stops = [
        row.left(),
        row.left() + edge,
        row.right() - edge,
        row.right(),
    ];
    std::array::from_fn(|index| {
        let rect = egui::Rect::from_min_max(
            egui::pos2(stops[index], row.top()),
            egui::pos2(stops[index + 1], row.bottom()),
        )
        .shrink2(egui::vec2(3.0, 0.0));
        let mut child = ui.new_child(egui::UiBuilder::new().id_salt(index).max_rect(rect));
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        child
    })
}

fn desk_surface(visuals: &egui::Visuals) -> egui::Frame {
    egui::Frame::new()
        .fill(visuals.window_fill)
        .stroke(visuals.window_stroke)
        .inner_margin(egui::Margin::symmetric(11, 7))
}

fn sort_direction_button(ui: &mut egui::Ui, enabled: bool, descending: bool) -> bool {
    let response = ui
        .add_enabled(
            enabled,
            egui::Button::new("")
                .frame(false)
                .min_size(egui::vec2(22.0, ui.spacing().interact_size.y)),
        )
        .on_hover_text("Reverse sort order");
    let center = response.rect.center();
    let direction = if descending { -1.0 } else { 1.0 };
    let tip = center + egui::vec2(0.0, 4.0 * direction);
    let tint = if enabled {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    let stroke = egui::Stroke::new(1.0_f32, tint);
    ui.painter()
        .line_segment([center - egui::vec2(0.0, 4.0 * direction), tip], stroke);
    for side in [-1.0, 1.0] {
        ui.painter().line_segment(
            [tip, tip + egui::vec2(side * 3.0, -3.0 * direction)],
            stroke,
        );
    }
    response.clicked()
}

/// Which entry of a playlist to start: random when shuffled, otherwise the
/// first entry of the visible sort order (or the first track if no view yet).
fn playlist_entry(len: usize, shuffle: bool, order: Option<&[usize]>) -> Option<usize> {
    if len == 0 {
        return None;
    }
    if shuffle {
        Some(rand::random_range(0..len))
    } else {
        Some(
            order
                .and_then(|entries| entries.first().copied())
                .unwrap_or(0),
        )
    }
}

#[cfg(test)]
mod playlist_entry_tests {
    use super::playlist_entry;

    #[test]
    fn empty_playlist_has_no_entry() {
        assert_eq!(playlist_entry(0, false, None), None);
        assert_eq!(playlist_entry(0, true, None), None);
    }

    #[test]
    fn without_shuffle_uses_first_visible_entry() {
        assert_eq!(playlist_entry(5, false, Some(&[3, 1, 2])), Some(3));
        assert_eq!(playlist_entry(5, false, None), Some(0));
    }

    #[test]
    fn with_shuffle_stays_within_bounds() {
        for _ in 0..32 {
            assert!(playlist_entry(5, true, Some(&[3, 1, 2])).unwrap() < 5);
        }
    }
}

/// The favorites heart lives between the track number and the title.
fn favorite_column_rect(row: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(
        egui::pos2(row.left() + 70.0, row.top()),
        egui::pos2(row.left() + 92.0, row.bottom()),
    )
}

fn playlist_columns(
    ui: &mut egui::Ui,
    height: f32,
    text: [&str; 4],
    header: bool,
    active: bool,
    tooltip: &str,
    artwork: Option<Option<egui::TextureId>>,
) -> egui::Response {
    use egui::{Align2, FontId, Rect, Sense, pos2, vec2};
    let sense = if header {
        Sense::hover()
    } else {
        Sense::click()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), sense);
    let painter = ui.painter();
    if active || response.hovered() && !header {
        painter.rect_filled(rect, 0.0, ui.visuals().selection.bg_fill);
    }
    painter.hline(rect.x_range(), rect.bottom(), ui.visuals().window_stroke);
    let left = rect.left();
    let boundaries = [
        left,
        left + 22.0,
        left + 70.0,
        left + 92.0,
        (rect.right() - 46.0).max(left + 92.0),
        rect.right(),
    ];
    for (column, content) in text.into_iter().enumerate() {
        // Skip the heart gap reserved between number and title.
        let start = if column >= 2 { column + 1 } else { column };
        if boundaries[start + 1] <= boundaries[start] {
            continue;
        }
        let clip = Rect::from_min_max(
            pos2(boundaries[start], rect.top()),
            pos2(boundaries[start + 1], rect.bottom()),
        )
        .intersect(ui.clip_rect());
        let (x, align) = if column == 1 || column == 3 {
            (boundaries[start + 1] - 7.0, Align2::RIGHT_CENTER)
        } else {
            (boundaries[start] + 7.0, Align2::LEFT_CENTER)
        };
        let x = if column == 2
            && let Some(texture) = artwork
        {
            let size = height - 4.0;
            let cover = Rect::from_min_size(pos2(x, rect.top() + 2.0), vec2(size, size));
            if !header {
                let p = painter.with_clip_rect(clip);
                if let Some(texture) = texture {
                    p.image(
                        texture,
                        cover,
                        Rect::from_min_max(pos2(0., 0.), pos2(1., 1.)),
                        egui::Color32::WHITE,
                    );
                } else {
                    p.rect_filled(cover, 2, ui.visuals().widgets.inactive.bg_fill);
                }
            }
            x + size + 6.0
        } else {
            x
        };
        let color = if header {
            ui.visuals().weak_text_color()
        } else if active {
            ui.visuals().selection.stroke.color
        } else {
            ui.visuals().text_color()
        };
        painter.with_clip_rect(clip).text(
            pos2(x, rect.center().y),
            align,
            content,
            FontId::proportional(13.0),
            color,
        );
    }
    if tooltip.is_empty() {
        response
    } else {
        response.on_hover_text(tooltip)
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

fn cover_image(bytes: &[u8]) -> Option<image::RgbaImage> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    Some(reader.decode().ok()?.thumbnail(600, 600).into_rgba8())
}

#[derive(Clone, Copy)]
enum Icon {
    Stop,
    Play,
    Pause,
    Previous,
    Next,
    Shuffle,
}

fn icon(ui: &mut egui::Ui, icon: Icon, label: &str, enabled: bool, selected: bool) -> bool {
    use egui::{Rect, Shape, Stroke, vec2};
    let response = ui
        .add_enabled(
            enabled,
            egui::Button::new("")
                .frame(false)
                .selected(selected)
                .min_size(vec2(25.0, 28.0)),
        )
        .on_hover_text(label);
    let main = matches!(icon, Icon::Play | Icon::Pause);
    let accent = ui.visuals().selection.stroke.color;
    if main && enabled {
        ui.painter()
            .circle_filled(response.rect.center(), 12.5, accent);
    } else if response.hovered() || selected {
        ui.painter()
            .rect_filled(response.rect, 3.0, ui.visuals().widgets.hovered.bg_fill);
    }
    let center = response.rect.center();
    let color = if main && enabled {
        if accent.intensity() > 0.57 {
            egui::Color32::BLACK
        } else {
            egui::Color32::WHITE
        }
    } else if enabled {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color().gamma_multiply(0.55)
    };
    let stroke = Stroke::new(1.5_f32, color);
    let rect = |x, y, w, h| {
        ui.painter().rect_filled(
            Rect::from_center_size(center + vec2(x, y), vec2(w, h)),
            0.5,
            color,
        )
    };
    let triangle = |x: f32, sign: f32| {
        ui.painter().add(Shape::convex_polygon(
            vec![
                center + vec2(x - sign * 4.0, -5.0),
                center + vec2(x + sign * 4.5, 0.0),
                center + vec2(x - sign * 4.0, 5.0),
            ],
            color,
            Stroke::NONE,
        ))
    };
    let line = |a, b| ui.painter().line_segment([center + a, center + b], stroke);
    match icon {
        Icon::Stop => {
            rect(0.0, 0.0, 10.0, 10.0);
        }
        Icon::Play => {
            triangle(0.0, 1.0);
        }
        Icon::Pause => {
            rect(-2.5, 0.0, 2.0, 10.0);
            rect(2.5, 0.0, 2.0, 10.0);
        }
        Icon::Previous => {
            rect(-5.0, 0.0, 1.6, 10.0);
            triangle(1.0, -1.0);
        }
        Icon::Next => {
            triangle(-1.0, 1.0);
            rect(5.0, 0.0, 1.6, 10.0);
        }
        Icon::Shuffle => {
            for sign in [-1.0, 1.0] {
                let y = 4.0 * sign;
                line(vec2(-6.0, -y), vec2(-3.0, -y));
                line(vec2(-3.0, -y), vec2(3.0, y));
                line(vec2(3.0, y), vec2(7.0, y));
                line(vec2(7.0, y), vec2(4.5, y - 2.0));
                line(vec2(7.0, y), vec2(4.5, y + 2.0));
            }
        }
    }
    response.clicked()
}

fn volume_slider(ui: &mut egui::Ui, volume: &mut f32) -> egui::Response {
    let mut response = ui.add_sized(
        egui::vec2(190.0, ui.spacing().interact_size.y),
        egui::Slider::new(volume, 0.0..=1.0)
            .text("Volume")
            .custom_formatter(|value, decimals| {
                let mut label = egui::emath::format_with_decimals_in_range(value, decimals);
                // Wheel steps can leave a value just below 1 that still rounds to 1.000.
                if let Some((whole @ ("0" | "1"), fractional)) = label.split_once('.')
                    && fractional.bytes().all(|digit| digit == b'0')
                {
                    label.truncate(whole.len());
                }
                label
            }),
    );
    volume_wheel(ui, volume, &mut response);
    response
}

struct VolumeResponse {
    response: egui::Response,
    speaker_clicked: bool,
}

fn desk_volume(ui: &mut egui::Ui, volume: &mut f32, is_muted: bool) -> VolumeResponse {
    use egui::{Align2, Sense, Stroke, pos2, vec2};

    let (label, label_response) = ui.allocate_exact_size(vec2(35.0, 24.0), Sense::hover());
    let slider = ui
        .scope(|ui| {
            ui.spacing_mut().slider_width = (ui.available_width() - 25.0).clamp(52.0, 85.0);
            ui.spacing_mut().interact_size.y = 20.0;
            // Keep egui's mouse and keyboard handling; paint the compact track separately.
            ui.set_opacity(0.0);
            ui.add(
                egui::Slider::new(volume, 0.0..=1.0)
                    .show_value(false)
                    .step_by(0.01)
                    .handle_shape(egui::style::HandleShape::Circle),
            )
        })
        .inner;
    let (speaker, speaker_response) = ui.allocate_exact_size(vec2(16.0, 20.0), Sense::click());
    let mut response = slider
        .clone()
        .union(label_response)
        .union(speaker_response.clone())
        .on_hover_text(if is_muted {
            "Unmute"
        } else {
            "Mute · scroll to adjust"
        });
    volume_wheel(ui, volume, &mut response);

    let mut accent = ui.visuals().selection.stroke.color;
    let muted = ui.visuals().weak_text_color();
    if !response.enabled() {
        accent = muted;
    }
    let painter = ui.painter();
    painter.text(
        label.right_center(),
        Align2::RIGHT_CENTER,
        format!("{:.0}%", *volume * 100.0),
        egui::TextStyle::Small.resolve(ui.style()),
        ui.visuals().text_color(),
    );
    let track = slider.rect.x_range().shrink(slider.rect.height() / 2.5);
    let y = slider.rect.center().y;
    let knob = pos2(egui::lerp(track, *volume), y);
    painter.hline(
        track,
        y,
        Stroke::new(3.0_f32, ui.visuals().window_stroke.color),
    );
    if *volume > 0.0 && !is_muted {
        painter.hline(track.min..=knob.x, y, Stroke::new(3.0_f32, accent));
    }
    painter.circle_filled(
        knob,
        if slider.hovered() || slider.dragged() {
            4.5
        } else {
            3.5
        },
        accent,
    );
    if slider.has_focus() {
        painter.circle_stroke(knob, 6.0, Stroke::new(1.0_f32, accent));
    }

    let center = speaker.center();
    let outline = [
        (-6.0, -2.0),
        (-3.0, -2.0),
        (0.0, -5.0),
        (0.0, 5.0),
        (-3.0, 2.0),
        (-6.0, 2.0),
        (-6.0, -2.0),
    ];
    painter.add(egui::Shape::line(
        outline
            .into_iter()
            .map(|(x, y)| center + vec2(x, y))
            .collect(),
        Stroke::new(1.0_f32, muted),
    ));
    if *volume > 0.0 && !is_muted {
        painter.add(egui::Shape::line(
            vec![
                center + vec2(3.0, -3.0),
                center + vec2(5.0, 0.0),
                center + vec2(3.0, 3.0),
            ],
            Stroke::new(1.0_f32, muted),
        ));
    }
    if is_muted {
        painter.line_segment(
            [center + vec2(-7.0, -7.0), center + vec2(7.0, 7.0)],
            Stroke::new(1.6_f32, ui.visuals().text_color()),
        );
    }
    VolumeResponse {
        response,
        speaker_clicked: speaker_response.clicked(),
    }
}

fn volume_wheel(ui: &egui::Ui, volume: &mut f32, response: &mut egui::Response) {
    if response.enabled() && response.hovered() {
        let steps: f32 = ui.input(|input| {
            input
                .events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::MouseWheel { unit, delta, .. } => Some(match unit {
                        egui::MouseWheelUnit::Point => delta.y / 24.0,
                        egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => delta.y,
                    }),
                    _ => None,
                })
                .sum()
        });
        let adjusted = (*volume + steps * 0.02).clamp(0.0, 1.0);
        if adjusted != *volume {
            *volume = adjusted;
            response.mark_changed();
            ui.ctx().request_repaint();
        }
    }
}

fn seek_bar(ui: &mut egui::Ui, elapsed: Duration, duration: Duration) -> Option<Duration> {
    use egui::{Align2, CursorIcon, FontId, PointerButton, Sense, vec2};

    let height = 14.0;
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width().max(1.0), height),
        Sense::click_and_drag(),
    );
    let rail = rect.shrink2(vec2(38.0, (height - 3.0) * 0.5));
    let position = response.interact_pointer_pos().map(|pointer| {
        let fraction = ((pointer.x - rail.left()) / rail.width().max(1.0)).clamp(0.0, 1.0);
        duration.mul_f64(f64::from(fraction))
    });
    let button = PointerButton::Primary;
    let commit = response.clicked_by(button) || response.drag_stopped_by(button);
    let preview = if commit || response.dragged_by(button) {
        position.unwrap_or(elapsed)
    } else {
        elapsed
    };
    let fraction = (preview.as_secs_f64() / duration.as_secs_f64()).clamp(0.0, 1.0) as f32;
    let painter = ui.painter();
    painter.rect_filled(rail, 0.0, ui.visuals().window_stroke.color);
    if fraction > 0.0 {
        let mut fill = rail;
        fill.max.x = rail.left() + rail.width() * fraction;
        painter.rect_filled(fill, 0.0, ui.visuals().selection.stroke.color);
    }
    painter.text(
        rect.left_center(),
        Align2::LEFT_CENTER,
        time(preview),
        FontId::proportional(10.5),
        ui.visuals().weak_text_color(),
    );
    painter.text(
        rect.right_center(),
        Align2::RIGHT_CENTER,
        time(duration),
        FontId::proportional(10.5),
        ui.visuals().weak_text_color(),
    );
    response
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text("Click or drag to seek");
    if commit { position } else { None }
}

fn time(duration: Duration) -> String {
    let seconds = duration.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn apply_ui_chrome(ctx: &egui::Context, theme: theme::Theme) {
    ctx.set_visuals(theme.visuals());
    ctx.style_mut(|style| {
        let (item_spacing, button_padding, interact_height) = match theme.density {
            theme::Density::Compact => (6.0, egui::vec2(8.0, 3.0), 27.0),
            theme::Density::Comfortable => (7.0, egui::vec2(9.0, 4.0), 28.0),
            theme::Density::Relaxed => (11.0, egui::vec2(12.0, 7.0), 36.0),
        };
        let scale = f32::from(theme.font_scale_percent.clamp(85, 135)) / 100.0;
        style.spacing.item_spacing = egui::vec2(item_spacing, item_spacing);
        style.spacing.button_padding = button_padding;
        style.spacing.interact_size = egui::vec2(34.0, interact_height);
        style.spacing.slider_rail_height = 5.0;
        style.spacing.window_margin = egui::Margin::symmetric(10, 10);
        style.spacing.menu_margin = egui::Margin::symmetric(7, 7);
        style.spacing.scroll.bar_width = 10.0;
        style.spacing.scroll.handle_min_length = 28.0;
        style.text_styles.insert(
            egui::TextStyle::Small,
            egui::FontId::proportional(11.0 * scale),
        );
        style.text_styles.insert(
            egui::TextStyle::Body,
            egui::FontId::proportional(13.5 * scale),
        );
        style.text_styles.insert(
            egui::TextStyle::Button,
            egui::FontId::proportional(13.0 * scale),
        );
        style.text_styles.insert(
            egui::TextStyle::Heading,
            egui::FontId::proportional(18.0 * scale),
        );
    });
}

fn main() -> eframe::Result {
    diagnostics::install();
    let viewport = egui::ViewportBuilder::default()
        .with_decorations(false)
        .with_inner_size([1120.0, 680.0])
        .with_min_inner_size([680.0, 420.0]);
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "Rust Player",
        options,
        Box::new(|creation| {
            let mut player = Player::new(&creation.egui_ctx);
            player.audio_devices = Some(audio::DeviceMonitor::new(&creation.egui_ctx));
            player.taskbar = taskbar::Taskbar::new(creation, &creation.egui_ctx);
            player.window_chrome = window_chrome::NativeChrome::new(creation);
            player.media_keys = media_keys::MediaKeys::new(creation, &creation.egui_ctx);
            apply_ui_chrome(&creation.egui_ctx, player.library.theme);
            match fonts::load(player.library.font.as_deref()) {
                Ok(fonts) => creation.egui_ctx.set_fonts(fonts),
                Err(error) => {
                    creation.egui_ctx.set_fonts(fonts::defaults());
                    player.status = format!("Could not load saved font: {error}");
                }
            }
            Ok(Box::new(player))
        }),
    )
}
