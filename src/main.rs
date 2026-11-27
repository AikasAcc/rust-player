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
mod file_assoc;
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
mod search;
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
    collections::{HashMap, HashSet},
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
    Name {
        target: Option<usize>,
        name: String,
    },
    Smart {
        target: Option<usize>,
        name: String,
        query: String,
    },
    Delete(usize),
}

#[derive(Clone, Copy)]
enum TrackEdit {
    /// Swap adjacent rows (context menu).
    Move(usize),
    /// Remove a row and insert it before the target (drag and drop).
    Reorder(usize),
}

/// Pure membership evaluation for a saved smart query. Metadata is keyed by
/// `library::path_key`; paths without metadata still match path/file terms.
fn smart_query_members(
    query: &str,
    universe: &[PathBuf],
    metadata: &HashMap<String, TrackInfo>,
) -> Vec<PathBuf> {
    let query = search::Query::compile(query);
    universe
        .iter()
        .filter(|path| {
            let info = metadata
                .get(&library::path_key(path))
                .cloned()
                .unwrap_or_else(|| TrackInfo::read(path));
            query.matches(Some(&info), path)
        })
        .cloned()
        .collect()
}

fn apply_track_edit<T>(items: &mut Vec<T>, index: usize, edit: TrackEdit) {
    match edit {
        TrackEdit::Move(other) if index.max(other) < items.len() => items.swap(index, other),
        TrackEdit::Reorder(target) if index < items.len() => {
            let item = items.remove(index);
            items.insert(target.min(items.len()), item);
        }
        _ => {}
    }
}

/// Enough state to restore one playlist after a track-level edit. The metadata
/// copy is in-memory only; it is rebuilt from the cache when absent.
#[derive(Clone)]
struct PlaylistSnapshot {
    list: usize,
    tracks: Vec<PathBuf>,
    ids: Vec<TrackId>,
    metadata: Vec<TrackInfo>,
    has_view: bool,
    sort: Sort,
    descending: bool,
    favorites: Option<Vec<Option<TrackId>>>,
}

enum PlaylistAction {
    TogglePin,
    Rename,
    EditSmartQuery,
    RefreshSmart,
    MakeNormal,
    Copy,
    Delete,
    ExportAbsolute,
    ExportRelative,
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
    Undo,
    Redo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shortcut {
    PlayPause,
    Next,
    Previous,
    Stop,
    Mute,
    Favorite,
    Shuffle,
    Undo,
    Redo,
}

fn shortcut(modifiers: egui::Modifiers, key: egui::Key) -> Option<Shortcut> {
    let plain = modifiers == egui::Modifiers::default();
    if plain && key == egui::Key::Space {
        Some(Shortcut::PlayPause)
    } else if plain && key == egui::Key::N {
        Some(Shortcut::Next)
    } else if plain && key == egui::Key::P {
        Some(Shortcut::Previous)
    } else if plain && key == egui::Key::S {
        Some(Shortcut::Stop)
    } else if plain && key == egui::Key::M {
        Some(Shortcut::Mute)
    } else if plain && key == egui::Key::F {
        Some(Shortcut::Favorite)
    } else if modifiers == egui::Modifiers::CTRL && key == egui::Key::S {
        Some(Shortcut::Shuffle)
    } else if modifiers == egui::Modifiers::CTRL && key == egui::Key::ArrowRight {
        Some(Shortcut::Next)
    } else if modifiers == egui::Modifiers::CTRL && key == egui::Key::ArrowLeft {
        Some(Shortcut::Previous)
    } else if modifiers == egui::Modifiers::CTRL && key == egui::Key::Z {
        Some(Shortcut::Undo)
    } else if (modifiers == (egui::Modifiers::CTRL | egui::Modifiers::SHIFT) && key == egui::Key::Z)
        || (modifiers == egui::Modifiers::CTRL && key == egui::Key::Y)
    {
        Some(Shortcut::Redo)
    } else {
        None
    }
}

#[cfg(test)]
mod shortcut_tests {
    use super::*;

    #[test]
    fn plain_keys_map_to_transport_actions() {
        let plain = egui::Modifiers::default();
        assert_eq!(shortcut(plain, egui::Key::Space), Some(Shortcut::PlayPause));
        assert_eq!(shortcut(plain, egui::Key::N), Some(Shortcut::Next));
        assert_eq!(shortcut(plain, egui::Key::P), Some(Shortcut::Previous));
        assert_eq!(shortcut(plain, egui::Key::S), Some(Shortcut::Stop));
        assert_eq!(shortcut(plain, egui::Key::M), Some(Shortcut::Mute));
        assert_eq!(shortcut(plain, egui::Key::F), Some(Shortcut::Favorite));
    }

    #[test]
    fn ctrl_modifiers_are_not_treated_as_plain_keys() {
        assert_eq!(
            shortcut(egui::Modifiers::CTRL, egui::Key::S),
            Some(Shortcut::Shuffle)
        );
        assert_eq!(
            shortcut(egui::Modifiers::CTRL, egui::Key::ArrowRight),
            Some(Shortcut::Next)
        );
        assert_eq!(shortcut(egui::Modifiers::CTRL, egui::Key::Space), None);
        assert_eq!(shortcut(egui::Modifiers::SHIFT, egui::Key::N), None);
    }

    #[test]
    fn undo_and_redo_have_distinct_modifiers() {
        assert_eq!(
            shortcut(egui::Modifiers::CTRL, egui::Key::Z),
            Some(Shortcut::Undo)
        );
        assert_eq!(
            shortcut(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::Z),
            Some(Shortcut::Redo)
        );
        assert_eq!(
            shortcut(egui::Modifiers::CTRL, egui::Key::Y),
            Some(Shortcut::Redo)
        );
    }
}

#[cfg(test)]
mod queue_title_tests {
    use super::*;

