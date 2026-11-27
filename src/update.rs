use crate::{artwork, digest};
use eframe::egui;
use serde::Deserialize;
use std::{
    io::Read,
    path::PathBuf,
    process::Command,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
};

const RELEASE_API: &str = "https://api.github.com/repos/AikasAcc/rust-player/releases/latest";
const MAX_INSTALLER_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub page_url: String,
    download_url: String,
    sha256: String,
}

#[derive(Default)]
pub enum State {
    #[default]
    Idle,
    Checking(Receiver<Result<Option<Release>, String>>),
    UpToDate,
    Available(Release),
    Downloading(Receiver<Result<(tempfile::TempPath, String), String>>),
    Ready((tempfile::TempPath, String)),
    Installing,
    Failed(String),
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    html_url: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
}

impl State {
    pub fn check(&mut self, ctx: &egui::Context, current: &str) {
        if matches!(
            self,
            Self::Checking(_) | Self::Downloading(_) | Self::Installing
        ) {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let current = current.to_owned();
        let repaint = ctx.clone();
        thread::spawn(move || {
            let _ = sender.send(fetch_release(&current));
            repaint.request_repaint();
        });
        *self = Self::Checking(receiver);
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        let next = match self {
            Self::Checking(receiver) => {
                received(receiver, "Update check").map(|result| match result {
                    Ok(Some(release)) => Self::Available(release),
                    Ok(None) => Self::UpToDate,
                    Err(error) => Self::Failed(error),
                })
            }
            Self::Downloading(receiver) => {
                received(receiver, "Update download").map(|result| match result {
                    Ok(path) => Self::Ready(path),
                    Err(error) => Self::Failed(error),
                })
            }
            _ => None,
        };
        if let Some(next) = next {
            *self = next;
            ctx.request_repaint();
        }
    }

    pub fn download(&mut self, ctx: &egui::Context) {
        let Self::Available(release) = self else {
            return;
        };
        let release = release.clone();
        let (sender, receiver) = mpsc::channel();
        let repaint = ctx.clone();
        thread::spawn(move || {
            let _ = sender.send(download_release(&release));
            repaint.request_repaint();
        });
        *self = Self::Downloading(receiver);
    }

    pub fn install(&mut self) -> Result<(), String> {
        let current = std::env::current_exe().map_err(|error| error.to_string())?;
        let updater = current
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("Rust-Player-Updater.exe");
        if !updater.is_file() {
            return Err("Rust-Player-Updater.exe is missing next to Rust-Player.exe.".into());
        }
        // The child must never see a path that this process can delete while
        // it is reading. Persist the verified download before spawning it.
        let (staged, digest) = staged_download(self)?;
        let started = Command::new(updater)
            .arg("--wait-pid")
            .arg(std::process::id().to_string())
            .arg("--source")
            .arg(&staged)
            .arg("--digest")
            .arg(&digest)
            .arg("--target")
            .arg(&current)
            .spawn();
        if let Err(error) = started {
            let _ = std::fs::remove_file(&staged);
            let message = format!("Could not start the updater: {error}");
            *self = State::Failed(message.clone());
            return Err(message);
        }
        // The child now owns the staged file. A second click must not try to
        // hand it over again while the updater is already replacing the player.
        *self = State::Installing;
        Ok(())
    }
}

/// Move the staged file out of the state machine and make it persistent. The
/// updater takes over ownership only after this succeeds.
fn staged_download(state: &mut State) -> Result<(PathBuf, String), String> {
    if !matches!(state, State::Ready(_)) {
        return Err("No verified update is ready to install.".into());
    }
    let State::Ready((download, digest)) = std::mem::take(state) else {
        return Err("No verified update is ready to install.".into());
    };
    match download.keep() {
        Ok(path) => Ok((path, digest)),
        Err(error) => {
            // The TempPath is already consumed here. Leave a visible reason
            // in the UI instead of silently falling back to Idle.
            let message = format!("Could not persist the downloaded update: {error}");
            *state = State::Failed(message.clone());
            Err(message)
        }
    }
}

fn fetch_release(current: &str) -> Result<Option<Release>, String> {
    let mut response = artwork::https_agent_with_timeout(std::time::Duration::from_secs(300))
        .get(RELEASE_API)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "Rust-Player-Updater")
        .call()
        .map_err(|error| format!("Update check failed: {error}"))?;
    let release: ApiRelease = response
        .body_mut()
        .read_json()
        .map_err(|error| format!("Could not read the update response: {error}"))?;
    release_from(release, current)
}

