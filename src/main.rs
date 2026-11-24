#![windows_subsystem = "windows"]

mod artwork;
mod audio;
mod audio_setup;
mod audio_ui;
mod background;
mod cue;
mod diagnostics;
mod fonts;
mod gapless_ui;
mod lastfm;
mod library;
mod listening;
mod lyrics;
mod media_keys;
mod playlist_io;
mod plugins;
mod presence;
mod settings;
mod taskbar;
mod theme;

use eframe::egui::{self, RichText};
use library::{Library, Media, Playlist, PlaylistRow, PlaylistView, Sort, TrackInfo, file_title};
use rand::seq::SliceRandom;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};

type Track = (usize, usize);
type ImportResult = (Vec<PathBuf>, usize);

struct Playing {
    counted: bool,
    lastfm: lastfm::Listen,
    track: Track,
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

#[derive(serde::Serialize, serde::Deserialize)]
struct SavedTrack {
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
}

struct CoverViewer {
    cover: background::Cover,
    title: String,
    artist: String,
}

impl TrackProperties {
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
        }
    }
}

impl SavedTrack {
    fn resolve(&self, library: &Library) -> Option<Track> {
        let (list, index) = self.track;
        if let Some(playlist) = library.playlists.get(list) {
            if playlist.tracks.get(index) == Some(&self.path) {
                return Some(self.track);
            }
            if let Some(index) = playlist.tracks.iter().position(|path| path == &self.path) {
                return Some((list, index));
            }
        }
        library
            .playlists
            .iter()
            .enumerate()
            .find_map(|(list, playlist)| {
                playlist
                    .tracks
                    .iter()
                    .position(|path| path == &self.path)
                    .map(|index| (list, index))
            })
    }
}

