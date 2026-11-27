//! Per-user Windows file association for audio files.
//!
//! No administrator rights are needed: everything lives below
//! `HKEY_CURRENT_USER\Software\Classes`. An association is only removed when
//! it still points at this player, so another application's default is never
//! overwritten by the cleanup action.

use crate::library;
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const PROG_ID: &str = "RustPlayer.Audio";
const CLASSES: &str = r"Software\Classes";
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Value written for the `shell\open\command` verb. Windows substitutes `%1`
/// with the path that was double-clicked.
pub fn open_command(exe: &Path) -> String {
    format!("\"{}\" \"%1\"", exe.to_string_lossy())
}

/// Only extensions the in-app importer understands are associated.
pub fn is_supported(path: &Path) -> bool {
    path.extension().and_then(OsStr::to_str).is_some_and(|ext| {
        let ext = ext.to_ascii_lowercase();
        library::AUDIO.contains(&ext.as_str())
    })
}

/// Paths Windows passes on a double-click, already filtered to audio files.
pub fn startup_paths() -> Vec<PathBuf> {
    std::env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .filter(|path| is_supported(path))
        .collect()
}

fn reg(args: &[&str]) -> Result<String, String> {
    let mut command = Command::new("reg");
    command.args(args);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let output = command
        .output()
        .map_err(|error| format!("Could not run reg.exe: {error}"))?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if message.is_empty() {
            "The Windows registry command failed.".to_string()
        } else {
            message
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn prog_key() -> String {
    format!(r"HKCU\{CLASSES}\{PROG_ID}")
}

fn extension_key(ext: &str) -> String {
    format!(r"HKCU\{CLASSES}.{ext}")
}

/// Tell Explorer that file associations changed so icons and verbs refresh
/// without signing out.
#[cfg(windows)]
fn notify_shell() {
    // SAFETY: SHChangeNotify takes no pointers here and is called with the
    // documented ASSOCCHANGED/IDLIST combination.
    unsafe {
        windows::Win32::UI::Shell::SHChangeNotify(
            windows::Win32::UI::Shell::SHCNE_ASSOCCHANGED,
            windows::Win32::UI::Shell::SHCNF_IDLIST,
            None,
            None,
        );
    }
}

#[cfg(not(windows))]
fn notify_shell() {}

/// Register the player as the per-user default for supported audio files.
/// An explicit button click is required; startup never changes associations.
pub fn register() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let command = open_command(&exe);
    reg(&[
        "add",
        &prog_key(),
        "/ve",
        "/d",
        "Rust Player audio file",
        "/f",
    ])?;
    reg(&[
        "add",
        &format!(r"{}\shell\open\command", prog_key()),
        "/ve",
        "/d",
        &command,
        "/f",
    ])?;
    for ext in library::AUDIO {
        reg(&["add", &extension_key(ext), "/ve", "/d", PROG_ID, "/f"])?;
    }
    notify_shell();
    Ok(())
}

/// Drop only associations that still point at this player.
pub fn unregister() -> Result<(), String> {
    for ext in library::AUDIO {
        let key = extension_key(ext);
        if reg(&["query", &key, "/ve"])
            .is_ok_and(|value| value.split_whitespace().any(|part| part == PROG_ID))
        {
            reg(&["delete", &key, "/f"])?;
        }
    }
    let _ = reg(&["delete", &prog_key(), "/f"]);
    notify_shell();
    Ok(())
}

/// Whether `.mp3` currently opens with this player. Other extensions are
/// registered together, so one representative check is enough for the UI.
pub fn is_registered() -> bool {
    reg(&["query", &extension_key("mp3"), "/ve"])
        .is_ok_and(|value| value.split_whitespace().any(|part| part == PROG_ID))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_command_quotes_the_player_and_passes_the_file() {
        let command = open_command(Path::new(r"C:\Program Files\Rust Player\rust-player.exe"));
        assert_eq!(
            command,
            r#""C:\Program Files\Rust Player\rust-player.exe" "%1""#
        );
    }

    #[test]
    fn only_importable_extensions_are_accepted() {
        assert!(is_supported(Path::new("Song.MP3")));
        assert!(is_supported(Path::new("album.ape")));
        assert!(is_supported(Path::new("disc.wv")));
        assert!(is_supported(Path::new("score.cue")));
        assert!(!is_supported(Path::new("notes.txt")));
        assert!(!is_supported(Path::new("no-extension")));
    }
}
