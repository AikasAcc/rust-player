#![windows_subsystem = "windows"]
#[path = "../digest.rs"]
mod digest;
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
};
use windows::Win32::{
    Foundation::{CloseHandle, WAIT_OBJECT_0},
    System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
};

fn arg(key: &str) -> io::Result<std::ffi::OsString> {
    env::args_os()
        .skip_while(|v| v != key)
        .nth(1)
        .ok_or_else(|| io::Error::other("Missing update argument"))
}
fn replace() -> Result<(), Box<dyn std::error::Error>> {
    let source = PathBuf::from(arg("--source")?);
    let target = PathBuf::from(arg("--target")?);
    let expected = arg("--digest")?;
    let pid: u32 = arg("--wait-pid")?.to_string_lossy().parse()?;
    let dir = target.parent().ok_or("Missing install folder")?;
    if dir.canonicalize()?
        != env::current_exe()?
            .parent()
            .ok_or("Missing updater folder")?
            .canonicalize()?
    {
        return Err("Install folder does not match the updater".into());
    }
    let mut staged = tempfile::NamedTempFile::new_in(dir)?;
    // Verify while copying: a wrong digest discards the staged file instead
    // of reading the installer twice.
    digest::copy_verified(
        fs::File::open(&source)?,
        &mut staged,
        &expected.to_string_lossy(),
    )?;
    staged.as_file().sync_all()?;
    unsafe {
        match OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            Ok(handle) => {
                let result = WaitForSingleObject(handle, 90_000);
                CloseHandle(handle)?;
                if result != WAIT_OBJECT_0 {
                    return Err("Player is still running".into());
                }
            }
            Err(e) if e.code().0 as u32 == 0x80070057 => (),
            Err(e) => return Err(e.into()),
        }
    }
    install(staged, &target)?;
    // The new player is already installed and running. A stale staged file is
    // cosmetic; failing the whole update here would misreport success.
    let _ = fs::remove_file(source);
    Ok(())
}

fn install(staged: tempfile::NamedTempFile, target: &Path) -> io::Result<()> {
    let mut backup = tempfile::NamedTempFile::new_in(
        target
            .parent()
            .ok_or_else(|| io::Error::other("Missing folder"))?,
    )?;
    io::copy(&mut fs::File::open(target)?, &mut backup)?;
    backup.as_file().sync_all()?;
    drop(staged.persist(target)?);
    if let Err(error) = Command::new(target).spawn() {
        drop(backup.persist(target)?);
        return Err(error);
    }
    Ok(())
}

fn main() {
    if let Err(error) = replace() {
        rfd::MessageDialog::new()
            .set_title("Rust Player update")
            .set_description(error.to_string())
            .show();
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn restart_failure_restores_previous_executable() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let target = dir.path().join("player.exe");
        fs::write(&target, b"previous version")?;
        let mut staged = tempfile::NamedTempFile::new_in(dir.path())?;
        staged.write_all(b"invalid executable")?;
        assert!(install(staged, &target).is_err());
        assert_eq!(fs::read(&target)?, b"previous version");
        Ok(())
    }
}