    fn info(title: &str) -> TrackInfo {
        TrackInfo {
            title: title.into(),
            ..TrackInfo::default()
        }
    }

    #[test]
    fn a_missing_owning_view_does_not_borrow_a_title_from_another_playlist() {
        let views = HashMap::from([(
            1,
            PlaylistView::build(vec![info("Unrelated track")], Sort::Added, false),
        )]);

        assert_eq!(Player::view_title(&views, 0, 0), None);
        assert_eq!(
            Player::view_title(&views, 1, 0).as_deref(),
            Some("Unrelated track")
        );
    }

    #[test]
    fn an_empty_title_is_not_used_for_a_queued_label() {
        let views = HashMap::from([(0, PlaylistView::build(vec![info("")], Sort::Added, false))]);
        assert_eq!(Player::view_title(&views, 0, 0), None);
    }
}

struct ThemeDraft {
    theme: theme::Theme,
    font: Option<PathBuf>,
    font_error: String,
    background: background::Settings,
    themes: Vec<theme::NamedTheme>,
    saved_themes_changed: bool,
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
    source: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum TagAction {
    #[default]
    Keep,
    Set,
    Clear,
}

impl TagAction {
    const ALL: [Self; 3] = [Self::Keep, Self::Set, Self::Clear];

    fn label(self) -> &'static str {
        match self {
            Self::Keep => "Keep",
            Self::Set => "Set",
            Self::Clear => "Clear",
        }
    }
}

#[derive(Default)]
struct TagField {
    action: TagAction,
    value: String,
}

impl TagField {
    fn value(&self) -> Option<String> {
        match self.action {
            TagAction::Keep => None,
            TagAction::Clear => Some(String::new()),
            TagAction::Set => Some(self.value.trim().to_owned()),
        }
    }
}

/// One bulk-edit dialog for a multi-row playlist selection.
#[derive(Default)]
struct TagEditor {
    paths: Vec<PathBuf>,
    title: TagField,
    artist: TagField,
    album: TagField,
    album_artist: TagField,
    genre: TagField,
    year: TagField,
}

struct TagJob {
    total: usize,
    received: usize,
    errors: Vec<String>,
    rx: mpsc::Receiver<Result<(), String>>,
}

const STATUS_VISIBLE: Duration = Duration::from_secs(3);
const STATUS_FADE: Duration = Duration::from_millis(450);

/// Tracks how long the current `Player.status` text has been on screen.
///
/// The status text itself stays a plain `String`, so every existing write site
/// keeps working. One-shot messages (`"Audio output changed. Playback paused."`)
/// disappear after three seconds. Messages ending in an ellipsis are progress
/// reports and stay until a worker replaces them, so a long import does not
/// vanish while it is still running.
#[derive(Debug)]
struct StatusTimer {
    shown: String,
    started: Instant,
    hold: bool,
}

impl StatusTimer {
    fn new() -> Self {
        Self {
            shown: String::new(),
            started: Instant::now(),
            hold: false,
        }
    }

    /// Progress reports and state-bearing errors must not fade away while
    /// they are still true. The signal-path panel reads the audio error text
    /// from `status`, so it has to stay until the sink recovers.
    fn is_held(message: &str) -> bool {
        message.contains('…')
            || message.contains("...")
            || message.starts_with("Audio output:")
            || message.starts_with("Could not reopen the audio output:")
            || message.starts_with("Could not load playlists:")
    }

    /// Restarts the timer unconditionally, used by explicit notices.
    fn arm(&mut self, message: &str) {
        self.shown = message.to_owned();
        self.started = Instant::now();
        self.hold = Self::is_held(message);
    }

    /// Restarts only when the status text changed, used once per UI frame.
    fn observe(&mut self, message: &str) {
        if self.shown != message {
            self.arm(message);
        }
    }

    fn clear(&mut self) {
        self.shown.clear();
        self.started = Instant::now();
        self.hold = false;
    }

    fn alpha_at(&self, now: Instant) -> f32 {
        if self.shown.is_empty() || self.hold {
            return 1.0;
        }
        let elapsed = now.saturating_duration_since(self.started);
        (STATUS_VISIBLE.saturating_sub(elapsed).as_secs_f32() / STATUS_FADE.as_secs_f32())
            .clamp(0.0, 1.0)
    }

    fn expired_at(&self, now: Instant) -> bool {
        !self.shown.is_empty()
            && !self.hold
            && now.saturating_duration_since(self.started) >= STATUS_VISIBLE
    }

    fn repaint_after(&self, now: Instant) -> Option<Duration> {
        if self.shown.is_empty() || self.hold {
            return None;
        }
        let elapsed = now.saturating_duration_since(self.started);
        let fade_start = STATUS_VISIBLE.saturating_sub(STATUS_FADE);
        Some(if elapsed < fade_start {
            fade_start - elapsed
        } else {
            Duration::from_millis(16)
        })
    }
}

#[cfg(test)]
mod status_tests {
    use super::*;

    fn timer(message: &str) -> StatusTimer {
        let mut timer = StatusTimer::new();
        timer.arm(message);
        timer
    }

    #[test]
    fn one_shot_status_fades_and_expires() {
        let timer = timer("Audio output changed. Playback paused.");
        let started = timer.started;
        assert_eq!(timer.alpha_at(started + Duration::from_secs(2)), 1.0);
        let fading = timer.alpha_at(started + Duration::from_millis(2700));
        assert!((0.0..1.0).contains(&fading), "unexpected alpha {fading}");
        assert_eq!(timer.alpha_at(started + STATUS_VISIBLE), 0.0);
        assert!(timer.expired_at(started + STATUS_VISIBLE));
    }