struct Player {
    background: background::Preview,
    setup: audio_setup::Setup,
    audio_devices: Option<audio::DeviceMonitor>,
    audio_retry_after: Option<Instant>,
    gapless_generation: u64,
    audio_release: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    playing: Option<Playing>,
    library: Library,
    library_path: PathBuf,
    library_needs_backup: bool,
    restore_track: bool,
    volume_changed: bool,
    volume_save_at: Option<Instant>,
    status: String,
    playlist_dialog: Option<PlaylistDialog>,
    track_properties: Option<TrackProperties>,
    cover_viewer: Option<CoverViewer>,
    listening: listening::Listening,
    listening_ranks: Option<listening::Rankings>,
    activity_open: bool,
    theme_draft: Option<ThemeDraft>,
    plugins: plugins::Host,
    plugins_open: bool,
    taskbar: Option<taskbar::Taskbar>,
    media_keys: Result<media_keys::MediaKeys, String>,
    settings_page: Option<settings::Page>,
    activity_tab: ActivityTab,
    discord_settings: bool,
    artwork_url: String,
    track_search: String,
    rpc: presence::Presence,
    lastfm: lastfm::Client,
    order: Vec<usize>,
    cursor: usize,
    imports: Vec<(usize, mpsc::Receiver<ImportResult>)>,
    views: HashMap<usize, PlaylistView>,
    indexing: HashMap<usize, mpsc::Receiver<Vec<TrackInfo>>>,
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
        let listening =
            match listening::Listening::load(&library_path.with_file_name("listening.json")) {
                Ok(data) => data,
                Err(error) => {
                    status = format!("Could not load listening activity: {error}");
                    listening::Listening::default()
                }
            };
        let plugins = plugins::Host::load(&library.plugins);
        let lastfm = lastfm::Client::new(ctx, &library_path, &library);
        let setup = audio_setup::Setup::new(!library_path.exists() && !library.audio_setup_done);
        Self {
            background: background::Preview::default(),
            setup,
            audio_devices: None,
            audio_retry_after: None,
            gapless_generation: 0,
            audio_release: None,
            playing: None,
            library,
            library_path,
            library_needs_backup,
            restore_track: true,
            volume_changed: false,
            volume_save_at: None,
            status,
            playlist_dialog: None,
            track_properties: None,
            cover_viewer: None,
            listening,
            listening_ranks: None,
            activity_open: false,
            theme_draft: None,
            plugins,
            plugins_open: false,
            taskbar: None,
            media_keys: Err("Windows media session not initialized".into()),
            settings_page: None,
            activity_tab: ActivityTab::default(),
            discord_settings: false,
            artwork_url: String::new(),
            track_search: String::new(),
            rpc: presence::Presence::new(),
            lastfm,
            order: Vec::new(),
            cursor: 0,
            imports: Vec::new(),
            views: HashMap::new(),
            indexing: HashMap::new(),
        }
    }

    fn save(&mut self) {
        self.volume_save_at = None;
        if self.library_needs_backup {
            match Library::back_up_invalid_file(&self.library_path) {
                Ok(backup) => {
                    self.library_needs_backup = false;
                    self.status = format!(
                        "Saved a copy of unreadable settings to {}",
                        backup.display()
                    );
                }
                Err(error) => {
                    self.status = format!("Could not back up unreadable settings: {error}");
                    return;
                }
            }
        }
        match self.library.save(&self.library_path) {
            Ok(()) => self.volume_changed = false,
            Err(error) => self.status = format!("Could not save playlists or settings: {error}"),
        }
    }

    fn volume_adjusted(&mut self) {
        self.volume_changed = true;
        self.volume_save_at = Some(Instant::now() + Duration::from_millis(300));
        if let Some(playing) = &self.playing {
            playing.sink.set_volume(self.library.volume);
        }
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

    fn restore_playback(&mut self, ctx: &egui::Context) {
        let path = self.library_path.with_file_name("playback.json");
        let saved: SavedTrack = match std::fs::read(path) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(Some(saved)) => saved,
                Ok(None) => return,
                Err(error) => {
                    self.status = format!("Could not read saved playback: {error}");
                    return;
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => {
                self.status = format!("Could not read saved playback: {error}");
                return;
            }
        };
        if let Some(track) = saved.resolve(&self.library) {
            match self.open(track, ctx, Some(saved.position)) {
                Ok(()) => {
                    self.library.selected = track.0;
                    self.plan(track);
                }
                Err(error) => self.status = format!("Could not restore last track: {error}"),
            }
        }
    }

    fn save_playback(&mut self, position: Duration) {
        let Some(playing) = &self.playing else { return };
        let track = playing.track;
        let path = self.library.playlists[track.0].tracks[track.1].clone();
        let saved = SavedTrack {
            track,
            path,
            position,
        };
        self.write_playback(Some(saved));
    }

    fn write_playback(&mut self, saved: Option<SavedTrack>) {
        if let Err(error) =
            library::save_json(&saved, &self.library_path.with_file_name("playback.json"))
        {
            self.status = format!("Could not save playback: {error}");
        }
    }

    fn previously_saved(&self) -> Option<SavedTrack> {
        let bytes = std::fs::read(self.library_path.with_file_name("playback.json")).ok()?;
        let mut saved: SavedTrack = serde_json::from_slice(&bytes).ok()?;
        saved.track = saved.resolve(&self.library)?;
        Some(saved)
    }

    fn remap_playback(&mut self, saved: Option<SavedTrack>, map: impl Fn(Track) -> Option<Track>) {
        if let Some(playing) = &mut self.playing {
            let old = playing.track;
            if let Some(new) = map(old) {
                playing.track = new;
                self.order = self
                    .order
                    .iter()
                    .filter_map(|&index| map((old.0, index)).map(|track| track.1))
                    .collect();
                self.cursor = self
                    .order
                    .iter()
                    .position(|&index| index == new.1)
                    .unwrap_or(0);
                let position = playing.sink.get_pos();
                self.save_playback(position);
                return;
            }
            self.observe_lastfm();
            self.lastfm.track_changed();
            self.playing = None;
            self.order.clear();
            self.cursor = 0;
            self.sync_presence();
            self.write_playback(None);
        } else if let Some(mut saved) = saved {
            if let Some(track) = map(saved.track) {
                saved.track = track;
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
        self.save();
        self.remap_playback(saved, |(old, track)| rebase(old).map(|new| (new, track)));
    }

    fn unique_playlist_name(&self, name: &str) -> String {
        let exists = |name: &str| {
            self.library
                .playlists
                .iter()
                .any(|list| list.name.eq_ignore_ascii_case(name))
        };
        if !exists(name) {
            return name.to_owned();
        }
        (2..)
            .map(|number| format!("{name} ({number})"))
            .find(|name| !exists(name))
            .expect("available playlist name")
    }

    fn playlist_action(&mut self, index: usize, action: PlaylistAction) {
        match action {
            PlaylistAction::TogglePin => {
                let playlist = &mut self.library.playlists[index];
                playlist.pinned = !playlist.pinned;
                self.save();
            }
            PlaylistAction::Rename => {
                self.playlist_dialog = Some(PlaylistDialog::Name {
                    target: Some(index),
                    name: self.library.playlists[index].name.clone(),
                });
            }
            PlaylistAction::Copy => {
                let mut playlist = self.library.playlists[index].clone();
                playlist.pinned = false;
                playlist.sidebar_position = None;
                playlist.name = self.unique_playlist_name(&format!("{} (Copy)", playlist.name));
                self.library.playlists.push(playlist);
                self.library.selected = self.library.playlists.len() - 1;
                self.save();
            }
            PlaylistAction::Delete => self.playlist_dialog = Some(PlaylistDialog::Delete(index)),
            PlaylistAction::Export => {
                let playlist = &self.library.playlists[index];
                let name = playlist
                    .name
                    .replace(['<', '>', ':', '"', '/', '\\', '|', '?', '*'], "_");
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("M3U8 playlist", &["m3u8"])
                    .set_file_name(format!("{name}.m3u8"))
                    .save_file()
                {
                    self.status = match playlist_io::export(playlist, &path) {
                        Ok(()) => format!("Exported “{}”", playlist.name),
                        Err(error) => format!("Could not export playlist: {error}"),
                    };
                }
            }
        }
    }

    fn import_playlists(&mut self) {
        let Some(paths) = rfd::FileDialog::new()
            .add_filter("Playlist files", playlist_io::EXTENSIONS)
            .pick_files()
        else {
            return;
        };
        let mut imported = 0;
        let mut skipped = 0;
        let mut errors = Vec::new();
        for path in paths {
            match playlist_io::import(&path) {
                Ok((mut playlist, ignored)) => {
                    playlist.name = self.unique_playlist_name(&playlist.name);
                    self.library.playlists.push(playlist);
                    self.library.selected = self.library.playlists.len() - 1;
                    imported += 1;
                    skipped += ignored;
                }
                Err(error) => errors.push(format!("{}: {error}", path.display())),
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

    fn toggle_favorite(&mut self) {
        let Some(playing) = &self.playing else { return };
        let path = self.library.playlists[playing.track.0].tracks[playing.track.1].clone();
        let index = if let Some(index) = self.library.favorites {
            index
        } else {
            let index = self.library.playlists.len();
            self.library.playlists.push(Playlist {
                name: self.unique_playlist_name("Favorites"),
                tracks: Vec::new(),
                pinned: false,
                sidebar_position: None,
                sort: Sort::Album,
                descending: false,
            });
            self.library.favorites = Some(index);
            index
        };
        let playlist = &self.library.playlists[index];
        let matches: Vec<_> = playlist
            .tracks
            .iter()
            .enumerate()
            .filter_map(|(entry, track)| (track == &path).then_some(entry))
            .collect();
        if matches.is_empty() {
            self.status = format!("Added to {}", playlist.name);
            self.library.playlists[index].tracks.push(path);
            self.save();
            return;
        }

        self.status = format!("Removed from {}", playlist.name);
        if playing.track.0 == index {
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
                        .position(|track| track == &path)
                        .map(|entry| (list, entry))
                });
            if let Some(track) = replacement {
                self.playing.as_mut().expect("active track").track = track;
                self.plan(track);
            }
        }
        for entry in matches.into_iter().rev() {
            self.edit_track((index, entry), TrackEdit::Remove);
        }
    }

    fn record_play(&mut self) {
        let Some(playing) = &mut self.playing else {
            return;
        };
        if playing.counted || playing.sink.is_paused() || playing.sink.output.is_none() {
            return;
        }
        playing.counted = true;
        let track = playing.track;
        let path = self.library.playlists[track.0].tracks[track.1].clone();
        self.listening
            .record(path, &playing.media.title, &playing.media.artist);
        self.listening_ranks = None;
        if let Err(error) = library::save_json(
            &self.listening,
            &self.library_path.with_file_name("listening.json"),
        ) {
            self.status = format!("Could not save listening activity: {error}");
        }
    }

    fn edit_track(&mut self, (list, index): Track, edit: TrackEdit) {
        let saved = self.previously_saved();
        let tracks = &mut self.library.playlists[list].tracks;
        match edit {
            TrackEdit::Remove => {
                tracks.remove(index);
            }
            TrackEdit::Move(other) => tracks.swap(index, other),
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
        self.save();
        self.remap_playback(saved, |(old, track)| {
            if old != list {
                return Some((old, track));
            }
            match edit {
                TrackEdit::Remove if track == index => None,
                TrackEdit::Remove if track > index => Some((list, track - 1)),
                TrackEdit::Move(other) if track == index => Some((list, other)),
                TrackEdit::Move(other) if track == other => Some((list, index)),
                _ => Some((list, track)),
            }
        });
        if matches!(edit, TrackEdit::Move(_)) && !self.library.shuffle {
            if let Some(track) = self
                .playing
                .as_ref()
                .map(|playing| playing.track)
                .filter(|track| track.0 == list)
            {
                self.plan(track);
            }
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

    fn track_properties_dialog(&mut self, ctx: &egui::Context) {
        let Some(properties) = self.track_properties.take() else {
            return;
        };
        let mut keep_open = true;
        let mut open_folder = false;
        let mut close = false;
        egui::Window::new("Track properties")
            .open(&mut keep_open)
            .collapsible(false)
            .resizable(true)
            .show(ctx, |ui| {
                let value = |ui: &mut egui::Ui, label: &str, value: &str| {
                    ui.horizontal_wrapped(|ui| {
                        ui.strong(label);
                        ui.label(if value.trim().is_empty() {
                            "Unknown"
                        } else {
                            value
                        });
                    });
                };
                value(ui, "Title", &properties.title);
                value(ui, "Artist", &properties.artist);
                value(ui, "Album", &properties.album);
                value(ui, "Album artist", &properties.album_artist);
                value(ui, "Track", &properties.track);
                value(ui, "Duration", &properties.duration);
                value(ui, "Format", &properties.format);
                value(ui, "File size", &properties.size);
                ui.separator();
                ui.strong("Location");
                ui.label(properties.path.display().to_string());
                ui.horizontal(|ui| {
                    open_folder = ui.button("Open containing folder").clicked();
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });
            });
        if close {
            keep_open = false;
        }
        if open_folder {
            self.open_containing_folder(properties.path.clone());
        }
        if keep_open {
            self.track_properties = Some(properties);
        }
    }

    fn open_cover_viewer(
        &mut self,
        ctx: &egui::Context,
        pixels: std::sync::Arc<image::RgbaImage>,
        title: String,
        artist: String,
    ) {
        self.cover_viewer = Some(CoverViewer {
            cover: background::Cover::new(ctx, pixels.as_ref().clone()),
            title,
            artist,
        });
    }

    fn cover_viewer_dialog(&mut self, ctx: &egui::Context) {
        let Some(viewer) = self.cover_viewer.take() else {
            return;
        };
        let mut keep_open = true;
        let mut close = false;
        let mut save = false;
        egui::Window::new("Album artwork")
            .open(&mut keep_open)
            .resizable(true)
            .default_width(620.0)
            .default_height(680.0)
            .show(ctx, |ui| {
                ui.heading(&viewer.title);
                if !viewer.artist.is_empty() {
                    ui.weak(&viewer.artist);
                }
                ui.add_space(8.0);
                let size = viewer.cover.texture.size_vec2();
                let available = ui.available_size();
                let limit = available.x.min((available.y - 58.0).max(120.0)).min(700.0);
                let response = ui.add(
                    egui::Image::new((
                        viewer.cover.texture.id(),
                        size * (limit / size.x.max(size.y)),
                    ))
                    .sense(egui::Sense::click()),
                );
                response.on_hover_text("Artwork preview");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    save = ui.button("Save cover as…").clicked();
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });
            });
        if close {
            keep_open = false;
        }
        if save {
            let title: String = viewer
                .title
                .chars()
                .filter(|character| {
                    !matches!(
                        character,
                        '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                    )
                })
                .take(80)
                .collect();
            if let Some(path) = rfd::FileDialog::new()
                .set_file_name(format!(
                    "{}.png",
                    if title.is_empty() { "cover" } else { &title }
                ))
                .add_filter("PNG image", &["png"])
                .save_file()
            {
                match viewer.cover.save_png(&path) {
                    Ok(()) => self.status = format!("Saved cover to {}", path.display()),
                    Err(error) => self.status = format!("Could not save cover: {error}"),
                }
            }
        }
        if keep_open {
            self.cover_viewer = Some(viewer);
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
                    if self
                        .playing
                        .as_ref()
                        .is_some_and(|playing| playing.track.0 == list)
                    {
                        let mut extra: Vec<_> = (first_new..playlist.tracks.len()).collect();
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

    fn plan(&mut self, (list, index): Track) {
        let len = self.library.playlists[list].tracks.len();
        self.order.clear();
        if self.library.shuffle {
            self.order.extend((0..len).filter(|&track| track != index));
            self.order.shuffle(&mut rand::rng());
            self.order.insert(0, index);
            self.cursor = 0;
        } else if let Some(view) = self.views.get(&list) {
            self.order.extend_from_slice(&view.order);
            self.order.extend(view.tracks.len()..len);
            self.cursor = self
                .order
                .iter()
                .position(|&item| item == index)
                .unwrap_or(index);
        } else {
            self.order.extend(0..len);
            self.cursor = index;
        }
    }

    fn request_index(&mut self, list: usize, ctx: &egui::Context) {
        let playlist = &self.library.playlists[list];
        let cached = self.views.get(&list).map_or(0, |view| view.tracks.len());
        if cached >= playlist.tracks.len() || self.indexing.contains_key(&list) {
            return;
        }
        let tracks = playlist.tracks[cached..].to_vec();

        let (tx, rx) = mpsc::channel();
        self.indexing.insert(list, rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(tracks.iter().map(|path| TrackInfo::read(path)).collect());
            ctx.request_repaint();
        });
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
        if let Some(playing) = &self.playing {
            if !self.library.shuffle && changed.contains(&playing.track.0) {
                self.plan(playing.track);
            }
        }
    }

    fn open(
        &mut self,
        track: Track,
        ctx: &egui::Context,
        restore: Option<Duration>,
    ) -> Result<(), String> {
        self.setup.stop_tone();
        let path = &self.library.playlists[track.0].tracks[track.1];
        let sink = audio::AudioTrack::open(
            path,
            self.output_request(),
            restore.unwrap_or_default(),
            self.library.volume,
            true,
            ctx,
        )?;
        self.audio_release = Some(sink.release_gate());
        self.audio_retry_after = None;
        if let Some(previous) = &self.playing {
            previous.sink.pause();
        }
        self.observe_lastfm();
        self.show_track(track, sink, ctx);
        if restore.is_none()
            && let Some(playing) = &self.playing
        {
            playing.sink.play();
        }
        Ok(())
    }

    fn show_track(&mut self, track: Track, sink: audio::AudioTrack, ctx: &egui::Context) {
        let duration = sink.duration;
        let path = &self.library.playlists[track.0].tracks[track.1];
        let mut media = Media::read(path);
        let timed_lyrics = library::timed_lyrics(&media.lyrics);
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
        self.artwork_url = self
            .library
            .artwork_urls
            .get(path)
            .cloned()
            .unwrap_or_default();
        self.gapless_generation = self.gapless_generation.wrapping_add(1);
        self.lastfm.track_changed();
        self.playing = Some(Playing {
            counted: false,
            lastfm: lastfm::Listen::default(),
            track,
            sink,
            duration,
            media,
            cover,
            online_cover: None,
            online_url: None,
            lookup: None,
            artwork_status: "",
            timed_lyrics,
            lyrics_lookup: None,
            lyrics_status: String::new(),
            lyrics_online: false,
        });
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
        let track = &self.library.playlists[playing.track.0].tracks[playing.track.1];
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

    fn start(&mut self, track: Track, ctx: &egui::Context) {
        match self.open(track, ctx, None) {
            Ok(()) => {
                self.plan(track);
                self.refresh_gapless();
                self.status.clear();
                self.save_playback(Duration::ZERO);
                self.sync_presence();
            }
            Err(error) => self.status = format!("Could not play track: {error}"),
        }
    }

    fn step(&mut self, offset: isize, ctx: &egui::Context) {
        let Some(list) = self.playing.as_ref().map(|p| p.track.0) else {
            return;
        };
        let Some(mut cursor) = self.cursor.checked_add_signed(offset) else {
            return;
        };
        let mut failed = None;
        while let Some(&track) = self.order.get(cursor) {
            match self.open((list, track), ctx, None) {
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
            let Some(next) = cursor.checked_add_signed(offset) else {
                break;
            };
            cursor = next;
        }
        if self.playing.as_ref().is_some_and(|p| p.sink.empty()) {
            self.stop();
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
        self.step(-1, ctx);
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
            let list = self.library.selected;
            let len = self.library.playlists[list].tracks.len();
            if len > 0 {
                let index = if self.library.shuffle {
                    rand::random_range(0..len)
                } else {
                    self.views
                        .get(&list)
                        .and_then(|view| view.order.first().copied())
                        .unwrap_or(0)
                };
                self.start((list, index), ctx);
            }
        }
    }

    fn stop(&mut self) {
        self.observe_lastfm();
        self.lastfm.track_changed();
        self.save_playback(Duration::ZERO);
        self.playing = None;
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
        if let Some(playing) = &self.playing {
            self.plan(playing.track);
        }
        self.save();
    }

    fn taskbar_buttons(&self) -> taskbar::Buttons {
        let loaded = self.playing.is_some();
        taskbar::Buttons {
            previous: (loaded && self.cursor > 0) || self.previous_restarts_track(),
            can_play: loaded
                || !self.library.playlists[self.library.selected]
                    .tracks
                    .is_empty(),
            playing: self.playing.as_ref().is_some_and(|p| !p.sink.is_paused()),
            next: loaded && self.cursor + 1 < self.order.len(),
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
            .map(|playing| {
                let path = &self.library.playlists[playing.track.0].tracks[playing.track.1];
                let artwork = self
                    .library
                    .artwork_urls
                    .get(path)
                    .map(String::as_str)
                    .filter(|url| presence::public_artwork_url(url))
                    .or(playing.online_url.as_deref())
                    .unwrap_or_default();
                presence::Activity {
                    title: playing.media.title.clone(),
                    artist: playing.media.artist.clone(),
                    album: playing.media.album.clone(),
                    artwork: artwork.into(),
                    fallback_artwork: self.library.fallback_asset.trim().into(),
                    elapsed: playing.sink.get_pos(),
                    duration: playing.duration,
                }
            });
        self.rpc
            .update(enabled.then(|| app_id.to_owned()), activity);
    }

    fn transport(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let active = self.playing.is_some();
        let paused = self.playing.as_ref().is_some_and(|p| p.sink.is_paused());
        ui.horizontal_wrapped(|ui| {
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if icon(ui, Icon::Stop, "Stop", active, false) {
                    self.stop();
                }
                if icon(
                    ui,
                    Icon::Play,
                    "Play",
                    paused
                        || (!active
                            && !self.library.playlists[self.library.selected]
                                .tracks
                                .is_empty()),
                    false,
                ) {
                    self.resume(ctx);
                }
                if icon(ui, Icon::Pause, "Pause", active && !paused, false) {
                    self.pause();
                }
                if icon(
                    ui,
                    Icon::Previous,
                    "Previous track",
                    self.taskbar_buttons().previous,
                    false,
                ) {
                    self.previous(ctx);
                }
                if icon(
                    ui,
                    Icon::Next,
                    "Next track",
                    self.playing.is_some() && self.cursor + 1 < self.order.len(),
                    false,
                ) {
                    self.step(1, ctx);
                }
                if icon(ui, Icon::Shuffle, "Shuffle", true, self.library.shuffle) {
                    self.toggle_shuffle();
                }
            });
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(4.0);
            let volume = ui
                .add_enabled_ui(self.software_volume_available(), |ui| {
                    volume_slider(ui, &mut self.library.volume)
                })
                .inner;
            if volume.changed() {
                self.volume_adjusted();
            }
            let starred = self.playing.as_ref().is_some_and(|playing| {
                let path = &self.library.playlists[playing.track.0].tracks[playing.track.1];
                self.library
                    .favorites
                    .is_some_and(|index| self.library.playlists[index].tracks.contains(path))
            });
            let label = if starred {
                "Remove from Favorites"
            } else {
                "Add to Favorites"
            };
            if ui
                .add_enabled(
                    self.playing.is_some(),
                    egui::Button::new(if starred { "♥" } else { "♡" })
                        .min_size(egui::vec2(34.0, 30.0)),
                )
                .on_hover_text(label)
                .clicked()
            {
                self.toggle_favorite();
            }
        });
        if let Some(playing) = &self.playing {
            let elapsed = playing.sink.get_pos();
            if let Some(duration) = playing.duration.filter(|d| !d.is_zero()) {
                let target = ui
                    .push_id(("seek", playing.track), |ui| {
                        seek_bar(ui, elapsed, duration)
                    })
                    .inner;
                if let Some(target) = target {
                    self.seek(target, ctx);
                }
            } else {
                ui.label(time(elapsed));
            }
        }
    }

    fn discord_dialog(&mut self, ctx: &egui::Context) {
        let mut open = self.discord_settings;
        let mut save = false;
        let mut lookup_changed = false;
        let mut refresh = false;
        egui::Window::new("Discord Rich Presence").open(&mut open).show(ctx, |ui| {
            save |= ui.checkbox(&mut self.library.discord_enabled, "Share what I’m listening to").changed();
            lookup_changed = ui.checkbox(&mut self.library.online_artwork, "Look up artwork via Apple Music").changed();
            ui.weak("Artwork lookup sends track metadata to Apple's music catalog.");
            ui.label("Discord Application ID");
            save |= ui.text_edit_singleline(&mut self.library.discord_id).lost_focus();
            if self.library.discord_enabled && (self.library.discord_id.trim().is_empty()
                || !self.library.discord_id.trim().bytes().all(|b| b.is_ascii_digit())) {
                ui.label("Enter the numeric Application ID from the Discord Developer Portal.");
            }
            ui.label("Default artwork asset key or public HTTPS image URL");
            save |= ui.text_edit_singleline(&mut self.library.fallback_asset).lost_focus();
            if let Some(playing) = &self.playing {
                ui.label("Public HTTPS artwork URL for this track");
                ui.text_edit_singleline(&mut self.artwork_url);
                if ui.button("Save track artwork URL").clicked() {
                    let path = &self.library.playlists[playing.track.0].tracks[playing.track.1];
                    let url = self.artwork_url.trim();
                    if url.is_empty() {
                        self.library.artwork_urls.remove(path);
                        save = true;
                    } else if presence::public_artwork_url(url) {
                        self.library.artwork_urls.insert(path.clone(), url.to_owned());
                        save = true;
                    } else {
                        ui.label("Enter a public HTTPS image URL.");
                    }
                }
                if !playing.artwork_status.is_empty() {
                    ui.weak(playing.artwork_status);
                }
            }
            ui.label("Discord displays public images or uploaded assets. Local cover files stay on this device.");
            ui.label(self.rpc.status());
            ui.horizontal(|ui| {
                if ui.button("Save settings").clicked() { save = true; }
                if ui.button("Refresh presence").clicked() { refresh = true; }
            });
        });
        self.discord_settings = open;
        if lookup_changed || refresh {
            self.lookup_artwork(ctx);
            save |= lookup_changed;
        }
        if save {
            self.save();
        }
        if save || refresh {
            self.sync_presence();
        }
    }

    fn activity_dialog(&mut self, ctx: &egui::Context) {
        egui::Window::new("Listening activity")
            .open(&mut self.activity_open)
            .default_width(540.0)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.selectable_value(&mut self.activity_tab, ActivityTab::History, "History");
                    ui.selectable_value(
                        &mut self.activity_tab,
                        ActivityTab::Songs,
                        "Most played songs",
                    );
                    ui.selectable_value(
                        &mut self.activity_tab,
                        ActivityTab::Artists,
                        "Most played artists",
                    );
                });
                ui.separator();
                if self.listening.history.is_empty() {
                    ui.label("Play a track to start your listening history.");
                    return;
                }
                let ranks = if self.activity_tab == ActivityTab::History {
                    None
                } else {
                    Some(
                        self.listening_ranks
                            .get_or_insert_with(|| self.listening.rankings()),
                    )
                };
                let count = match (&self.activity_tab, ranks.as_deref()) {
                    (ActivityTab::Songs, Some(ranks)) => ranks.songs.len(),
                    (ActivityTab::Artists, Some(ranks)) => ranks.artists.len(),
                    _ => self.listening.history.len(),
                };
                if count == 0 {
                    ui.label("No tagged artists in your listening history.");
                }
                let now = listening::now();
                let row_height = ui.text_style_height(&egui::TextStyle::Body) + 10.0;
                egui::ScrollArea::vertical().max_height(420.0).show_rows(
                    ui,
                    row_height,
                    count,
                    |ui, visible| {
                        for index in visible {
                            let (title, artist, value) =
                                match (&self.activity_tab, ranks.as_deref()) {
                                    (ActivityTab::Songs, Some(ranks)) => {
                                        let song = &self.listening.songs[&ranks.songs[index]];
                                        (
                                            song.title.as_str(),
                                            song.artist.as_str(),
                                            song.plays.to_string(),
                                        )
                                    }
                                    (ActivityTab::Artists, Some(ranks)) => {
                                        let artist = &self.listening.artists[&ranks.artists[index]];
                                        (artist.name.as_str(), "", artist.plays.to_string())
                                    }
                                    _ => {
                                        let entry = &self.listening.history[count - 1 - index];
                                        (
                                            entry.title.as_str(),
                                            entry.artist.as_str(),
                                            listening::age(now.saturating_sub(entry.time)),
                                        )
                                    }
                                };
                            ui.allocate_ui_with_layout(
                                egui::vec2(ui.available_width(), row_height),
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.weak(value);
                                    let text = if artist.is_empty() {
                                        (*title).to_owned()
                                    } else {
                                        format!("{title} · {artist}")
                                    };
                                    ui.with_layout(
                                        egui::Layout::left_to_right(egui::Align::Center),
                                        |ui| {
                                            ui.add(egui::Label::new(&text).truncate())
                                                .on_hover_text(text);
                                        },
                                    );
                                },
                            );
                        }
                    },
                );
            });
    }

    fn theme_editor(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.theme_draft.take() else {
            return;
        };
        let previous = draft.theme;
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new("Theme Editor")
            .open(&mut open)
            .collapsible(false)
            .default_width(440.0)
            .show(ctx, |ui| {
                let max_height = (ctx.content_rect().height() * 0.70).clamp(180.0, 560.0);
                egui::ScrollArea::vertical()
                    .max_height(max_height)
                    .show(ui, |ui| {
                        draft.saved_theme_settings(ui);
                        ui.separator();
                        let name = theme::Theme::PRESETS
                            .iter()
                            .find(|(_, preset)| *preset == draft.theme)
                            .map_or("Custom", |(name, _)| *name);
                        egui::ComboBox::from_label("Preset")
                            .selected_text(name)
                            .show_ui(ui, |ui| {
                                for (name, preset) in theme::Theme::PRESETS {
                                    ui.selectable_value(&mut draft.theme, preset, name);
                                }
                            });
                        ui.separator();
                        ui.group(|ui| {
                            ui.strong("Colors");
                            ui.weak("Tune the overall palette. Changes preview immediately.");
                            egui::Grid::new("theme colors")
                                .spacing([20.0, 8.0])
                                .show(ui, |ui| {
                                    for (label, color) in [
                                        ("Background", &mut draft.theme.background),
                                        ("Panels and buttons", &mut draft.theme.surface),
                                        ("Text", &mut draft.theme.text),
                                        ("Accent", &mut draft.theme.accent),
                                    ] {
                                        ui.label(label);
                                        ui.color_edit_button_srgb(color);
                                        ui.end_row();
                                    }
                                });
                        });
                        ui.separator();
                        ui.group(|ui| {
                            ui.strong("Layout");
                            ui.horizontal_wrapped(|ui| {
                                ui.label("Density");
                                for density in theme::Density::ALL {
                                    ui.selectable_value(
                                        &mut draft.theme.density,
                                        density,
                                        density.label(),
                                    );
                                }
                            });
                            ui.add(
                                egui::Slider::new(&mut draft.theme.corner_radius, 0..=20)
                                    .text("Corner roundness"),
                            );
                            ui.add(
                                egui::Slider::new(
                                    &mut draft.theme.surface_strength_percent,
                                    45..=90,
                                )
                                .text("Panel contrast")
                                .suffix("%"),
                            );
                            ui.add(
                                egui::Slider::new(
                                    &mut draft.theme.accent_strength_percent,
                                    15..=75,
                                )
                                .text("Highlight strength")
                                .suffix("%"),
                            );
                            ui.checkbox(&mut draft.theme.high_contrast, "Increase border contrast")
                                .on_hover_text("Makes panel and button boundaries more visible.");
                        });
                        ui.separator();
                        ui.group(|ui| {
                            ui.strong("Typography");
                            ui.add(
                                egui::Slider::new(&mut draft.theme.font_scale_percent, 85..=135)
                                    .text("Text size")
                                    .suffix("%"),
                            );
                            let font = draft
                                .font
                                .as_ref()
                                .and_then(|path| path.file_name())
                                .map(|name| name.to_string_lossy())
                                .unwrap_or_else(|| "Default font".into());
                            ui.label(format!("Font: {font}"));
                            ui.horizontal_wrapped(|ui| {
                                if ui.button("Choose font…").clicked() {
                                    if let Some(path) = rfd::FileDialog::new()
                                        .add_filter("Fonts", &["ttf", "otf"])
                                        .pick_file()
                                    {
                                        match fonts::load(Some(&path)) {
                                            Ok(fonts) => {
                                                ctx.set_fonts(fonts);
                                                draft.font = Some(path);
                                                draft.font_error.clear();
                                            }
                                            Err(error) => {
                                                draft.font_error =
                                                    format!("Could not load font: {error}")
                                            }
                                        }
                                    }
                                }
                                if ui.button("Use default font").clicked() {
                                    ctx.set_fonts(egui::FontDefinitions::default());
                                    draft.font = None;
                                    draft.font_error.clear();
                                }
                            });
                            if !draft.font_error.is_empty() {
                                ui.label(&draft.font_error);
                            }
                        });
                        ui.separator();
                        ui.group(|ui| {
                            ui.strong("Background");
                            ui.weak(
                                "Use a wallpaper or let the current album artwork set the mood.",
                            );
                            draft.background.settings(
                                ui,
                                &mut draft.copy_background,
                                self.background.status(),
                            );
                        });
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    apply = ui.button("Apply").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        if apply {
            if draft.copy_background
                && draft.background.uses_file()
                && let Some(path) = &draft.background.path
            {
                match background::copy_to_profile(path, &self.library_path) {
                    Ok(copied) => {
                        for profile in &mut draft.themes {
                            if profile.background.path.as_ref() == Some(path) {
                                profile.background.path = Some(copied.clone());
                            }
                        }
                        draft.background.path = Some(copied);
                    }
                    Err(error) => {
                        draft.theme_error = format!("Could not copy the background: {error}");
                        self.theme_draft = Some(draft);
                        return;
                    }
                }
            }
            self.library.theme = draft.theme;
            self.library.font = draft.font.clone();
            self.library.background = draft.background.clone();
            self.library.named_themes = std::mem::take(&mut draft.themes);
            self.save();
        }
        if !open || apply || cancel {
            if !apply && draft.font != self.library.font {
                ctx.set_fonts(fonts::load(self.library.font.as_deref()).unwrap_or_default());
            }
            apply_ui_chrome(ctx, self.library.theme);
            ctx.request_repaint();
        } else {
            if draft.theme != previous {
                apply_ui_chrome(ctx, draft.theme);
                ctx.request_repaint();
            }
            self.theme_draft = Some(draft);
        }
    }

    fn plugin_manager(&mut self, ctx: &egui::Context) {
        let mut changed = false;
        let mut remove = None;
        egui::Window::new("Plugins")
            .open(&mut self.plugins_open)
            .default_width(460.0)
            .min_width(420.0)
            .min_height(230.0)
            .show(ctx, |ui| {
                ui.weak("Add, enable, and manage local Rhai extensions.");
                ui.add_space(3.0);
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            self.library.plugins.len() < 32,
                            egui::Button::new("Add plugin…"),
                        )
                        .clicked()
                    {
                        let mut picker =
                            rfd::FileDialog::new().add_filter("Rhai plugin", &["rhai"]);
                        if let Ok(folder) = plugins::create_folder(&self.library_path) {
                            picker = picker.set_directory(folder);
                        }
                        if let Some(path) = picker.pick_file() {
                            if !self
                                .library
                                .plugins
                                .iter()
                                .any(|plugin| plugin.path == path)
                            {
                                self.library.plugins.push(plugins::PluginConfig {
                                    path,
                                    enabled: false,
                                });
                                changed = true;
                            }
                        }
                    }
                    if ui.button("Open Plugin Folder").clicked() {
                        let result =
                            plugins::create_folder(&self.library_path).and_then(|folder| {
                                std::process::Command::new("explorer.exe")
                                    .arg(folder)
                                    .spawn()
                            });
                        if let Err(error) = result {
                            self.status = format!("Could not open plugin folder: {error}");
                        }
                    }
                    changed |= ui.button("Reload").clicked();
                });
                if self.library.plugins.is_empty() {
                    ui.add_space(18.0);
                    ui.vertical_centered(|ui| {
                        ui.strong("No plugins installed");
                        ui.weak("Add a .rhai plugin to extend Rust Player.");
                    });
                    ui.add_space(18.0);
                }
                egui::ScrollArea::vertical()
                    .max_height(340.0)
                    .show(ui, |ui| {
                        for (index, config) in self.library.plugins.iter_mut().enumerate() {
                            ui.horizontal_wrapped(|ui| {
                                let name = config.name();
                                changed |= ui
                                    .checkbox(&mut config.enabled, name)
                                    .on_hover_text(config.path.display().to_string())
                                    .changed();
                                ui.weak(self.plugins.status(index));
                                if ui.button("Remove").clicked() {
                                    remove = Some(index);
                                    changed = true;
                                }
                            });
                        }
                    });
            });
        if let Some(index) = remove {
            self.library.plugins.remove(index);
        }
        if changed {
            self.plugins = plugins::Host::load(&self.library.plugins);
            if let Some(playing) = &self.playing {
                self.plugins.track_changed(&playing.media);
            }
            self.save();
            ctx.request_repaint();
        }
    }
}

impl eframe::App for Player {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.setup.update(ctx);
        self.lastfm.poll(ctx);
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
        if let Some(draft) = &mut self.theme_draft {
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
        self.finish_artwork(ctx);
        self.finish_lyrics();
        self.finish_indexes();
        self.request_index(self.library.selected, ctx);
        if let Some(playing) = &self.playing {
            self.request_index(playing.track.0, ctx);
        }
        if self
            .playing
            .as_ref()
            .is_some_and(|playing| playing.sink.empty())
        {
            self.advance_automatic(ctx);
        }
        if self.playing.as_ref().is_some_and(|p| !p.sink.is_paused()) || self.discord_settings {
            ctx.request_repaint_after(Duration::from_millis(200));
        }

        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.add_space(5.0);
            ui.horizontal_wrapped(|ui| {
                ui.strong(RichText::new("Rust Player").size(16.0));
                ui.weak("Local library");
                ui.add_space(6.0);
                if ui
                    .add(egui::Button::new("Add files").min_size(egui::vec2(78.0, 27.0)))
                    .clicked()
                {
                    if let Some(files) = rfd::FileDialog::new()
                        .add_filter("Audio and CUE sheets", library::AUDIO)
                        .pick_files()
                    {
                        self.import(files, ctx);
                    }
                }
                if ui
                    .add(egui::Button::new("Add folder").min_size(egui::vec2(84.0, 27.0)))
                    .clicked()
                {
                    if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                        self.import(vec![folder], ctx);
                    }
                }
                ui.separator();
                if ui.button("Discord").clicked() {
                    self.discord_settings = true;
                }
                if ui.button("Theme").clicked() {
                    self.open_theme_editor();
                }
                if ui.button("Plugins").clicked() {
                    self.plugins_open = true;
                }
                if ui.button("Settings").clicked() {
                    self.settings_page
                        .get_or_insert_with(settings::Page::default);
                }
            });
            ui.add_space(5.0);
        });

        egui::TopBottomPanel::bottom("transport").show(ctx, |ui| {
            ui.add_space(6.0);
            self.transport(ui, ctx);
            if !self.status.is_empty() {
                ui.label(&self.status);
            }
            ui.add_space(6.0);
        });

        egui::SidePanel::left("playlists")
            .default_width(210.0)
            .min_width(175.0)
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("PLAYLISTS").strong().size(12.0));
                    if ui.button("+").on_hover_text("New playlist").clicked() {
                        self.playlist_dialog = Some(PlaylistDialog::Name {
                            target: None,
                            name: String::new(),
                        });
                    }
                });
                let mut selected = self.library.selected;
                egui::Frame::new()
                    .fill(ui.visuals().widgets.inactive.weak_bg_fill)
                    .corner_radius(8)
                    .inner_margin(egui::Margin::symmetric(7, 5))
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            if ui.button("Import…").clicked() {
                                self.import_playlists();
                                selected = self.library.selected;
                            }
                            if ui.button("Listening activity").clicked() {
                                self.activity_open = true;
                            }
                        });
                    });
                ui.add_space(6.0);
                let mut action = None;
                let mut reorder = None;
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for index in self.library.sidebar_order() {
                        let list = &self.library.playlists[index];
                        let pinned = list.pinned || Some(index) == self.library.favorites;
                        let response = ui
                            .horizontal(|ui| {
                                let (rect, gutter) = ui.allocate_exact_size(
                                    egui::vec2(13.0, ui.spacing().interact_size.y),
                                    if pinned {
                                        egui::Sense::click()
                                    } else {
                                        egui::Sense::hover()
                                    },
                                );
                                if pinned {
                                    let center = rect.center();
                                    let stroke =
                                        egui::Stroke::new(1.3_f32, ui.visuals().text_color());
                                    for ((x1, y1), (x2, y2)) in [
                                        ((0.0, -5.0), (5.0, 0.0)),
                                        ((2.5, -2.5), (0.0, 0.0)),
                                        ((-2.5, -2.5), (2.5, 2.5)),
                                        ((0.0, 0.0), (-4.5, 4.5)),
                                    ] {
                                        ui.painter().line_segment(
                                            [
                                                center + egui::vec2(x1, y1),
                                                center + egui::vec2(x2, y2),
                                            ],
                                            stroke,
                                        );
                                    }
                                }
                                let sense = if Some(index) == self.library.favorites {
                                    egui::Sense::click()
                                } else {
                                    egui::Sense::click_and_drag()
                                };
                                ui.add(
                                    egui::Button::selectable(selected == index, &list.name)
                                        .sense(sense),
                                )
                                .union(gutter)
                            })
                            .inner;
                        if response.clicked() {
                            selected = index;
                        }
                        if response.drag_started_by(egui::PointerButton::Primary) {
                            response.dnd_set_drag_payload(index);
                        }
                        if let Some(from) = response.dnd_hover_payload::<usize>() {
                            if self.library.can_move_playlist(*from, index) {
                                if let Some(pointer) = ctx.pointer_interact_pos() {
                                    let after = pointer.y >= response.rect.center().y;
                                    let y = if after {
                                        response.rect.bottom()
                                    } else {
                                        response.rect.top()
                                    };
                                    ui.painter().hline(
                                        response.rect.left()..=ui.max_rect().right(),
                                        y,
                                        egui::Stroke::new(
                                            2.0_f32,
                                            ui.visuals().selection.stroke.color,
                                        ),
                                    );
                                    if response.dnd_release_payload::<usize>().is_some() {
                                        reorder = Some((*from, index, after));
                                    }
                                }
                            }
                        }
                        response.context_menu(|ui| {
                            if Some(index) != self.library.favorites {
                                let label = if list.pinned { "Unpin" } else { "Pin" };
                                if ui.button(label).clicked() {
                                    action = Some((index, PlaylistAction::TogglePin));
                                    ui.close();
                                }
                                ui.separator();
                            }
                            for (label, item) in [
                                ("Rename", PlaylistAction::Rename),
                                ("Copy", PlaylistAction::Copy),
                                ("Delete", PlaylistAction::Delete),
                                ("Export", PlaylistAction::Export),
                            ] {
                                if ui.button(label).clicked() {
                                    action = Some((index, item));
                                    ui.close();
                                }
                            }
                        });
                    }
                });
                let order_changed = reorder
                    .is_some_and(|(from, to, after)| self.library.move_playlist(from, to, after));
                if selected != self.library.selected || order_changed {
                    self.library.selected = selected;
                    self.save();
                }
                if let Some((index, action)) = action {
                    self.playlist_action(index, action);
                }
            });

        let mut cover_open = None;
        egui::SidePanel::right("now_playing")
            .default_width(290.0)
            .min_width(220.0)
            .show(ctx, |ui| {
                ui.heading("Now playing");
                self.online_lyrics_setting(ui);
                let mut search = false;
                let mut save = false;
                if let Some(playing) = &self.playing {
                    egui::Frame::new()
                        .fill(ui.visuals().faint_bg_color)
                        .corner_radius(12)
                        .inner_margin(egui::Margin::same(10))
                        .show(ui, |ui| {
                            if let Some(cover) = playing.cover() {
                                let cover = &cover.texture;
                                let size = cover.size_vec2();
                                let response = ui.add(
                                    egui::Image::new((
                                        cover.id(),
                                        size * (ui
                                            .available_width()
                                            .min(ui.available_height() * 0.48)
                                            .min(380.0)
                                            / size.x.max(size.y)),
                                    ))
                                    .sense(egui::Sense::click()),
                                );
                                if response.double_clicked() {
                                    cover_open = Some((
                                        playing.cover().expect("visible cover").pixels(),
                                        playing.media.title.clone(),
                                        playing.media.artist.clone(),
                                    ));
                                }
                                response.on_hover_text("Double-click to open artwork");
                            } else {
                                let edge = ui.available_width().min(380.0);
                                ui.allocate_ui_with_layout(
                                    egui::vec2(edge, edge),
                                    egui::Layout::top_down(egui::Align::Center)
                                        .with_main_justify(true),
                                    |ui| ui.weak("No cover artwork"),
                                );
                            }
                            ui.add_space(6.0);
                            ui.label(RichText::new(&playing.media.title).strong().size(16.0));
                            if !playing.media.artist.is_empty() {
                                ui.label(RichText::new(&playing.media.artist).size(13.5));
                            }
                            if !playing.media.album.is_empty() {
                                ui.weak(&playing.media.album);
                            }
                        });
                    if (0..self.library.plugins.len())
                        .any(|index| !self.plugins.output(index).is_empty())
                    {
                        egui::CollapsingHeader::new("Plugin details")
                            .default_open(true)
                            .show(ui, |ui| {
                                egui::ScrollArea::vertical()
                                    .id_salt("plugin output")
                                    .max_height(85.0)
                                    .show(ui, |ui| {
                                        for (index, plugin) in
                                            self.library.plugins.iter().enumerate()
                                        {
                                            let output = self.plugins.output(index);
                                            if !output.is_empty() {
                                                ui.label(output).on_hover_text(plugin.name());
                                            }
                                        }
                                    });
                            });
                    }
                    ui.separator();
                    ui.heading("Lyrics");
                    ui.horizontal_wrapped(|ui| {
                        search = ui
                            .add_enabled(
                                playing.lyrics_lookup.is_none(),
                                egui::Button::new("Search LRCLIB"),
                            )
                            .on_hover_text("Send this track’s title and artist to LRCLIB.")
                            .clicked();
                        if playing.lyrics_online {
                            save = ui.button("Save lyrics").clicked();
                        }
                    });
                    if !playing.lyrics_status.is_empty() {
                        ui.weak(&playing.lyrics_status);
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("lyrics")
                        .show(ui, |ui| {
                            if !playing.timed_lyrics.is_empty() {
                                let position = playing.sink.get_pos();
                                let current = playing
                                    .timed_lyrics
                                    .partition_point(|(timestamp, _)| *timestamp <= position)
                                    .checked_sub(1);
                                let id = ui.id().with(("lyric-line", playing.track));
                                let last = ctx.data_mut(|data| {
                                    let last = data.get_temp::<usize>(id);
                                    if let Some(current) = current {
                                        data.insert_temp(id, current);
                                    }
                                    last
                                });
                                for (index, (_, line)) in playing.timed_lyrics.iter().enumerate() {
                                    if current == Some(index) {
                                        let row = ui.label(RichText::new(line).strong());
                                        if last != current {
                                            row.scroll_to_me(Some(egui::Align::Center));
                                        }
                                    } else {
                                        ui.weak(line);
                                    }
                                }
                            } else if playing.media.lyrics.is_empty() {
                                if playing.lyrics_status.is_empty() {
                                    ui.weak(
                                        "No local lyrics found. Use Search LRCLIB to look online.",
                                    );
                                }
                            } else {
                                ui.label(&playing.media.lyrics);
                            }
                        });
                } else {
                    ui.weak("Nothing playing");
                }
                if search {
                    self.lookup_lyrics(ctx);
                }
                if save {
                    self.save_lyrics();
                }
            });
        if let Some((pixels, title, artist)) = cover_open {
            self.open_cover_viewer(ctx, pixels, title, artist);
        }

        let background = self
            .theme_draft
            .as_ref()
            .map_or(&self.library.background, |draft| &draft.background)
            .clone();
        self.background.update(
            &background,
            self.playing.as_ref().and_then(Playing::cover),
            ctx,
        );
        egui::CentralPanel::default().show(ctx, |ui| {
            self.background.paint(&background, ui);
            let list = self.library.selected;
            let playlist = &mut self.library.playlists[list];
            let old_sort = (playlist.sort, playlist.descending);
            ui.horizontal_wrapped(|ui| {
                egui::ComboBox::from_label("Sort by")
                    .selected_text(playlist.sort.label())
                    .show_ui(ui, |ui| {
                        for sort in [
                            Sort::Added,
                            Sort::Title,
                            Sort::Artist,
                            Sort::Album,
                            Sort::Duration,
                        ] {
                            ui.selectable_value(&mut playlist.sort, sort, sort.label());
                        }
                    });
                ui.add_enabled_ui(playlist.sort != Sort::Added, |ui| {
                    let (ascending, descending) = if playlist.sort == Sort::Duration {
                        ("Shortest first", "Longest first")
                    } else {
                        ("A–Z", "Z–A")
                    };
                    ui.selectable_value(&mut playlist.descending, false, ascending);
                    ui.selectable_value(&mut playlist.descending, true, descending);
                });
            });
            if old_sort != (playlist.sort, playlist.descending) {
                if let Some(view) = self.views.remove(&list) {
                    self.views.insert(
                        list,
                        PlaylistView::build(view.tracks, playlist.sort, playlist.descending),
                    );
                }
                if !self.library.shuffle {
                    if let Some(track) = self
                        .playing
                        .as_ref()
                        .map(|p| p.track)
                        .filter(|track| track.0 == list)
                    {
                        self.plan(track);
                    }
                }
                self.request_index(list, ctx);
                self.save();
            }
            let playlist = &self.library.playlists[list];
            egui::Frame::new()
                .fill(ui.visuals().widgets.inactive.weak_bg_fill)
                .corner_radius(10)
                .inner_margin(egui::Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&playlist.name).strong().size(18.0));
                        ui.weak(format!("{} tracks", playlist.tracks.len()));
                        ui.add_space(8.0);
                        let search = ui.add_sized(
                            egui::vec2(250.0, ui.spacing().interact_size.y),
                            egui::TextEdit::singleline(&mut self.track_search)
                                .hint_text("Search songs or artists…"),
                        );
                        if !self.track_search.is_empty() && ui.small_button("Clear").clicked() {
                            self.track_search.clear();
                        }
                        search.on_hover_text("Filters by title, artist, album, or file name");
                    });
                });
            ui.add_space(6.0);
            if self.indexing.contains_key(&list) {
                ui.weak("Reading track information…");
            }
            if playlist.tracks.is_empty() {
                ui.label("Drop audio files or folders here.");
            }
            let mut play = None;
            let mut edit = None;
            let mut properties = None;
            let mut folder = None;
            let (album_fill, album_text) = self
                .theme_draft
                .as_ref()
                .map(|draft| draft.theme)
                .unwrap_or(self.library.theme)
                .album_colors();
            let view = self.views.get(&list);
            let query = self.track_search.trim().to_lowercase();
            let filtered_rows = (!query.is_empty()).then(|| {
                let mut rows = Vec::new();
                if let Some(view) = view {
                    let mut pending_album = None;
                    for row in &view.rows {
                        match row {
                            PlaylistRow::Album(album) => pending_album = Some(album.clone()),
                            PlaylistRow::Track(index) => {
                                let track = &view.tracks[*index];
                                let matches = [
                                    track.title.as_str(),
                                    track.artist.as_str(),
                                    track.album.as_str(),
                                    track.album_artist.as_str(),
                                ]
                                .into_iter()
                                .any(|value| value.to_lowercase().contains(&query));
                                if matches {
                                    if let Some(album) = pending_album.take() {
                                        rows.push(PlaylistRow::Album(album));
                                    }
                                    rows.push(PlaylistRow::Track(*index));
                                }
                            }
                        }
                    }
                } else {
                    rows.extend(
                        playlist
                            .tracks
                            .iter()
                            .enumerate()
                            .filter(|(_, path)| file_title(path).to_lowercase().contains(&query))
                            .map(|(index, _)| PlaylistRow::Track(index)),
                    );
                }
                rows
            });
            if let Some(rows) = &filtered_rows {
                let matches = rows
                    .iter()
                    .filter(|row| matches!(row, PlaylistRow::Track(_)))
                    .count();
                ui.weak(format!("{matches} matching tracks"));
            }
            let height = ui.text_style_height(&egui::TextStyle::Body) + 7.0;
            playlist_columns(
                ui,
                height,
                ["", "Track", "Title / Artist", "Time"],
                true,
                false,
                false,
                "",
            );
            egui::ScrollArea::vertical().show_rows(
                ui,
                height,
                filtered_rows.as_ref().map_or_else(
                    || {
                        view.map_or(playlist.tracks.len(), |view| {
                            view.row_count(playlist.tracks.len())
                        })
                    },
                    Vec::len,
                ),
                |ui, rows| {
                    for row_index in rows {
                        let row = filtered_rows
                            .as_ref()
                            .and_then(|rows| rows.get(row_index))
                            .or_else(|| view.and_then(|view| view.rows.get(row_index)));
                        let index = match row {
                            Some(PlaylistRow::Album(album)) => {
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(ui.available_width(), height),
                                    egui::Sense::hover(),
                                );
                                let painter =
                                    ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
                                painter.rect_filled(rect, 3.0, album_fill);
                                painter.text(
                                    rect.left_center() + egui::vec2(9.0, 0.0),
                                    egui::Align2::LEFT_CENTER,
                                    album,
                                    egui::FontId::proportional(13.0),
                                    album_text,
                                );
                                continue;
                            }
                            Some(PlaylistRow::Track(index)) => *index,
                            None => view.map_or(row_index, |view| {
                                view.tracks.len() + row_index - view.rows.len()
                            }),
                        };
                        let path = &playlist.tracks[index];
                        let active = self
                            .playing
                            .as_ref()
                            .is_some_and(|p| p.track == (list, index));
                        let metadata = view.and_then(|view| view.tracks.get(index));
                        let title = metadata
                            .map(|track| track.title.clone())
                            .unwrap_or_else(|| file_title(path));
                        let title = match metadata {
                            Some(track)
                                if !track.artist.is_empty()
                                    && track.artist != track.album_artist =>
                            {
                                format!("{title}  /  {}", track.artist)
                            }
                            _ => title,
                        };
                        let number = metadata.map(TrackInfo::track_number).unwrap_or_default();
                        let duration = metadata
                            .and_then(|track| track.duration)
                            .map(time)
                            .unwrap_or_default();
                        let marker = if active { "▶" } else { "" };
                        let response = playlist_columns(
                            ui,
                            height,
                            [marker, &number, &title, &duration],
                            false,
                            active,
                            row_index % 2 == 0,
                            &path.display().to_string(),
                        );
                        if response.double_clicked() {
                            play = Some(index);
                        }
                        response.context_menu(|ui| {
                            if ui.button("Properties").clicked() {
                                properties = Some(path.clone());
                                ui.close();
                            }
                            if ui.button("Open containing folder").clicked() {
                                folder = Some(path.clone());
                                ui.close();
                            }
                            ui.separator();
                            if ui.button("Remove from playlist").clicked() {
                                edit = Some((index, TrackEdit::Remove));
                                ui.close();
                            }
                            if playlist.sort == Sort::Added {
                                if ui
                                    .add_enabled(index > 0, egui::Button::new("Move up"))
                                    .clicked()
                                {
                                    edit = Some((index, TrackEdit::Move(index - 1)));
                                    ui.close();
                                }
                                if ui
                                    .add_enabled(
                                        index + 1 < playlist.tracks.len(),
                                        egui::Button::new("Move down"),
                                    )
                                    .clicked()
                                {
                                    edit = Some((index, TrackEdit::Move(index + 1)));
                                    ui.close();
                                }
                            }
                        });
                    }
                },
            );
            if let Some((index, change)) = edit {
                self.edit_track((list, index), change);
            } else if let Some(index) = play {
                self.start((list, index), ctx);
            }
            if let Some(path) = properties {
                self.track_properties = Some(TrackProperties::read(path));
            }
            if let Some(path) = folder {
                self.open_containing_folder(path);
            }
        });

        if let Some(mut dialog) = self.playlist_dialog.take() {
            let mut open = true;
            match &mut dialog {
                PlaylistDialog::Name { target, name } => {
                    let title = if target.is_some() {
                        "Rename playlist"
                    } else {
                        "New playlist"
                    };
                    let mut submit = false;
                    let mut cancel = false;
                    egui::Window::new(title)
                        .open(&mut open)
                        .collapsible(false)
                        .show(ctx, |ui| {
                            ui.label("Playlist name");
                            let input = ui.text_edit_singleline(name);
                            let enter =
                                input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            ui.horizontal(|ui| {
                                submit = ui
                                    .add_enabled(!name.trim().is_empty(), egui::Button::new("Save"))
                                    .clicked()
                                    || (enter && !name.trim().is_empty());
                                cancel = ui.button("Cancel").clicked();
                            });
                        });
                    if submit {
                        if let Some(list) = *target {
                            self.library.playlists[list].name = name.trim().to_owned();
                        } else {
                            self.library.playlists.push(Playlist {
                                name: name.trim().to_owned(),
                                tracks: Vec::new(),
                                pinned: false,
                                sidebar_position: None,
                                sort: Sort::Album,
                                descending: false,
                            });
                            self.library.selected = self.library.playlists.len() - 1;
                        }
                        self.save();
                        open = false;
                    }
                    open &= !cancel;
                }
                PlaylistDialog::Delete(list) => {
                    let mut confirmed = false;
                    let mut cancel = false;
                    let name = &self.library.playlists[*list].name;
                    egui::Window::new("Delete playlist?")
                        .open(&mut open)
                        .collapsible(false)
                        .show(ctx, |ui| {
                            ui.label(format!("Delete “{name}” from Rust Player?"));
                            ui.label("Your audio files will stay on disk.");
                            ui.horizontal(|ui| {
                                cancel = ui.button("Cancel").clicked();
                                confirmed = ui.button("Delete playlist").clicked();
                            });
                        });
                    if confirmed {
                        self.delete_playlist(*list);
                    }
                    open &= !confirmed && !cancel;
                }
            }
            if open {
                self.playlist_dialog = Some(dialog);
            }
        }

        if self.discord_settings {
            self.discord_dialog(ctx);
        }
        if self.activity_open {
            self.activity_dialog(ctx);
        }
        if self.theme_draft.is_some() {
            self.theme_editor(ctx);
        }
        if self.plugins_open {
            self.plugin_manager(ctx);
        }
        self.track_properties_dialog(ctx);
        self.cover_viewer_dialog(ctx);
        self.settings_dialog(ctx);
        self.audio_setup_dialog(ctx);
        self.refresh_gapless();
        self.flush_volume(ctx);
        self.refresh_windows_controls();
    }

    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        self.observe_lastfm();
        if self.volume_changed {
            self.save();
        }
        if let Some(playing) = &self.playing {
            self.save_playback(playing.sink.get_pos());
        }
    }
}