/// Pure validation of the GitHub response, kept separate from network I/O so
/// the asset name, URL and digest checks can be regression-tested offline.
fn release_from(release: ApiRelease, current: &str) -> Result<Option<Release>, String> {
    if release.draft || release.prerelease || !newer(&release.tag_name, current) {
        return Ok(None);
    }
    let expected = format!(
        "Rust-Player-v{}-Windows-x64.exe",
        release.tag_name.trim_start_matches('v')
    );
    let asset = release
        .assets
        .into_iter()
        .find(|asset| asset.name == expected)
        .ok_or_else(|| {
            format!(
                "Release {} has no Windows x64 updater asset.",
                release.tag_name
            )
        })?;
    if asset.browser_download_url
        != format!(
            "https://github.com/AikasAcc/rust-player/releases/download/{}/{}",
            release.tag_name, expected
        )
    {
        return Err("Unexpected GitHub asset URL.".into());
    }
    let sha256 = asset
        .digest
        .as_deref()
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .filter(|digest| digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| "The release does not provide a usable SHA-256 digest.".to_owned())?;
    Ok(Some(Release {
        version: release.tag_name,
        page_url: release.html_url,
        download_url: asset.browser_download_url,
        sha256: sha256.to_ascii_lowercase(),
    }))
}

fn download_release(release: &Release) -> Result<(tempfile::TempPath, String), String> {
    let mut response = artwork::https_agent()
        .get(&release.download_url)
        .header("Accept", "application/octet-stream")
        .header("User-Agent", "Rust-Player-Updater")
        .call()
        .map_err(|error| format!("Update download failed: {error}"))?;
    // ureq enforces the size limit while reading; hashing during the copy
    // avoids holding the whole installer (up to 256 MiB) in memory.
    let reader = response
        .body_mut()
        .with_config()
        .limit(MAX_INSTALLER_BYTES)
        .reader();
    let staged = stream_verified(reader, &release.sha256)?;
    Ok((staged, release.sha256.clone()))
}

/// Copy a bounded reader to a temporary file while computing its digest. The
/// file is only handed to the updater after the digest matches.
fn stream_verified<R: Read>(reader: R, expected: &str) -> Result<tempfile::TempPath, String> {
    let mut file = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    digest::copy_verified(reader, &mut file, expected)?;
    Ok(file.into_temp_path())
}

/// `try_recv().ok()` cannot distinguish "still running" from "worker died".
/// The latter must leave the Failed state, not spin forever in Checking.
fn received<T>(receiver: &Receiver<Result<T, String>>, task: &str) -> Option<Result<T, String>> {
    match receiver.try_recv() {
        Ok(result) => Some(result),
        Err(TryRecvError::Empty) => None,
        Err(TryRecvError::Disconnected) => Some(Err(format!("{task} stopped unexpectedly."))),
    }
}