    #[test]
    fn progress_status_stays_until_it_is_replaced() {
        let timer = timer("Importing audio…");
        let late = timer.started + Duration::from_secs(30);
        assert!(timer.hold);
        assert_eq!(timer.alpha_at(late), 1.0);
        assert!(!timer.expired_at(late));
        assert_eq!(timer.repaint_after(late), None);
    }

    #[test]
    fn audio_failure_stays_visible_but_the_pause_notice_fades() {
        let failure = timer("Audio output: device is in use");
        let late = failure.started + Duration::from_secs(30);
        assert!(failure.hold);
        assert!(!failure.expired_at(late));

        let paused = timer("Audio output changed. Playback paused.");
        assert!(!paused.hold);
        assert!(paused.expired_at(paused.started + STATUS_VISIBLE));
    }

    #[test]
    fn repaint_wakes_for_the_fade_window() {
        let timer = timer("Undid playlist edit.");
        assert_eq!(
            timer.repaint_after(timer.started),
            Some(STATUS_VISIBLE - STATUS_FADE)
        );
        assert_eq!(
            timer.repaint_after(timer.started + Duration::from_millis(2600)),
            Some(Duration::from_millis(16))
        );
    }

    #[test]
    fn observing_restarts_only_for_a_new_message() {
        let mut timer = timer("Old message");
        let old = timer.started;
        timer.observe("Old message");
        assert_eq!(timer.started, old);
        timer.observe("Audio output changed. Playback paused.");
        assert!(timer.started >= old);
        assert!(!timer.expired_at(timer.started + Duration::from_secs(2)));
        assert!(!timer.hold);
    }
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
    fn new(list: usize, text: &str, playlist: &Playlist, view: Option<&PlaylistView>) -> Self {
        let query = search::Query::compile(text);
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
                        if query.matches(Some(track), &playlist.tracks[*index]) {
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
            if query.matches(None, path) {
                rows.push(PlaylistRow::Track(index));
                count += 1;
            }
        }
        Self {
            list,
            query: text.into(),
            rows,
            count,
        }
    }
}

#[cfg(test)]
mod search_results_tests {
    use super::*;

    fn playlist() -> Playlist {
        Playlist {
            name: "Test".into(),
            tracks: vec![
                PathBuf::from("Music/A/Help.flac"),
                PathBuf::from("Music/B/Other.flac"),
            ],
            ids: vec![],
            pinned: false,
            sidebar_position: None,
            sort: Sort::Added,
            descending: false,
            smart_query: None,
        }
    }

    #[test]
    fn field_queries_filter_indexed_rows_and_unindexed_paths() {
        let list = playlist();
        let mut info = TrackInfo {
            title: "Help!".into(),
            artist: "The Beatles".into(),
            ..Default::default()
        };
        info.prepare_search(&list.tracks[0]);
        let view = PlaylistView::build(vec![info], Sort::Added, false);

        let by_artist = SearchResults::new(0, "artist:beatles", &list, Some(&view));
        assert_eq!(by_artist.count, 1);
        assert!(matches!(by_artist.rows.as_slice(), [PlaylistRow::Track(0)]));

        let by_file = SearchResults::new(0, "file:other.flac", &list, Some(&view));
        assert_eq!(by_file.count, 1);
        assert!(matches!(by_file.rows.as_slice(), [PlaylistRow::Track(1)]));

        let no_match = SearchResults::new(0, "-path:music", &list, Some(&view));
        assert_eq!(no_match.count, 0);
    }
}

#[cfg(test)]
mod smart_playlist_tests {
    use super::*;
    use std::path::Path;

    fn info(title: &str, artist: &str) -> TrackInfo {
        let mut info = TrackInfo {
            title: title.into(),
            artist: artist.into(),
            ..Default::default()
        };
        info.prepare_search(Path::new("Music/A/Help.flac"));
        info
    }

    #[test]
    fn saved_query_filters_the_library_universe() {
        let universe = vec![
            PathBuf::from("Music/A/Help.flac"),
            PathBuf::from("Music/B/Other.flac"),
        ];
        let metadata = HashMap::from([
            (
                library::path_key(&universe[0]),
                info("Help!", "The Beatles"),
            ),
            (
                library::path_key(&universe[1]),
                info("Other", "Miles Davis"),
            ),
        ]);
        let members = smart_query_members("artist:beatles", &universe, &metadata);
        assert_eq!(members, vec![universe[0].clone()]);
    }

    #[test]
    fn path_terms_work_without_cached_metadata() {
        let universe = vec![PathBuf::from("Music/B/Other.flac")];
        let members = smart_query_members("file:other.flac", &universe, &HashMap::new());
        assert_eq!(members, universe);
    }

    #[test]
    fn old_saved_playlists_default_to_a_normal_playlist() {
        let playlist: Playlist = serde_json::from_str(
            r#"{"name":"Old","tracks":[],"ids":[],"pinned":false,
                "sidebar_position":null,"sort":"Album","descending":false}"#,
        )
        .expect("old playlist loads");
        assert_eq!(playlist.smart_query, None);
    }
}

#[derive(Default)]
struct FacetCache {
    list: usize,
    values: Vec<search::Facet>,
}

