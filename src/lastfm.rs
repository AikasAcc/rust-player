mod queue;

use crate::{
    Player,
    library::{Library, Media},
};
use eframe::egui;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering::Relaxed},
        mpsc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use symphonia::core::{checksum::Md5, io::Monitor};
use windows::{
    Win32::{
        Foundation::ERROR_NOT_FOUND,
        Security::Credentials::{
            CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree,
            CredReadW, CredWriteW,
        },
    },
    core::{PCWSTR, PWSTR, w},
};

const API: &str = "https://ws.audioscrobbler.com/2.0/";
const AUTHORIZE: &str = "https://www.last.fm/api/auth/";
const CREDENTIAL: PCWSTR = w!("Rust Player/Last.fm");
const RETRY: Duration = Duration::from_secs(30);

#[derive(Default)]
pub struct Listen {
    previous: Duration,
    heard: Duration,
    began: Option<u64>,
    submitted: bool,
    notified: bool,
}

impl Listen {
    pub fn rebase(&mut self, listened: Duration) {
        self.previous = listened;
    }

    fn add(&mut self, listened: Duration, limit: Duration, now: SystemTime) -> Option<u64> {
        let delta = listened.saturating_sub(self.previous);
        self.rebase(listened);
        if delta.is_zero() || self.submitted {
            return None;
        }
        let began = self.began.get_or_insert_with(|| {
            now.duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .saturating_sub(delta)
                .as_secs()
        });
        self.heard += delta;
        self.submitted = self.heard >= limit;
        self.submitted.then_some(*began)
    }
}

fn threshold(duration: Duration) -> Option<Duration> {
    (duration > Duration::from_secs(30)).then(|| (duration / 2).min(Duration::from_secs(240)))
}

#[derive(Serialize, Deserialize)]
struct Track {
    title: String,
    artist: String,
    album: String,
    album_artist: String,
    recording_id: String,
    track_number: Option<u32>,
    duration: Option<Duration>,
}

impl Track {
    fn from_media(media: &Media, duration: Option<Duration>) -> Self {
        Self {
            title: media.title.trim().into(),
            artist: media.artist.trim().into(),
            album: media.album.trim().into(),
            album_artist: media.album_artist.trim().into(),
            recording_id: media.recording_id.trim().into(),
            track_number: media.track_number,
            duration,
        }
    }

    fn fields(&self, account: &Account) -> BTreeMap<&'static str, String> {
        let mut fields = BTreeMap::from([
            ("sk", account.session.key.clone()),
            ("track", self.title.clone()),
            ("artist", self.artist.clone()),
        ]);
        if !self.album.is_empty() {
            fields.insert("album", self.album.clone());
        }
        if !self.album_artist.is_empty() && self.album_artist != self.artist {
            fields.insert("albumArtist", self.album_artist.clone());
        }
        if valid_mbid(&self.recording_id) {
            fields.insert("mbid", self.recording_id.clone());
        }
        if let Some(number) = self.track_number.filter(|&number| number > 0) {
            fields.insert("trackNumber", number.to_string());
        }
        if let Some(duration) = self.duration {
            fields.insert("duration", duration.as_secs().to_string());
        }
        fields
    }
}