fn newer(candidate: &str, current: &str) -> bool {
    fn version(value: &str) -> Option<[u32; 3]> {
        let parts = value
            .strip_prefix('v')
            .unwrap_or(value)
            .split('.')
            .map(str::parse::<u32>)
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        parts.try_into().ok()
    }
    version(candidate)
        .zip(version(current))
        .is_some_and(|(a, b)| a > b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn a_streamed_download_is_staged_only_when_its_digest_matches() {
        let payload = b"abc".to_vec();
        let digest = "00".repeat(32);
        let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

        let staged = stream_verified(Cursor::new(&payload), expected).unwrap();
        assert_eq!(std::fs::read(&staged).unwrap(), payload);
        std::fs::remove_file(&staged).unwrap();

        let error = stream_verified(Cursor::new(&payload), &digest).unwrap_err();
        assert!(error.contains("SHA-256"), "{error}");
    }

    #[test]
    fn compares_release_versions() {
        assert!(newer("v0.4.2", "0.4.1"));
        assert!(!newer("v0.4.1", "0.4.1"));
    }

    fn api(digest: Option<&str>, name: &str, url: &str) -> ApiRelease {
        ApiRelease {
            tag_name: "v0.7.0".into(),
            html_url: "https://github.com/AikasAcc/rust-player/releases/tag/v0.7.0".into(),
            draft: false,
            prerelease: false,
            assets: vec![ApiAsset {
                name: name.into(),
                browser_download_url: url.into(),
                digest: digest.map(str::to_owned),
            }],
        }
    }

    fn valid_api() -> ApiRelease {
        api(
            Some("sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
            "Rust-Player-v0.7.0-Windows-x64.exe",
            "https://github.com/AikasAcc/rust-player/releases/download/v0.7.0/Rust-Player-v0.7.0-Windows-x64.exe",
        )
    }

    #[test]
    fn a_valid_release_is_accepted_with_a_normalized_digest() {
        let release = release_from(valid_api(), "0.6.0").unwrap().unwrap();
        assert_eq!(release.version, "v0.7.0");
        assert_eq!(
            release.sha256,
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        );
        assert!(release.download_url.ends_with("Windows-x64.exe"));
    }

    #[test]
    fn drafts_prereleases_and_older_tags_are_skipped() {
        let mut candidate = valid_api();
        candidate.draft = true;
        assert!(release_from(candidate, "0.6.0").unwrap().is_none());
        let mut candidate = valid_api();
        candidate.prerelease = true;
        assert!(release_from(candidate, "0.6.0").unwrap().is_none());
        assert!(release_from(valid_api(), "0.7.0").unwrap().is_none());
    }

    #[test]
    fn wrong_asset_name_url_or_digest_is_rejected() {
        let expected_name = "Rust-Player-v0.7.0-Windows-x64.exe";
        let expected_url = "https://github.com/AikasAcc/rust-player/releases/download/v0.7.0/Rust-Player-v0.7.0-Windows-x64.exe";
        let candidate = api(None, "wrong.exe", expected_url);
        assert!(
            release_from(candidate, "0.6.0")
                .unwrap_err()
                .contains("updater asset")
        );
        let candidate = api(None, expected_name, "https://evil.example/update.exe");
        assert!(
            release_from(candidate, "0.6.0")
                .unwrap_err()
                .contains("URL")
        );
        for digest in [
            None,
            Some("deadbeef"),
            Some("sha256:zz00000000000000000000000000000000000000000000000000000000000000"),
        ] {
            let candidate = api(digest, expected_name, expected_url);
            assert!(
                release_from(candidate, "0.6.0")
                    .unwrap_err()
                    .contains("SHA-256")
            );
        }
    }

    #[test]
    fn a_staged_download_survives_the_state_transition() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let path = file.path().to_path_buf();
        let mut state = State::Ready((file.into_temp_path(), "digest".into()));
        let (staged, digest) = staged_download(&mut state).unwrap();
        assert_eq!(staged, path);
        assert_eq!(digest, "digest");
        assert!(staged.is_file(), "the updater's source must not vanish");
        assert!(matches!(state, State::Idle));
        std::fs::remove_file(staged).unwrap();

        let mut idle = State::Idle;
        assert!(staged_download(&mut idle).is_err());
    }

    #[test]
    fn a_second_install_cannot_reuse_the_staged_download() {
        let ctx = egui::Context::default();
        let mut installing = State::Installing;
        assert!(staged_download(&mut installing).is_err());
        assert!(
            matches!(installing, State::Installing),
            "a failed install attempt must not erase the installing status"
        );
        installing.check(&ctx, "0.7.0");
        assert!(matches!(installing, State::Installing));
    }

    #[test]
    fn a_dead_worker_does_not_leave_the_ui_waiting() {
        let ctx = egui::Context::default();
        let (_, check) = mpsc::channel();
        let mut state = State::Checking(check);
        state.poll(&ctx);
        assert!(matches!(state, State::Failed(message) if message.contains("Update check")));

        let (_, download) = mpsc::channel();
        let mut state = State::Downloading(download);
        state.poll(&ctx);
        assert!(matches!(state, State::Failed(message) if message.contains("Update download")));
    }

    #[test]
    fn an_empty_channel_keeps_waiting() {
        let ctx = egui::Context::default();
        let (sender, check) = mpsc::channel();
        let mut state = State::Checking(check);
        state.poll(&ctx);
        assert!(matches!(state, State::Checking(_)));
        drop(sender);
    }
}
