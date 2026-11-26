use serde_json::{Value, json};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::windows::io::AsRawHandle,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows::Win32::{Foundation::HANDLE, System::Pipes::PeekNamedPipe};

const PROJECT_URL: &str = "https://github.com/AikasAcc/rust-player";

pub struct Activity {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub artwork: String,
    pub fallback_artwork: String,
    pub elapsed: Duration,
    pub duration: Option<Duration>,
}

struct Update {
    id: Option<String>,
    activity: Option<Activity>,
    sent_at: SystemTime,
}

pub struct Presence {
    tx: mpsc::Sender<Update>,
    status: Arc<Mutex<String>>,
}

impl Presence {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let status = Arc::new(Mutex::new("Sharing is off".to_owned()));
        let worker_status = Arc::clone(&status);
        std::thread::spawn(move || run(rx, worker_status));
        Self { tx, status }
    }

    pub fn status(&self) -> String {
        self.status.lock().expect("status lock").clone()
    }

    pub fn update(&self, id: Option<String>, activity: Option<Activity>) {
        if self
            .tx
            .send(Update {
                id,
                activity,
                sent_at: SystemTime::now(),
            })
            .is_err()
        {
            worker_stopped(&self.status);
        }
    }
}

/// Without this the UI would keep showing the last successful Discord status
/// forever if the worker thread panicked.
fn worker_stopped(status: &Arc<Mutex<String>>) {
    if let Ok(mut status) = status.lock() {
        *status = "Discord status worker stopped unexpectedly.".into();
    }
}

