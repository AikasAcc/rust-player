use crate::{artwork, digest};
use eframe::egui;
use serde::Deserialize;
use std::{
    io::Write,
    process::Command,
    sync::mpsc::{self, Receiver},
    thread,
};

const RELEASE_API: &str = "https://api.github.com/repos/AikasAcc/rust-player/releases/latest";
const MAX_INSTALLER_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone)]
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
        if matches!(self, Self::Checking(_) | Self::Downloading(_)) {
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
            Self::Checking(receiver) => receiver.try_recv().ok().map(|result| match result {
                Ok(Some(release)) => Self::Available(release),
                Ok(None) => Self::UpToDate,
                Err(error) => Self::Failed(error),
            }),
            Self::Downloading(receiver) => receiver.try_recv().ok().map(|result| match result {
                Ok(path) => Self::Ready(path),
                Err(error) => Self::Failed(error),
            }),
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
        let Self::Ready((download, digest)) = self else {
            return Err("No verified update is ready to install.".into());
        };
        let current = std::env::current_exe().map_err(|error| error.to_string())?;
        let updater = current
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("Rust-Player-Updater.exe");
        if !updater.is_file() {
            return Err("Rust-Player-Updater.exe is missing next to Rust-Player.exe.".into());
        }
        Command::new(updater)
            .arg("--wait-pid")
            .arg(std::process::id().to_string())
            .arg("--source")
            .arg(download.as_os_str())
            .arg("--digest")
            .arg(digest)
            .arg("--target")
            .arg(&current)
            .spawn()
            .map_err(|error| format!("Could not start the updater: {error}"))?;
        // Transfer ownership of the staged file to the updater process.
        if let State::Ready((path, _)) = std::mem::take(self) {
            path.keep().map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

fn fetch_release(current: &str) -> Result<Option<Release>, String> {
    let mut response = artwork::https_agent()
        .get(RELEASE_API)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "Rust-Player-Updater")
        .call()
        .map_err(|error| format!("Update check failed: {error}"))?;
    let release: ApiRelease = response
        .body_mut()
        .read_json()
        .map_err(|error| format!("Could not read the update response: {error}"))?;
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
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_INSTALLER_BYTES)
        .read_to_vec()
        .map_err(|error| format!("Could not download the update: {error}"))?;
    digest::verify(&bytes, &release.sha256)?;
    let mut file = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    Ok((file.into_temp_path(), release.sha256.clone()))
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
    use super::newer;
    #[test]
    fn compares_release_versions() {
        assert!(newer("v0.4.2", "0.4.1"));
        assert!(!newer("v0.4.1", "0.4.1"));
    }
}