fn playlist_columns(
    ui: &mut egui::Ui,
    height: f32,
    text: [&str; 4],
    header: bool,
    active: bool,
    alternate: bool,
    tooltip: &str,
) -> egui::Response {
    use egui::{Align2, FontId, Rect, Sense, pos2, vec2};
    let sense = if header {
        Sense::hover()
    } else {
        Sense::click()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), sense);
    let painter = ui.painter();
    let fill = if active || response.hovered() && !header {
        ui.visuals().selection.bg_fill
    } else if header {
        ui.visuals().widgets.inactive.bg_fill
    } else if alternate {
        ui.visuals().widgets.inactive.weak_bg_fill
    } else {
        ui.visuals().panel_fill
    };
    let row_rect = if header {
        rect
    } else {
        rect.shrink2(vec2(1.0, 2.0))
    };
    painter.rect_filled(row_rect, if header { 6 } else { 5 }, fill);
    let left = rect.left();
    let boundaries = [
        left,
        left + 28.0,
        left + 78.0,
        rect.right() - 54.0,
        rect.right(),
    ];
    for (column, content) in text.into_iter().enumerate() {
        if boundaries[column + 1] <= boundaries[column] {
            continue;
        }
        let clip = Rect::from_min_max(
            pos2(boundaries[column], rect.top()),
            pos2(boundaries[column + 1], rect.bottom()),
        )
        .intersect(ui.clip_rect());
        let (x, align) = if column == 1 || column == 3 {
            (boundaries[column + 1] - 7.0, Align2::RIGHT_CENTER)
        } else {
            (boundaries[column] + 7.0, Align2::LEFT_CENTER)
        };
        let color = if header {
            ui.visuals().weak_text_color()
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
    response.on_hover_text(tooltip)
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
    if response.hovered() || selected {
        ui.painter()
            .rect_filled(response.rect, 3.0, ui.visuals().widgets.hovered.bg_fill);
    }
    let center = response.rect.center();
    let color = if enabled {
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
                if let Some((whole @ ("0" | "1"), fractional)) = label.split_once('.') {
                    if fractional.bytes().all(|digit| digit == b'0') {
                        label.truncate(whole.len());
                    }
                }
                label
            }),
    );
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
    response
}