fn valid_mbid(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

#[derive(Serialize, Deserialize)]
struct Play {
    track: Track,
    timestamp: u64,
}

#[derive(Serialize, Deserialize)]
struct Keys {
    key: String,
    secret: String,
}

#[derive(Serialize, Deserialize)]
struct Session {
    name: String,
    key: String,
}

#[derive(Serialize, Deserialize)]
struct Account {
    keys: Keys,
    session: Session,
}

enum Command {
    Begin(Keys),
    Finish,
    Cancel,
    Disconnect,
    Wake,
    NowPlaying {
        epoch: u64,
        track_id: u64,
        expires: Instant,
        track: Track,
    },
    Scrobble(u64, Play),
}

enum Event {
    Authorization(url::Url),
    Connected(String),
    Disconnected,
    AuthError(String, bool),
    Submission(String),
    Pending(usize),
}

#[derive(Default)]
struct Sharing {
    epoch: AtomicU64,
    track_id: AtomicU64,
    enabled: AtomicBool,
    now_playing: AtomicBool,
}

pub struct Client {
    tx: mpsc::Sender<Command>,
    rx: mpsc::Receiver<Event>,
    sharing: Arc<Sharing>,
    name: Option<String>,
    waiting: bool,
    busy: bool,
    status: String,
    key: String,
    secret: String,
    pending: usize,
}

impl Client {
    pub fn new(ctx: &egui::Context, library_path: &Path, library: &Library) -> Self {
        let (tx, commands) = mpsc::channel();
        let (events, rx) = mpsc::channel();
        let (account, status) = match read_account() {
            Ok(account) => (account, String::new()),
            Err(error) => (None, error),
        };
        let name = account.as_ref().map(|account| account.session.name.clone());
        let sharing = Arc::new(Sharing {
            enabled: AtomicBool::new(library.lastfm_enabled),
            now_playing: AtomicBool::new(library.lastfm_now_playing),
            ..Sharing::default()
        });
        let worker_sharing = Arc::clone(&sharing);
        let worker_ctx = ctx.clone();
        let queue_path = library_path.with_file_name("lastfm-queue.json");
        std::thread::spawn(move || {
            worker(
                commands,
                events,
                worker_sharing,
                account,
                worker_ctx,
                queue_path,
            )
        });
        Self {
            tx,
            rx,
            sharing,
            name,
            waiting: false,
            busy: false,
            status,
            key: String::new(),
            secret: String::new(),
            pending: 0,
        }
    }
    pub fn poll(&mut self, ctx: &egui::Context) {
        for event in self.rx.try_iter() {
            match event {
                Event::Authorization(url) => {
                    self.busy = false;
                    self.waiting = true;
                    self.secret.clear();
                    self.status =
                        "Authorize Rust Player in your browser, then finish connecting.".into();
                    ctx.open_url(egui::OpenUrl::new_tab(url.as_str()));
                }
                Event::Connected(name) => {
                    self.name = Some(name);
                    self.waiting = false;
                    self.busy = false;
                    self.key.clear();
                    self.status.clear();
                }
                Event::Disconnected => {
                    self.name = None;
                    self.waiting = false;
                    self.busy = false;
                    self.status.clear();
                }
                Event::AuthError(message, waiting) => {
                    self.status = message;
                    self.waiting = waiting;
                    self.busy = false;
                }
                Event::Submission(message) => self.status = message,
                Event::Pending(count) => self.pending = count,
            }
        }
    }

    pub fn track_changed(&self) {
        self.sharing.track_id.fetch_add(1, Relaxed);
    }

    fn send(&mut self, command: Command) {
        if self.tx.send(command).is_err() {
            self.status = "The Last.fm background worker is unavailable.".into();
            self.busy = false;
        }
    }

    pub fn observe(
        &mut self,
        listen: &mut Listen,
        media: &Media,
        duration: Option<Duration>,
        listened: Duration,
        playing: bool,
    ) {
        if !self.sharing.enabled.load(Relaxed)
            || self.name.is_none()
            || self.busy
            || !media.tagged_title
            || media.artist.trim().is_empty()
        {
            listen.heard = Duration::ZERO;
            listen.began = None;
            listen.rebase(listened);
            return;
        }
        let epoch = self.sharing.epoch.load(Relaxed);
        if playing
            && !listen.notified
            && listened > listen.previous
            && self.sharing.now_playing.load(Relaxed)
        {
            listen.notified = true;
            self.send(Command::NowPlaying {
                epoch,
                track_id: self.sharing.track_id.load(Relaxed),
                expires: Instant::now() + RETRY,
                track: Track::from_media(media, duration),
            });
        }
        if let Some(duration) = duration
            && let Some(limit) = threshold(duration)
            && let Some(timestamp) = listen.add(listened, limit, SystemTime::now())
        {
            self.send(Command::Scrobble(
                epoch,
                Play {
                    track: Track::from_media(media, Some(duration)),
                    timestamp,
                },
            ));
        }
    }

    fn settings(&mut self, ui: &mut egui::Ui, library: &mut Library) -> bool {
        ui.heading("Last.fm");
        let changed = ui
            .checkbox(&mut library.lastfm_enabled, "Scrobble to Last.fm")
            .changed();
        if changed {
            self.sharing.enabled.store(library.lastfm_enabled, Relaxed);
            self.sharing.epoch.fetch_add(1, Relaxed);
            self.send(Command::Wake);
        }
        let now_playing_changed = ui
            .add_enabled_ui(library.lastfm_enabled, |ui| {
                ui.checkbox(
                    &mut library.lastfm_now_playing,
                    "Send Now Playing notifications",
                )
                .changed()
            })
            .inner;
        if now_playing_changed {
            self.sharing
                .now_playing
                .store(library.lastfm_now_playing, Relaxed);
        }
        ui.weak(
            "Sends track tags and playback time. Pending scrobbles are saved locally for retry.",
        );
        ui.add_enabled_ui(!self.busy, |ui| {
            if let Some(name) = &self.name {
                ui.label(format!("Connected as {name}"));
                if ui
                    .button("Disconnect")
                    .on_hover_text("Forget this connection and clear pending scrobbles.")
                    .clicked()
                {
                    self.sharing.epoch.fetch_add(1, Relaxed);
                    self.busy = true;
                    self.send(Command::Disconnect);
                }
                if self.pending > 0 {
                    ui.horizontal(|ui| {
                        ui.weak(format!("Pending scrobbles: {}", self.pending));
                        if ui
                            .add_enabled(library.lastfm_enabled, egui::Button::new("Retry now"))
                            .clicked()
                        {
                            self.send(Command::Wake);
                        }
                    });
                }
            } else if self.waiting {
                ui.horizontal(|ui| {
                    if ui.button("Finish connection").clicked() {
                        self.busy = true;
                        self.send(Command::Finish);
                    }
                    if ui.button("Cancel connection").clicked() {
                        self.busy = true;
                        self.send(Command::Cancel);
                    }
                });
            } else {
                ui.hyperlink_to(
                    "Create a Last.fm API account",
                    "https://www.last.fm/api/account/create",
                );
                ui.label("API key");
                ui.add(
                    egui::TextEdit::singleline(&mut self.key)
                        .password(true)
                        .char_limit(64),
                );
                ui.label("Shared secret");
                ui.add(
                    egui::TextEdit::singleline(&mut self.secret)
                        .password(true)
                        .char_limit(64),
                );
                let key = self.key.trim();
                let secret = self.secret.trim();
                let valid = [key, secret].iter().all(|value| {
                    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
                });
                if ui
                    .add_enabled(valid, egui::Button::new("Connect to Last.fm"))
                    .clicked()
                {
                    self.busy = true;
                    self.status = "Contacting Last.fm…".into();
                    self.send(Command::Begin(Keys {
                        key: key.into(),
                        secret: secret.into(),
                    }));
                }
            }
        });
        if !self.status.is_empty() {
            ui.weak(&self.status);
        }
        changed || now_playing_changed
    }
}

impl Player {
    pub(super) fn observe_lastfm(&mut self) {
        if let Some(playing) = &mut self.playing {
            self.lastfm.observe(
                &mut playing.lastfm,
                &playing.media,
                playing.duration,
                playing.sink.listened(),
                !playing.sink.is_paused(),
            );
        }
    }

    pub(super) fn lastfm_settings(&mut self, ui: &mut egui::Ui) {
        let was_enabled = self.library.lastfm_enabled;
        if self.lastfm.settings(ui, &mut self.library) {
            if was_enabled != self.library.lastfm_enabled
                && let Some(playing) = &mut self.playing
            {
                playing.lastfm.heard = Duration::ZERO;
                playing.lastfm.began = None;
                playing.lastfm.rebase(playing.sink.listened());
            }
            self.save();
        }
    }
}

struct ApiError {
    code: Option<u64>,
    status: String,
    retry: bool,
}

impl ApiError {
    fn local(status: &str) -> Self {
        Self {
            code: None,
            status: status.into(),
            retry: false,
        }
    }

    fn lastfm(code: u64) -> Self {
        let status = match code {
            9 => "The Last.fm session has expired. Disconnect and connect again.".into(),
            10 | 13 => "Last.fm rejected the API key or shared secret.".into(),
            14 => "Authorize Rust Player in the browser before finishing the connection.".into(),
            15 => "The authorization token has expired. Start the connection again.".into(),
            _ => format!("Last.fm error {code}."),
        };
        Self {
            code: Some(code),
            status,
            retry: matches!(code, 11 | 16 | 29),
        }
    }
}

fn signature(params: &BTreeMap<&str, String>, secret: &str) -> String {
    let mut hash = Md5::default();
    for (name, value) in params {
        hash.process_buf_bytes(name.as_bytes());
        hash.process_buf_bytes(value.as_bytes());
    }
    hash.process_buf_bytes(secret.as_bytes());
    hash.md5()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn call(
    agent: &ureq::Agent,
    keys: &Keys,
    method: &str,
    mut params: BTreeMap<&str, String>,
    post: bool,
) -> Result<Value, ApiError> {
    params.insert("method", method.into());
    params.insert("api_key", keys.key.clone());
    params.insert("api_sig", signature(&params, &keys.secret));
    params.insert("format", "json".into());
    let encoded = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(params)
        .finish();
    let mut response = if post {
        agent
            .post(API)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .send(encoded.as_str())
    } else {
        agent.get(format!("{API}?{encoded}")).call()
    }
    .map_err(|_| ApiError {
        code: None,
        status: "Could not reach Last.fm.".into(),
        retry: true,
    })?;
    let status = response.status();
    let payload = response
        .body_mut()
        .with_config()
        .limit(65536)
        .read_json::<Value>();
    if let Ok(payload) = &payload
        && let Some(code) = payload["error"].as_u64()
    {
        return Err(ApiError::lastfm(code));
    }
    if !status.is_success() {
        return Err(ApiError {
            code: None,
            status: format!("Last.fm returned HTTP {}.", status.as_u16()),
            retry: status.is_server_error() || status.as_u16() == 429,
        });
    }
    payload.map_err(|_| ApiError::local("Invalid Last.fm response."))
}

fn submit(agent: &ureq::Agent, account: &Account, play: &Play) -> Result<(), ApiError> {
    let mut fields = play.track.fields(account);
    fields.insert("timestamp", play.timestamp.to_string());
    let result = call(agent, &account.keys, "track.scrobble", fields, true)?;
    if result["scrobbles"]["@attr"]["accepted"].as_u64() == Some(1)
        || result["scrobbles"]["@attr"]["accepted"].as_str() == Some("1")
    {
        Ok(())
    } else {
        Err(ApiError::local("Last.fm did not accept this scrobble."))
    }
}

fn worker(
    commands: mpsc::Receiver<Command>,
    events: mpsc::Sender<Event>,
    sharing: Arc<Sharing>,
    mut account: Option<Account>,
    ctx: egui::Context,
    queue_path: PathBuf,
) {
    let agent = ureq::Agent::config_builder()
        .https_only(true)
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(8)))
        .tls_config(crate::artwork::https_agent().config().tls_config().clone())
        .build()
        .new_agent();
    let mut authorization: Option<(Keys, String)> = None;
    let mut retry_at = Instant::now();
    let notify = |event| {
        if events.send(event).is_ok() {
            ctx.request_repaint_of(egui::ViewportId::ROOT);
        }
    };
    let load = |owner: &str| {
        let queue = queue::Queue::read(&queue_path, owner).unwrap_or_else(|_| {
            notify(Event::Submission(
                "Could not read the Last.fm scrobble queue.".into(),
            ));
            queue::Queue::empty(owner)
        });
        notify(Event::Pending(queue.items.len()));
        queue
    };
    let persist = |queue: &queue::Queue| {
        notify(Event::Pending(queue.items.len()));
        if queue.save(&queue_path).is_err() {
            notify(Event::Submission(
                "Could not save the Last.fm scrobble queue.".into(),
            ));
        }
    };
    let mut queue = account.as_ref().map(|account| load(&account.session.name));
    let mut invalid_session = false;
    loop {
        let can_retry = sharing.enabled.load(Relaxed)
            && !invalid_session
            && queue.as_ref().is_some_and(|queue| !queue.items.is_empty());
        let command = if !can_retry {
            commands.recv().ok()
        } else {
            match commands.recv_timeout(retry_at.saturating_duration_since(Instant::now())) {
                Ok(command) => Some(command),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if sharing.enabled.load(Relaxed)
                        && let Some(account) = &account
                        && let Some(queue) = &mut queue
                        && let Some(play) = queue.items.front()
                    {
                        match submit(&agent, account, play) {
                            Ok(()) => {
                                queue.items.pop_front();
                                notify(Event::Submission(String::new()));
                                persist(queue);
                            }
                            Err(error) if error.retry => {
                                notify(Event::Submission(format!(
                                    "{} Retrying in 30 seconds.",
                                    error.status
                                )));
                                retry_at = Instant::now() + RETRY;
                            }
                            Err(error) => {
                                notify(Event::Submission(error.status));
                                if matches!(error.code, Some(4 | 9 | 10 | 13 | 26)) {
                                    invalid_session = true;
                                } else {
                                    queue.items.pop_front();
                                    persist(queue);
                                }
                            }
                        }
                    }
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        };
        let Some(command) = command else { return };
        match command {
            Command::Begin(keys) => {
                let result = call(&agent, &keys, "auth.getToken", BTreeMap::new(), false).and_then(
                    |result| {
                        result["token"]
                            .as_str()
                            .filter(|value| !value.is_empty())
                            .map(str::to_owned)
                            .ok_or_else(|| {
                                ApiError::local("Last.fm did not supply an authorization token.")
                            })
                    },
                );
                match result {
                    Ok(token) => {
                        let mut url = url::Url::parse(AUTHORIZE).expect("valid authorization URL");
                        url.query_pairs_mut()
                            .append_pair("api_key", &keys.key)
                            .append_pair("token", &token);
                        authorization = Some((keys, token));
                        notify(Event::Authorization(url));
                    }
                    Err(error) => notify(Event::AuthError(error.status, false)),
                }
            }
            Command::Finish => {
                let Some((keys, token)) = authorization.take() else {
                    notify(Event::AuthError(
                        "Start the connection again.".into(),
                        false,
                    ));
                    continue;
                };
                let fields = BTreeMap::from([("token", token.clone())]);
                match call(&agent, &keys, "auth.getSession", fields, false).and_then(
                    |mut result| {
                        serde_json::from_value::<Session>(result["session"].take()).map_err(|_| {
                            ApiError::local("Last.fm did not supply an account session.")
                        })
                    },
                ) {
                    Ok(session) => {
                        let connected = Account { keys, session };
                        match write_account(&connected) {
                            Ok(()) => {
                                notify(Event::Connected(connected.session.name.clone()));
                                queue = Some(load(&connected.session.name));
                                invalid_session = false;
                                retry_at = Instant::now();
                                account = Some(connected);
                            }
                            Err(error) => notify(Event::AuthError(error, false)),
                        }
                    }
                    Err(error) => {
                        let waiting = error.retry || error.code == Some(14);
                        if waiting {
                            authorization = Some((keys, token));
                        }
                        notify(Event::AuthError(error.status, waiting));
                    }
                }
            }
            Command::Cancel => {
                authorization = None;
                notify(Event::Disconnected);
            }
            Command::Disconnect => {
                authorization = None;
                match delete_account() {
                    Ok(()) => {
                        account = None;
                        queue = None;
                        invalid_session = false;
                        notify(Event::Disconnected);
                        notify(Event::Pending(0));
                        if queue::Queue::clear(&queue_path).is_err() {
                            notify(Event::Submission(
                                "Could not clear the Last.fm scrobble queue.".into(),
                            ));
                        }
                    }
                    Err(error) => notify(Event::AuthError(error, false)),
                }
            }
            Command::Scrobble(generation, play) => {
                if generation == sharing.epoch.load(Relaxed)
                    && sharing.enabled.load(Relaxed)
                    && let Some(queue) = &mut queue
                {
                    queue.push(play);
                    persist(queue);
                }
            }
            Command::Wake => retry_at = Instant::now(),
            Command::NowPlaying {
                epoch,
                track_id,
                expires,
                track,
            } => {
                if !invalid_session
                    && epoch == sharing.epoch.load(Relaxed)
                    && track_id == sharing.track_id.load(Relaxed)
                    && sharing.enabled.load(Relaxed)
                    && sharing.now_playing.load(Relaxed)
                    && Instant::now() <= expires
                    && let Some(account) = &account
                    && let Err(error) = call(
                        &agent,
                        &account.keys,
                        "track.updateNowPlaying",
                        track.fields(account),
                        true,
                    )
                    && !error.retry
                {
                    notify(Event::Submission(error.status));
                }
            }
        }
    }
}

fn read_account() -> Result<Option<Account>, String> {
    let mut credential = std::ptr::null_mut();
    // Credential Manager owns the credential and its blob until CredFree.
    match unsafe { CredReadW(CREDENTIAL, CRED_TYPE_GENERIC, 0, &mut credential) } {
        Err(error) if error.code() == ERROR_NOT_FOUND.to_hresult() => return Ok(None),
        Err(_) => {
            return Err(
                "Could not read the Last.fm connection from Windows Credential Manager.".into(),
            );
        }
        Ok(()) => {}
    }
    let result = unsafe {
        let data = &*credential;
        if data.CredentialBlob.is_null() || data.CredentialBlobSize == 0 {
            Err("The stored Last.fm connection is empty.".into())
        } else {
            let bytes =
                std::slice::from_raw_parts(data.CredentialBlob, data.CredentialBlobSize as usize);
            serde_json::from_slice(bytes)
                .map(Some)
                .map_err(|_| "The stored Last.fm connection is invalid.".into())
        }
    };
    unsafe { CredFree(credential.cast()) };
    result
}

fn write_account(account: &Account) -> Result<(), String> {
    let mut blob =
        serde_json::to_vec(account).map_err(|_| "Could not encode the Last.fm connection.")?;
    let mut target: Vec<u16> = "Rust Player/Last.fm".encode_utf16().chain([0]).collect();
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(target.as_mut_ptr()),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        ..Default::default()
    };
    // Both buffers remain valid for this call. Windows copies them into its credential store.
    unsafe { CredWriteW(&credential, 0) }
        .map_err(|_| "Could not save the Last.fm connection in Windows Credential Manager.".into())
}