fn run(rx: mpsc::Receiver<Update>, status: Arc<Mutex<String>>) {
    let mut connection: Option<(String, Rpc)> = None;
    let mut current = None;
    loop {
        let mut update = match rx.recv_timeout(Duration::from_secs(15)) {
            Ok(update) => update,
            Err(mpsc::RecvTimeoutError::Timeout) => match current.take() {
                Some(update) => update,
                None => continue,
            },
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        while let Ok(next) = rx.try_recv() {
            update = next;
        }
        current = None;
        if connection
            .as_ref()
            .is_some_and(|(id, _)| update.id.as_ref() != Some(id) || update.activity.is_none())
            && let Some((_, mut rpc)) = connection.take()
        {
            let _ = rpc.set_activity(Value::Null);
        }
        let message = match (&update.id, &update.activity) {
            (None, _) => "Sharing is off".into(),
            (Some(_), None) => "Waiting for playback".into(),
            (Some(id), Some(track)) => {
                let result = (|| {
                    if connection.is_none() {
                        connection = Some((id.clone(), Rpc::connect(id)?));
                    }
                    let (_, rpc) = connection.as_mut().expect("connection established");
                    for (image, source) in artwork_attempts(track) {
                        match rpc.set_activity(payload(track, update.sent_at, image)) {
                            Ok(reply) => return Ok(accepted(&reply, source)),
                            Err(RpcError::Rejected(_)) => continue,
                            Err(error) => return Err(error),
                        }
                    }
                    let reply = rpc.set_activity(payload(track, update.sent_at, ""))?;
                    Ok(accepted(&reply, "no artwork"))
                })();
                current = Some(update);
                match result {
                    Ok(message) => message,
                    Err(error) => {
                        connection = None;
                        error.message()
                    }
                }
            }
        };
        *status.lock().expect("status lock") = message;
    }
    if let Some((_, mut rpc)) = connection {
        let _ = rpc.set_activity(Value::Null);
    }
}

enum RpcError {
    Transport(String),
    Rejected(String),
}

impl RpcError {
    fn message(self) -> String {
        match self {
            Self::Transport(error) => format!("Discord connection: {error}"),
            Self::Rejected(error) => format!("Discord rejected activity: {error}"),
        }
    }
}

impl From<std::io::Error> for RpcError {
    fn from(error: std::io::Error) -> Self {
        Self::Transport(error.to_string())
    }
}

struct Rpc {
    pipe: File,
    nonce: u64,
}

impl Rpc {
    fn connect(id: &str) -> Result<Self, RpcError> {
        let pipe = (0..10)
            .find_map(|index| {
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(format!(r"\\.\pipe\discord-ipc-{index}"))
                    .ok()
            })
            .ok_or_else(|| RpcError::Transport("Open the Discord desktop app.".into()))?;
        Self::handshake(pipe, id)
    }

    fn handshake(pipe: File, id: &str) -> Result<Self, RpcError> {
        let mut rpc = Self { pipe, nonce: 0 };
        rpc.send(
            0,
            &serde_json::to_vec(&json!({ "v": 1, "client_id": id })).expect("handshake JSON"),
        )?;
        rpc.wait_for(|value| value["evt"] == "READY")?;
        Ok(rpc)
    }

    fn set_activity(&mut self, activity: Value) -> Result<Value, RpcError> {
        self.nonce += 1;
        let nonce = self.nonce.to_string();
        let request = json!({
            "cmd": "SET_ACTIVITY",
            "args": {"pid": std::process::id(), "activity": activity},
            "nonce": nonce,
        });
        self.send(1, &serde_json::to_vec(&request).expect("activity JSON"))?;
        self.wait_for(|value| value["cmd"] == "SET_ACTIVITY" && value["nonce"] == nonce)
    }

    fn send(&mut self, opcode: u32, body: &[u8]) -> Result<(), RpcError> {
        let mut frame = Vec::with_capacity(8 + body.len());
        frame.extend(opcode.to_le_bytes());
        frame.extend((body.len() as u32).to_le_bytes());
        frame.extend(body);
        if self.pipe.write(&frame)? != frame.len() {
            return Err(RpcError::Transport("Incomplete IPC write.".into()));
        }
        Ok(())
    }

    fn wait_for(&mut self, matches: impl Fn(&Value) -> bool) -> Result<Value, RpcError> {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let mut header = [0; 8];
            self.read_until(&mut header, deadline)?;
            let opcode = u32::from_le_bytes(header[..4].try_into().expect("opcode bytes"));
            let size = u32::from_le_bytes(header[4..].try_into().expect("length bytes")) as usize;
            if size > 64 * 1024 {
                return Err(RpcError::Transport("Oversized IPC reply.".into()));
            }
            let mut body = vec![0; size];
            self.read_until(&mut body, deadline)?;
            if opcode == 3 {
                self.send(4, &body)?;
                continue;
            }
            let reply: Value = serde_json::from_slice(&body)
                .map_err(|error| RpcError::Transport(format!("Invalid IPC reply: {error}")))?;
            if opcode == 2 || reply["evt"] == "ERROR" {
                let text = reply["data"]["message"]
                    .as_str()
                    .or_else(|| reply["message"].as_str())
                    .unwrap_or("The desktop client refused the request.");
                return Err(RpcError::Rejected(text.chars().take(200).collect()));
            }
            if opcode == 1 && matches(&reply) {
                return Ok(reply);
            }
        }
    }

    fn read_until(&mut self, bytes: &mut [u8], deadline: Instant) -> Result<(), RpcError> {
        if bytes.is_empty() {
            return Ok(());
        }
        let handle = HANDLE(self.pipe.as_raw_handle() as isize);
        while Instant::now() < deadline {
            let mut available = 0;
            // The file owns this handle, and this worker is the only pipe reader.
            unsafe { PeekNamedPipe(handle, None, 0, None, Some(&mut available), None) }
                .map_err(|error| RpcError::Transport(error.to_string()))?;
            if available as usize >= bytes.len() {
                self.pipe.read_exact(bytes)?;
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Err(RpcError::Transport("Discord did not reply in time.".into()))
    }
}

pub fn public_artwork_url(input: &str) -> bool {
    input.len() <= 300
        && !input.chars().any(char::is_whitespace)
        && url::Url::parse(input).is_ok_and(|url| {
            url.scheme() == "https"
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.fragment().is_none()
        })
}

fn payload(track: &Activity, sent_at: SystemTime, artwork: &str) -> Value {
    let now = sent_at.duration_since(UNIX_EPOCH).unwrap_or_default();
    let start = now.saturating_sub(track.elapsed).as_secs();
    let mut timestamps = json!({ "start": start });
    if let Some(duration) = track.duration.filter(|d| *d > track.elapsed) {
        timestamps["end"] = json!(start.saturating_add(duration.as_secs()));
    }
    let mut activity = json!({
        "type": 2,
        "details": text(&track.title),
        "state": text(if track.artist.is_empty() { "Unknown artist" } else { &track.artist }),
        "timestamps": timestamps,
        "buttons": [{"label": "Rust Player on GitHub", "url": PROJECT_URL}],
    });
    if !artwork.is_empty() {
        let hover = if track.album.is_empty() {
            &track.title
        } else {
            &track.album
        };
        activity["assets"] = json!({"large_image": artwork, "large_text": text(hover)});
    }
    activity
}

/// Artwork sources in priority order. The label follows the URL actually sent,
/// so an accepted fallback is never reported as primary artwork. An identical
/// fallback is skipped: retrying the same URL cannot help.
fn artwork_attempts(activity: &Activity) -> Vec<(&str, &'static str)> {
    let mut attempts = Vec::new();
    if !activity.artwork.is_empty() {
        attempts.push((activity.artwork.as_str(), "artwork"));
    }
    if !activity.fallback_artwork.is_empty() && activity.fallback_artwork != activity.artwork {
        attempts.push((activity.fallback_artwork.as_str(), "fallback artwork"));
    }
    attempts
}

fn accepted(reply: &Value, source: &str) -> String {
    let buttons: Vec<&str> = reply["data"]["buttons"]
        .as_array()
        .map(|buttons| buttons.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if buttons.is_empty() {
        format!("Discord accepted the activity with {source}.")
    } else {
        format!(
            "Discord accepted the activity with {source}; button: {}.",
            buttons.join(", ")
        )
    }
}

fn text(value: &str) -> String {
    let mut output = String::new();
    for character in value.trim().chars().filter(|ch| !ch.is_control()) {
        if output.len() + character.len_utf8() > 128 {
            break;
        }
        output.push(character);
    }
    if output.is_empty() {
        return "Unknown".into();
    }
    if output.len() == 1 {
        output.push(' ');
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dead_presence_worker_is_visible_in_the_status() {
        let status = Arc::new(Mutex::new("Sharing is off".to_owned()));
        worker_stopped(&status);
        assert!(status.lock().unwrap().contains("stopped unexpectedly"));
    }

    #[test]
    fn artwork_source_labels_match_the_url_actually_sent() {
        let mut track = Activity {
            title: "Song".into(),
            artist: "Artist".into(),
            album: "Album".into(),
            artwork: String::new(),
            fallback_artwork: "cover".into(),
            elapsed: Duration::ZERO,
            duration: None,
        };
        assert_eq!(
            artwork_attempts(&track),
            vec![("cover", "fallback artwork")]
        );

        track.artwork.push_str("cover");
        assert_eq!(artwork_attempts(&track), vec![("cover", "artwork")]);

        track.fallback_artwork.push_str("-small");
        assert_eq!(
            artwork_attempts(&track),
            vec![("cover", "artwork"), ("cover-small", "fallback artwork")]
        );
    }

    #[test]
    fn activity_exposes_the_github_project_button() {
        let track = Activity {
            title: "Song".into(),
            artist: "Artist".into(),
            album: "Album".into(),
            artwork: "cover".into(),
            fallback_artwork: String::new(),
            elapsed: Duration::from_secs(5),
            duration: Some(Duration::from_secs(200)),
        };
        let value = payload(&track, SystemTime::now(), "cover");
        assert_eq!(value["buttons"][0]["label"], "Rust Player on GitHub");
        assert_eq!(value["buttons"][0]["url"], PROJECT_URL);
    }

    fn sample(artist: &str, album: &str, duration: Option<Duration>) -> Activity {
        Activity {
            title: "Song".into(),
            artist: artist.into(),
            album: album.into(),
            artwork: String::new(),
            fallback_artwork: String::new(),
            elapsed: Duration::from_secs(30),
            duration,
        }
    }

    #[test]
    fn timestamps_use_elapsed_time_and_omit_end_when_duration_is_unknown() {
        let sent = UNIX_EPOCH + Duration::from_secs(1_000);
        let value = payload(
            &sample("Artist", "Album", Some(Duration::from_secs(200))),
            sent,
            "",
        );
        assert_eq!(value["timestamps"]["start"], 970);
        assert_eq!(value["timestamps"]["end"], 1_170);
        assert!(value["assets"].is_null());

        let value = payload(&sample("Artist", "Album", None), sent, "");
        assert_eq!(value["timestamps"]["start"], 970);
        assert!(value["timestamps"].get("end").is_none());
    }

    #[test]
    fn text_strips_controls_truncates_bytes_and_fills_empty_fields() {
        assert_eq!(text("  So\u{1b}ng  "), "Song");
        assert_eq!(text("   "), "Unknown");
        assert_eq!(text("x"), "x ");
        assert_eq!(text(&"é".repeat(100)).len(), 128);
    }

    #[test]
    fn missing_tags_still_produce_a_valid_activity() {
        let value = payload(&sample("", "", None), UNIX_EPOCH, "cover");
        assert_eq!(value["state"], "Unknown artist");
        assert_eq!(value["assets"]["large_image"], "cover");
        assert_eq!(value["assets"]["large_text"], "Song");
    }

    #[test]
    fn artwork_urls_must_be_public_https_links() {
        assert!(public_artwork_url("https://example.com/cover.jpg?size=600"));
        assert!(!public_artwork_url("http://example.com/cover.jpg"));
        assert!(!public_artwork_url(
            "https://user:pass@example.com/cover.jpg"
        ));
        assert!(!public_artwork_url("https://example.com/my cover.jpg"));
        assert!(!public_artwork_url("https://example.com/cover.jpg#part"));
        assert!(!public_artwork_url(&format!(
            "https://example.com/{}",
            "a".repeat(300)
        )));
    }

    #[test]
    fn accepted_reply_reports_the_button_discord_stored() {
        let reply = json!({"data": {"buttons": ["Rust Player on GitHub"]}});
        assert_eq!(
            accepted(&reply, "artwork"),
            "Discord accepted the activity with artwork; button: Rust Player on GitHub."
        );
        assert_eq!(
            accepted(&json!({"data": {}}), "no artwork"),
            "Discord accepted the activity with no artwork."
        );
    }
}