fn seek_bar(ui: &mut egui::Ui, elapsed: Duration, duration: Duration) -> Option<Duration> {
    use egui::{Align2, CursorIcon, FontId, PointerButton, Sense, vec2};

    let height = 14.0;
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width().max(1.0), height),
        Sense::click_and_drag(),
    );
    let position = response.interact_pointer_pos().map(|pointer| {
        let fraction = ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
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
    let rounding = (height / 2.0) as u8;
    painter.rect_filled(rect, rounding, ui.visuals().extreme_bg_color);
    if fraction > 0.0 {
        let mut fill = rect;
        fill.max.x = rect.left() + rect.width() * fraction;
        painter.rect_filled(fill, rounding, ui.visuals().selection.bg_fill);
    }
    painter.text(
        rect.left_center() + vec2(8.0, 0.0),
        Align2::LEFT_CENTER,
        format!("{} / {}", time(preview), time(duration)),
        FontId::proportional(10.5),
        ui.visuals().text_color(),
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
            player.media_keys = media_keys::MediaKeys::new(creation, &creation.egui_ctx);
            apply_ui_chrome(&creation.egui_ctx, player.library.theme);
            if let Some(path) = player.library.font.as_deref() {
                match fonts::load(Some(path)) {
                    Ok(fonts) => creation.egui_ctx.set_fonts(fonts),
                    Err(error) => player.status = format!("Could not load saved font: {error}"),
                }
            }
            Ok(Box::new(player))
        }),
    )
}