fn delete_account() -> Result<(), String> {
    // This target is specific to Rust Player, within the current Windows user's credentials.
    match unsafe { CredDeleteW(CREDENTIAL, CRED_TYPE_GENERIC, 0) } {
        Ok(()) => Ok(()),
        Err(error) if error.code() == ERROR_NOT_FOUND.to_hresult() => Ok(()),
        Err(_) => {
            Err("Could not remove the Last.fm connection from Windows Credential Manager.".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signs_sorted_utf8_parameters_before_form_encoding() {
        let fields = BTreeMap::from([
            ("track", "A & Ö".into()),
            ("artist", "A+B".into()),
            ("method", "track.scrobble".into()),
        ]);
        assert_eq!(
            signature(&fields, "test"),
            "7b6a23812065c960962335d59e756410"
        );
    }

    #[test]
    fn counts_audio_across_output_restart_and_submits_once() {
        let s = Duration::from_secs;
        assert_eq!(threshold(s(30)), None);
        assert_eq!(threshold(s(31)), Some(Duration::from_millis(15_500)));
        assert_eq!(threshold(s(600)), Some(s(240)));
        let mut listen = Listen::default();
        let now = UNIX_EPOCH + s(1000);
        assert_eq!(listen.add(s(12), s(20), now), None);
        assert_eq!(listen.add(s(12), s(20), now + s(60)), None);
        listen.rebase(Duration::ZERO);
        assert_eq!(listen.add(s(8), s(20), now + s(68)), Some(988));
        assert_eq!(listen.add(s(28), s(20), now + s(88)), None);
    }
}