#[derive(Default)]
struct Dialogs {
    playlist_dialog: Option<PlaylistDialog>,
    track_properties: Option<TrackProperties>,
    tag_editor: Option<TagEditor>,
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
    tag_job: Option<TagJob>,
    cached_favorite: Option<(TrackId, bool)>,
    search_deadline: Option<Instant>,
    facets: Option<FacetCache>,
    facets_expanded: bool,
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
    status_timer: StatusTimer,
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
    track_selection: HashSet<usize>,
    track_anchor: Option<usize>,
    selection_list: Option<usize>,
    undo: Vec<PlaylistSnapshot>,
    redo: Vec<PlaylistSnapshot>,
    rpc: presence::Presence,
    lastfm: lastfm::Client,
    order: Vec<TrackId>,
    cursor: usize,
    order_list: Option<usize>,
    queue_order_edited: bool,
    play_queue: play_queue::PlayQueue,
    inspector_tab: InspectorTab,
    imports: Vec<(usize, mpsc::Receiver<ImportResult>)>,
    /// Files handed over by Windows on a double-click. Imported and played on
    /// the first frame once the UI is ready.
    startup_files: Vec<PathBuf>,
    play_first_import: bool,
    file_assoc_status: String,
    playlist_imports: Vec<mpsc::Receiver<Vec<PlaylistImportResult>>>,
    playlist_exports: Vec<mpsc::Receiver<PlaylistExportResult>>,
    views: HashMap<usize, PlaylistView>,
    indexing: HashMap<usize, mpsc::Receiver<Vec<TrackInfo>>>,
    /// Opt-in UI frame timing: set RUST_PLAYER_FRAME_LOG to a CSV path.
    frame_log: Option<std::io::BufWriter<std::fs::File>>,
    frame_log_count: usize,
}

