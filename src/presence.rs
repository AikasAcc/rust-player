use serde_json::{Value, json};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::windows::io::AsRawHandle,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows::Win32::{Foundation::HANDLE, System::Pipes::PeekNamedPipe};

pub struct Activity {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub artwork: String,
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
        let _ = self.tx.send(Update {
            id,
            activity,
            sent_at: SystemTime::now(),
        });
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
        {
            if let Some((_, mut rpc)) = connection.take() {
                let _ = rpc.set_activity(Value::Null);
            }
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
                    let mut activity = payload(track, update.sent_at);
                    match rpc.set_activity(activity.clone()) {
                        Err(RpcError::Rejected(_)) if activity.get("assets").is_some() => {
                            activity
                                .as_object_mut()
                                .expect("activity is an object")
                                .remove("assets");
                            rpc.set_activity(activity)?;
                            Ok("Discord confirmed activity (artwork was rejected)")
                        }
                        Err(error) => Err(error),
                        Ok(()) => Ok("Discord confirmed activity"),
                    }
                })();
                current = Some(update);
                match result {
                    Ok(message) => message.into(),
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

    fn set_activity(&mut self, activity: Value) -> Result<(), RpcError> {
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

    fn wait_for(&mut self, matches: impl Fn(&Value) -> bool) -> Result<(), RpcError> {
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
                return Ok(());
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

fn payload(track: &Activity, sent_at: SystemTime) -> Value {
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
    });
    if !track.artwork.is_empty() {
        let hover = if track.album.is_empty() {
            &track.title
        } else {
            &track.album
        };
        activity["assets"] = json!({"large_image": track.artwork, "large_text": text(hover)});
    }
    activity
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
