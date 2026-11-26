use crate::Player;
use eframe::egui;
use std::{
    fs,
    io::Write,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

fn logs() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("Rust Player")
        .join("logs")
}

pub fn install() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let folder = logs();
        if fs::create_dir_all(&folder).is_ok() {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            let path = folder.join(format!(
                "crash-{}-{}.log",
                timestamp.as_millis(),
                std::process::id()
            ));
            if let Ok(mut file) = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
            {
                let _ = writeln!(
                    file,
                    "Rust Player {} · {} · Unix time {}",
                    env!("CARGO_PKG_VERSION"),
                    std::env::consts::ARCH,
                    timestamp.as_secs()
                );
                if let Some(location) = info.location() {
                    let _ = writeln!(file, "Panic at {}:{}", location.file(), location.line());
                }
                let _ = writeln!(file, "{}", std::backtrace::Backtrace::force_capture());
            }
        }
        previous(info);
    }));
}

/// Persist a background failure. A GUI build has no console, so stderr would
/// silently discard problems such as a failed cache write.
pub fn append_error(tag: &str, message: &str) {
    append_to(&logs(), tag, message);
}

/// Persist a failure that happens while the window is closing. There is no UI
/// left to show a status message at that point.
pub fn append_shutdown(message: &str) {
    append_error("shutdown", message);
}

fn append_to(folder: &std::path::Path, tag: &str, message: &str) {
    if fs::create_dir_all(folder).is_err() {
        return;
    }
    let path = folder.join(format!("{tag}-{}.log", std::process::id()));
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let _ = writeln!(file, "{seconds} {message}");
    }
}

impl Player {
    pub(super) fn diagnostic_settings(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui.button("Open log folder").clicked() {
                let folder = logs();
                let result = fs::create_dir_all(&folder).and_then(|()| {
                    std::process::Command::new("explorer.exe")
                        .arg(folder)
                        .spawn()
                        .map(|_| ())
                });
                if let Err(error) = result {
                    self.status = format!("Could not open the log folder: {error}");
                }
            }
            if ui.button("Copy diagnostics").clicked() {
                let mut details = format!(
                    "Rust Player {}\nPlatform: {} {}\nRequested output: {:?}\n",
                    env!("CARGO_PKG_VERSION"),
                    std::env::consts::OS,
                    std::env::consts::ARCH,
                    self.library.audio_output.mode
                );
                if let Some(playing) = &self.playing {
                    use std::fmt::Write;
                    let sink = &playing.sink;
                    let source = &sink.source;
                    let _ = writeln!(
                        details,
                        "Decoder: {} · {} Hz · {} ch · {}",
                        source.codec,
                        source.rate,
                        source.channels,
                        source.kind.label()
                    );
                    if let Some(output) = &sink.output {
                        let _ = writeln!(
                            details,
                            "Opened: {:?} · {} Hz · {} ch · {}\nUnderruns: {}\nBuffer frames: {:?}",
                            output.mode,
                            output.rate,
                            output.channels,
                            output.sample,
                            sink.underruns(),
                            output.buffer_frames
                        );
                    }
                }
                ui.ctx().copy_text(details);
                self.status = "Diagnostics copied.".into();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::append_to;

    #[test]
    fn background_errors_are_appended_with_their_tag() {
        let dir = tempfile::tempdir().unwrap();
        append_to(dir.path(), "metadata-cache", "first failure");
        append_to(dir.path(), "metadata-cache", "second failure");
        let path = dir
            .path()
            .join(format!("metadata-cache-{}.log", std::process::id()));
        let contents = std::fs::read_to_string(path).unwrap();
        assert_eq!(contents.lines().count(), 2);
        assert!(contents.lines().all(|line| line.ends_with(" failure")));
    }
}