/// `prune_favorites()` rewrites the Favorites playlist when a source is
/// deleted, renamed or relinked. Cached rows and a running index job belong to
/// the old list and must be dropped together with it.
fn invalidate_favorites_cache(
    favorites: Option<usize>,
    views: &mut HashMap<usize, PlaylistView>,
    indexing: &mut HashMap<usize, mpsc::Receiver<Vec<TrackInfo>>>,
) {
    if let Some(index) = favorites {
        views.remove(&index);
        indexing.remove(&index);
    }
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
                    let message = format!("Could not load listening activity: {error}");
                    if status.is_empty() {
                        status = message;
                    } else {
                        // Both files can be corrupt; do not hide the more
                        // important playlist error behind the listening one.
                        status = format!("{status} · {message}");
                    }
                    (listening::Listening::default(), true)
                }
            };
        let queue_path = library_path.with_file_name("queue.json");
        let (play_queue, queue_needs_backup) = match play_queue::PlayQueue::load(&queue_path) {
            Ok(mut queue) => {
                queue.retain(|id| library.locations.contains_key(&id));
                (queue, false)
            }
            Err(play_queue::LoadError::Missing) => (play_queue::PlayQueue::default(), false),
            Err(play_queue::LoadError::Invalid(error)) => {
                let message = format!("Could not load queue: {error}");
                if status.is_empty() {
                    status = message;
                } else {
                    status = format!("{status} · {message}");
                }
                (play_queue::PlayQueue::default(), true)
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
        if queue_needs_backup {
            backup.need(queue_path);
        }
        let mut frame_log = std::env::var_os("RUST_PLAYER_FRAME_LOG").and_then(|path| {
            std::fs::File::create(path)
                .ok()
                .map(|file| std::io::BufWriter::with_capacity(64 * 1024, file))
        });
        if let Some(log) = &mut frame_log {
            let _ = writeln!(log, "frame,micros");
        }
        let mut player = Self {
            relink: None,
            gain_jobs: HashMap::new(),
            media_job: None,
            media_generation: 0,
            property_jobs: HashMap::new(),
            tag_job: None,
            cached_favorite: None,
            search_deadline: None,
            facets: None,
            facets_expanded: false,
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
            play_queue,
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
            status_timer: StatusTimer::new(),
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
            track_selection: HashSet::new(),
            track_anchor: None,
            selection_list: None,
            undo: Vec::new(),
            redo: Vec::new(),
            rpc: presence::Presence::new(),
            lastfm,
            order: Vec::new(),
            cursor: usize::MAX,
            imports: Vec::new(),
            startup_files: Vec::new(),
            play_first_import: false,
            file_assoc_status: String::new(),
            playlist_imports: Vec::new(),
            playlist_exports: Vec::new(),
            views: HashMap::new(),
            indexing: HashMap::new(),
            frame_log,
            frame_log_count: 0,
        };
        player.refresh_smart_playlists();
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

    fn invalidate_search(&mut self) {
        self.search_results = None;
        self.facets = None;
    }

    fn save(&mut self) {
        self.cached_favorite = None;
        self.invalidate_search();
        self.volume_save_at = None;
        if self.library.prune_favorites() {
            // Favorites rows may have been removed or repointed to a relinked
            // source. A cached view would still contain the old rows and can
            // make the UI index past the end of the shorter playlist.
            invalidate_favorites_cache(self.library.favorites, &mut self.views, &mut self.indexing);
        }
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
        // listening.json is written by record_play(), which backs it up right
        // before its own enqueue. A failure there must not block settings.
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
        self.save_queue();
    }

    /// Persist only the explicit "PLAY NEXT" entries. The playlist part of the
    /// queue is derived from the current order and is saved with playback.json.
    fn save_queue(&mut self) {
        let path = self.library_path.with_file_name("queue.json");
        let mut notices = Vec::new();
        if !self.prepare_backup(path.clone(), "queue.invalid-", "queue", &mut notices) {
            return;
        }
        match serde_json::to_vec(&self.play_queue) {
            Ok(bytes) => {
                self.save_worker.enqueue(path, bytes);
                if !notices.is_empty() {
                    self.status = notices.join(" ");
                }
            }
            Err(error) => {
                notices.push(format!("Could not save queue: {error}"));
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
        self.status = message.into();
        self.status_timer.arm(&self.status);
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
        let saved: SavedTrack = match serde_json::from_slice(&bytes) {
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
        let saved = self.normalize_saved(saved)?;
        if missing_id {
            self.write_playback(Some(saved.clone()));
        }
        Some(saved)
    }

    /// Refresh identity, coordinates and path of a persisted track together.
    /// Both restore paths use this so a reorder cannot leave them disagreeing.
    fn normalize_saved(&self, mut saved: SavedTrack) -> Option<SavedTrack> {
        let id = saved.resolve(&self.library)?;
        saved.id = Some(id);
        if let Some(track) = self.library.locate(id) {
            saved.track = track;
        }
        if let Some(path) = self.library.track_path(id) {
            saved.path = path.to_path_buf();
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
        } else if let Some(saved) = saved {
            let saved = self.normalize_saved(saved);
            self.write_playback(saved);
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
        self.undo.clear();
        self.redo.clear();
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
            PlaylistAction::EditSmartQuery => {
                let playlist = &self.library.playlists[index];
                self.dialogs.playlist_dialog = Some(PlaylistDialog::Smart {
                    target: Some(index),
                    name: playlist.name.clone(),
                    query: playlist.smart_query.clone().unwrap_or_default(),
                });
            }
            PlaylistAction::RefreshSmart => {
                if self.refresh_smart_playlists() {
                    self.status = "Smart playlists updated".into();
                    self.save();
                } else {
                    self.status = "Smart playlists are already up to date".into();
                }
                self.request_index(index, ctx);
            }
            PlaylistAction::MakeNormal => {
                self.library.playlists[index].smart_query = None;
                self.status = "Smart query removed; the current tracks were kept".into();
                self.save();
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
            PlaylistAction::ExportAbsolute | PlaylistAction::ExportRelative => {
                let playlist = self.library.playlists[index].clone();
                let name = playlist
                    .name
                    .replace(['<', '>', ':', '"', '/', '\\', '|', '?', '*'], "_");
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("M3U8 playlist", &["m3u8"])
                    .set_file_name(format!("{name}.m3u8"))
                    .save_file()
                {
                    let mode = if matches!(action, PlaylistAction::ExportRelative) {
                        playlist_io::PathMode::Relative
                    } else {
                        playlist_io::PathMode::Absolute
                    };
                    let (send, receive) = mpsc::channel();
                    self.playlist_exports.push(receive);
                    let label = playlist.name.clone();
                    let ctx = ctx.clone();
                    std::thread::spawn(move || {
                        let result = playlist_io::export(&playlist, &path, mode)
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
                        imported += 1;
                        skipped += ignored;
                    }
                    Err(error) => errors.push(error),
                }
            }
            // Importing N playlists used to rebuild the whole location/ID map N
            // times. New playlists have no IDs yet, so one pass is equivalent.
            if imported > 0 {
                self.library.rebuild_locations();
                self.library.selected = self.library.playlists.len() - 1;
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

    fn snapshot_playlist(&self, list: usize) -> PlaylistSnapshot {
        let playlist = &self.library.playlists[list];
        let view = self.views.get(&list);
        PlaylistSnapshot {
            list,
            tracks: playlist.tracks.clone(),
            ids: playlist.ids.clone(),
            metadata: view.map(|view| view.tracks.clone()).unwrap_or_default(),
            has_view: view.is_some(),
            sort: playlist.sort,
            descending: playlist.descending,
            favorites: (Some(list) == self.library.favorites)
                .then(|| self.library.favorite_ids.clone()),
        }
    }

    fn record_edit(&mut self, list: usize) {
        self.undo.push(self.snapshot_playlist(list));
        if self.undo.len() > 64 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn apply_playlist_snapshot(&mut self, snapshot: PlaylistSnapshot) {
        let saved = self.previously_saved();
        {
            let playlist = &mut self.library.playlists[snapshot.list];
            playlist.tracks = snapshot.tracks;
            playlist.ids = snapshot.ids;
            playlist.sort = snapshot.sort;
            playlist.descending = snapshot.descending;
        }
        if let Some(favorites) = snapshot.favorites {
            self.library.favorite_ids = favorites;
        }
        self.indexing.remove(&snapshot.list);
        self.views.remove(&snapshot.list);
        if snapshot.has_view {
            self.views.insert(
                snapshot.list,
                PlaylistView::build(snapshot.metadata, snapshot.sort, snapshot.descending),
            );
        }
        self.cached_favorite = None;
        self.invalidate_search();
        self.library.rebuild_locations();
        self.save();
        self.refresh_playback_ids(saved);
        self.clear_track_selection();
    }

    fn undo_edit(&mut self) {
        let Some(snapshot) = self.undo.pop() else {
            self.status = "Nothing to undo.".into();
            return;
        };
        self.redo.push(self.snapshot_playlist(snapshot.list));
        self.apply_playlist_snapshot(snapshot);
        self.status = "Undid playlist edit.".into();
    }

    fn redo_edit(&mut self) {
        let Some(snapshot) = self.redo.pop() else {
            self.status = "Nothing to redo.".into();
            return;
        };
        self.undo.push(self.snapshot_playlist(snapshot.list));
        self.apply_playlist_snapshot(snapshot);
        self.status = "Redid playlist edit.".into();
    }

    fn edit_track(&mut self, (list, index): Track, edit: TrackEdit) {
        let len = self.library.playlists[list].tracks.len();
        let changed = match edit {
            TrackEdit::Move(other) => index.max(other) < len,
            TrackEdit::Reorder(target) => index < len && target != index,
        };
        if changed {
            self.record_edit(list);
        }
        let saved = self.previously_saved();
        let playlist = &mut self.library.playlists[list];
        apply_track_edit(&mut playlist.ids, index, edit);
        apply_track_edit(&mut playlist.tracks, index, edit);
        if Some(list) == self.library.favorites {
            apply_track_edit(&mut self.library.favorite_ids, index, edit);
        }
        self.indexing.remove(&list);
        if let Some(view) = self.views.remove(&list) {
            let mut metadata = view.tracks;
            apply_track_edit(&mut metadata, index, edit);
            let playlist = &self.library.playlists[list];
            self.views.insert(
                list,
                PlaylistView::build(metadata, playlist.sort, playlist.descending),
            );
        }
        self.library.rebuild_locations();
        self.save();
        self.refresh_playback_ids(saved);
        if matches!(edit, TrackEdit::Move(_) | TrackEdit::Reorder(_))
            && !self.library.shuffle
            && self.order_list == Some(list)
            && let Some(id) = self.order_id()
        {
            self.plan(id);
        }
    }

    fn enqueue_tracks(&mut self, list: usize, indices: &[usize]) {
        let mut added = 0;
        for &index in indices {
            let Some(id) = self.library.track_id((list, index)) else {
                continue;
            };
            let title = self
                .views
                .get(&list)
                .and_then(|view| view.tracks.get(index))
                .map(|track| track.title.clone())
                .filter(|title| !title.is_empty())
                .unwrap_or_else(|| file_title(&self.library.playlists[list].tracks[index]));
            self.play_queue.push(id, title);
            added += 1;
        }
        if added > 0 {
            self.save_queue();
            self.status = if added == 1 {
                "Added 1 track to the queue.".into()
            } else {
                format!("Added {added} tracks to the queue.")
            };
        }
    }

    fn remove_tracks(&mut self, list: usize, indices: &[usize]) {
        let total = self.library.playlists[list].tracks.len();
        let mut unique: Vec<_> = indices
            .iter()
            .copied()
            .filter(|index| *index < total)
            .collect();
        unique.sort_unstable();
        unique.dedup();
        if unique.is_empty() {
            return;
        }
        self.record_edit(list);
        let saved = self.previously_saved();
        let favorites = Some(list) == self.library.favorites;
        {
            let playlist = &mut self.library.playlists[list];
            for &index in unique.iter().rev() {
                if index < playlist.ids.len() {
                    playlist.ids.remove(index);
                }
                if index < playlist.tracks.len() {
                    playlist.tracks.remove(index);
                }
            }
        }
        if favorites {
            for &index in unique.iter().rev() {
                if index < self.library.favorite_ids.len() {
                    self.library.favorite_ids.remove(index);
                }
            }
        }
        self.indexing.remove(&list);
        if let Some(view) = self.views.remove(&list) {
            let mut metadata = view.tracks;
            for &index in unique.iter().rev() {
                if index < metadata.len() {
                    metadata.remove(index);
                }
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
        self.status = if unique.len() == 1 {
            "Removed 1 track.".into()
        } else {
            format!("Removed {} tracks.", unique.len())
        };
    }

    fn clear_track_selection(&mut self) {
        self.track_selection.clear();
        self.track_anchor = None;
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

    fn finish_imports(&mut self, ctx: &egui::Context) {
        let mut index = 0;
        while index < self.imports.len() {
            let (list, rx) = &self.imports[index];
            let list = *list;
            match rx.try_recv() {
                Ok((tracks, skipped)) => {
                    self.imports.remove(index);
                    let play = std::mem::take(&mut self.play_first_import);
                    let count = tracks.len();
                    let playlist = &mut self.library.playlists[list];
                    let first_new = playlist.tracks.len();
                    playlist.tracks.extend(tracks);
                    self.library.rebuild_locations();
                    let first_id = if play {
                        self.library.playlists[list].ids.get(first_new).copied()
                    } else {
                        None
                    };
                    self.refresh_smart_playlists();
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
                    if let Some(id) = first_id {
                        self.library.selected = list;
                        self.start(id, ctx);
                    }
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
        self.invalidate_search();
        self.thumbs.invalidate_all();
    }

    /// Rebuild smart playlists from the ordinary library playlists. Membership
    /// is materialized so sorting, playback order and stable IDs keep working;
    /// only the query editor changes the list contents.
    fn refresh_smart_playlists(&mut self) -> bool {
        let mut metadata: HashMap<String, TrackInfo> = HashMap::new();
        let mut universe: Vec<PathBuf> = Vec::new();
        let mut seen = HashSet::new();
        for (list, playlist) in self.library.playlists.iter().enumerate() {
            if playlist.smart_query.is_some() {
                continue;
            }
            let view = self.views.get(&list);
            for (index, path) in playlist.tracks.iter().enumerate() {
                let key = library::path_key(path);
                if seen.insert(key.clone()) {
                    universe.push(path.clone());
                }
                let info = view
                    .and_then(|view| view.tracks.get(index))
                    .cloned()
                    .unwrap_or_else(|| TrackInfo::read(path));
                metadata.insert(key, info);
            }
        }
        let cache_file = self.library_path.with_file_name("metadata-cache.json");
        let cached = metadata_cache::cached_infos(&cache_file, &universe);
        for path in &universe {
            let key = library::path_key(path);
            if !metadata.contains_key(&key)
                && let Some(info) = cached.get(path)
            {
                metadata.insert(key, info.clone());
            }
        }
        let mut updates: Vec<(usize, Vec<PathBuf>, Vec<TrackId>)> = Vec::new();
        for (list, playlist) in self.library.playlists.iter().enumerate() {
            let Some(query_text) = &playlist.smart_query else {
                continue;
            };
            let mut old_ids = HashMap::new();
            for (path, id) in playlist.tracks.iter().zip(playlist.ids.iter()) {
                old_ids.insert(library::path_key(path), *id);
            }
            let tracks = smart_query_members(query_text, &universe, &metadata);
            let mut ids = Vec::new();
            for path in &tracks {
                // Zero is replaced by `rebuild_locations` with a fresh,
                // collision-free identity.
                ids.push(
                    old_ids
                        .get(&library::path_key(path))
                        .copied()
                        .unwrap_or(TrackId(0)),
                );
            }
            if tracks != playlist.tracks || ids != playlist.ids {
                updates.push((list, tracks, ids));
            }
        }
        let changed = !updates.is_empty();
        for (list, tracks, ids) in updates {
            let playlist = &mut self.library.playlists[list];
            playlist.tracks = tracks;
            playlist.ids = ids;
        }
        if changed {
            self.library.rebuild_locations();
            for list in 0..self.library.playlists.len() {
                self.views.remove(&list);
                self.indexing.remove(&list);
            }
            self.invalidate_search();
            self.track_selection.clear();
        }
        changed
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

    /// Title from the owning playlist view only. A different playlist can have
    /// a row at the same index, but that row is a different track; borrowing its
    /// title would silently relabel the queued entry.
    fn view_title(
        views: &HashMap<usize, PlaylistView>,
        list: usize,
        track: usize,
    ) -> Option<String> {
        views
            .get(&list)
            .and_then(|view| view.tracks.get(track))
            .filter(|info| !info.title.is_empty())
            .map(|info| info.title.clone())
    }

    /// Best-known title for a stable ID from indexed playlist views. Mirrors
    /// the lookup used by the queue panel so saved labels stay identical.
    fn queued_title(&self, id: TrackId) -> Option<String> {
        let (list, track) = self.library.locate(id)?;
        Self::view_title(&self.views, list, track)
    }

    /// Queued titles are snapshots taken when a row was added. Re-read them
    /// once fresh metadata arrives, e.g. after a tag edit or ReplayGain scan.
    fn refresh_queue_titles(&mut self) {
        let titles: HashMap<TrackId, String> = self
            .play_queue
            .queued_tracks()
            .into_iter()
            .filter_map(|id| self.queued_title(id).map(|title| (id, title)))
            .collect();
        if self.play_queue.refresh_titles(&titles) {
            self.save_queue();
        }
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
            self.invalidate_search();
            // Fresh metadata can change which tracks a saved query matches.
            self.refresh_smart_playlists();
            self.refresh_queue_titles();
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
            match lyrics::overwrite(&track.with_extension(extension), &playing.media.lyrics) {
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

    fn open_tag_editor(&mut self, paths: Vec<PathBuf>) {
        self.dialogs.tag_editor = Some(TagEditor {
            paths,
            ..TagEditor::default()
        });
    }

    /// Hand a bulk edit to a worker thread so hundreds of files never block
    /// the UI. Each field is independently Keep / Set / Clear.
    fn start_tag_job(&mut self, editor: TagEditor, ctx: &egui::Context) {
        let values = (
            editor.title.value(),
            editor.artist.value(),
            editor.album.value(),
            editor.album_artist.value(),
            editor.genre.value(),
            editor.year.value(),
        );
        let paths = editor.paths;
        if paths.is_empty() || values == (None, None, None, None, None, None) {
            self.status = "No tag fields selected".into();
            return;
        }
        let total = paths.len();
        let (tx, rx) = mpsc::channel();
        let paint = ctx.clone();
        std::thread::spawn(move || {
            for path in &paths {
                let result = tagging::write_batch(
                    path,
                    tagging::BatchEdit {
                        title: values.0.as_deref(),
                        artist: values.1.as_deref(),
                        album: values.2.as_deref(),
                        album_artist: values.3.as_deref(),
                        genre: values.4.as_deref(),
                        year: values.5.as_deref(),
                    },
                );
                let _ = tx.send(result);
            }
            paint.request_repaint();
        });
        self.tag_job = Some(TagJob {
            total,
            received: 0,
            errors: Vec::new(),
            rx,
        });
        self.status = format!("Writing tags for {total} files…");
    }

    fn finish_tag_jobs(&mut self, ctx: &egui::Context) {
        let mut done = false;
        if let Some(job) = &mut self.tag_job {
            loop {
                match job.rx.try_recv() {
                    Ok(Ok(())) => job.received += 1,
                    Ok(Err(error)) => {
                        job.received += 1;
                        if !job.errors.contains(&error) {
                            job.errors.push(error);
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        done = true;
                        break;
                    }
                }
            }
            done |= job.received >= job.total;
            if done {
                let ok = job.received.saturating_sub(job.errors.len());
                let errors = job.errors.clone();
                self.status = if errors.is_empty() {
                    format!("Tagged {ok} files")
                } else {
                    format!(
                        "Tagged {ok} files · {} failed: {}",
                        errors.len(),
                        errors.first().map_or("", String::as_str)
                    )
                };
            } else {
                self.status = format!("Writing tags… {}/{}", job.received, job.total);
            }
        }
        if done {
            self.tag_job = None;
            self.refresh_metadata(ctx);
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
        // Manual Next (button, taskbar, media key) must honour Repeat::All just
        // like the automatic transition does.
        let mut candidate = gapless_ui::step_cursor(
            self.library.repeat,
            self.adjacent_cursor(offset),
            self.order.len(),
            offset,
        );
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

    /// foobar-style global keys. Skipped while a widget has keyboard focus so
    /// typing in the search box or a settings field never changes playback.
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.memory(|memory| memory.focused().is_some()) {
            return;
        }
        let candidates = [
            (egui::Modifiers::default(), egui::Key::Space),
            (egui::Modifiers::default(), egui::Key::N),
            (egui::Modifiers::default(), egui::Key::P),
            (egui::Modifiers::default(), egui::Key::S),
            (egui::Modifiers::default(), egui::Key::M),
            (egui::Modifiers::default(), egui::Key::F),
            (egui::Modifiers::CTRL, egui::Key::S),
            (egui::Modifiers::CTRL, egui::Key::ArrowRight),
            (egui::Modifiers::CTRL, egui::Key::ArrowLeft),
            (egui::Modifiers::CTRL, egui::Key::Z),
            (egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::Z),
            (egui::Modifiers::CTRL, egui::Key::Y),
        ];
        let mut actions = Vec::new();
        ctx.input_mut(|input| {
            for (modifiers, key) in candidates {
                if input.consume_key(modifiers, key)
                    && let Some(action) = shortcut(modifiers, key)
                    && !actions.contains(&action)
                {
                    actions.push(action);
                }
            }
        });
        for action in actions {
            match action {
                Shortcut::PlayPause => {
                    if self
                        .playing
                        .as_ref()
                        .is_some_and(|playing| !playing.sink.is_paused())
                    {
                        self.pause();
                    } else {
                        self.resume(ctx);
                    }
                }
                Shortcut::Next => self.step(1, ctx),
                Shortcut::Previous => self.previous(ctx),
                Shortcut::Stop => self.stop(),
                Shortcut::Mute => self.toggle_mute(),
                Shortcut::Favorite => self.toggle_favorite(),
                Shortcut::Shuffle => self.toggle_shuffle(),
                Shortcut::Undo => self.undo_edit(),
                Shortcut::Redo => self.redo_edit(),
            }
        }
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
            next: !self.play_queue.is_empty() || (loaded && self.next_cursor().is_some()),
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
        self.status_timer.observe(&self.status);
        let now = Instant::now();
        if self.status_timer.expired_at(now) {
            self.status.clear();
            self.status_timer.clear();
        } else if let Some(delay) = self.status_timer.repaint_after(now) {
            ctx.request_repaint_after(delay);
        }
        if !self.startup_files.is_empty() {
            // A file opened from Explorer takes precedence over a restored
            // session; otherwise the old song would start before the new one.
            self.restore_track = false;
            self.play_first_import = true;
            let files = std::mem::take(&mut self.startup_files);
            self.import(files, ctx);
        }
        if std::mem::take(&mut self.restore_track) && self.library.restore_last_track {
            self.restore_playback(ctx);
        }
        self.update_audio(ctx);
        self.run_transport_commands(ctx);
        self.handle_shortcuts(ctx);
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
        self.finish_imports(ctx);
        self.finish_playlist_io();
        self.finish_property_jobs();
        self.finish_tag_jobs(ctx);
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
                    let fade = self.status_timer.alpha_at(Instant::now());
                    ui.colored_label(
                        ui.visuals().weak_text_color().gamma_multiply(fade),
                        &self.status,
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
        // Drain and join: graceful exit must never lose a queued snapshot or
        // the outcome of its final write. The window is gone, so persist any
        // failure to the diagnostic log instead of showing a status message.
        let (reports, panicked) = self.save_worker.finish();
        if panicked {
            crate::diagnostics::append_shutdown("Save worker stopped unexpectedly.");
        }
        for report in reports {
            if let Some(error) = report.error {
                crate::diagnostics::append_shutdown(&error);
            }
        }
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
mod track_edit_tests {
    use super::{TrackEdit, apply_track_edit};

    #[test]
    fn reorder_moves_a_row_before_the_target() {
        let mut items = vec!["A", "B", "C", "D"];
        // A onto C: the UI compensates to target index 1.
        apply_track_edit(&mut items, 0, TrackEdit::Reorder(1));
        assert_eq!(items, ["B", "A", "C", "D"]);
        apply_track_edit(&mut items, 3, TrackEdit::Reorder(0));
        assert_eq!(items, ["D", "B", "A", "C"]);
    }

    #[test]
    fn reorder_ignores_stale_indices() {
        let mut items = vec!["A", "B"];
        apply_track_edit(&mut items, 5, TrackEdit::Reorder(0));
        apply_track_edit(&mut items, 0, TrackEdit::Reorder(9));
        assert_eq!(items, ["B", "A"]);
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

#[derive(Clone, Copy, Default)]
pub(crate) struct RowFlags {
    pub active: bool,
    pub selected: bool,
}

fn playlist_columns(
    ui: &mut egui::Ui,
    height: f32,
    text: [&str; 4],
    header: bool,
    flags: RowFlags,
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
    if flags.active || flags.selected || response.hovered() && !header {
        let fill = if flags.selected && !flags.active {
            ui.visuals().widgets.inactive.weak_bg_fill
        } else {
            ui.visuals().selection.bg_fill
        };
        painter.rect_filled(rect, 0.0, fill);
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
        } else if flags.active {
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
            player.startup_files = file_assoc::startup_paths();
            match audio::DeviceMonitor::new(&creation.egui_ctx) {
                Ok(monitor) => player.audio_devices = Some(monitor),
                Err(error) => player.status = format!("Device monitor unavailable: {error}"),
            }
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
