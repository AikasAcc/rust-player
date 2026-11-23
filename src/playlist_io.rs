use crate::library::{Playlist, Sort, supported};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

pub const EXTENSIONS: &[&str] = &["m3u", "m3u8", "pls"];

pub fn import(path: &Path) -> io::Result<(Playlist, usize)> {
    let bytes = fs::read(path)?;
    let (text, _, _) = match encoding_rs::Encoding::for_bom(&bytes) {
        Some((encoding, _)) => encoding.decode(&bytes),
        None if std::str::from_utf8(&bytes).is_ok() => encoding_rs::UTF_8.decode(&bytes),
        None => encoding_rs::WINDOWS_1252.decode(&bytes),
    };
    let is_pls = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("pls"));
    let entries = if is_pls {
        let mut entries: Vec<_> = text
            .lines()
            .filter_map(|line| {
                let (key, value) = line.trim().split_once('=')?;
                let key = key.trim().to_ascii_lowercase();
                let number = key.strip_prefix("file")?.parse::<usize>().ok()?;
                Some((number, value.trim()))
            })
            .collect();
        entries.sort_by_key(|(number, _)| *number);
        entries.into_iter().map(|(_, value)| value).collect()
    } else {
        text.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .collect::<Vec<_>>()
    };
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    let mut tracks = Vec::new();
    let mut skipped = 0;
    for entry in entries {
        if let Some(track) = local_track(base, entry) {
            tracks.push(track);
        } else {
            skipped += 1;
        }
    }
    let name = path
        .file_stem()
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Imported playlist".into());
    Ok((
        Playlist {
            name,
            tracks,
            pinned: false,
            sidebar_position: None,
            sort: Sort::Added,
            descending: false,
        },
        skipped,
    ))
}

fn local_track(base: &Path, entry: &str) -> Option<PathBuf> {
    let entry = entry
        .strip_prefix('"')
        .and_then(|text| text.strip_suffix('"'))
        .unwrap_or(entry);
    if entry.is_empty() || entry.contains('\0') {
        return None;
    }
    let path = if entry
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("file:"))
    {
        url::Url::parse(entry).ok()?.to_file_path().ok()?
    } else if entry.contains("://") {
        return None;
    } else {
        base.join(entry)
    };
    supported(&path).then_some(path)
}

pub fn export(playlist: &Playlist, path: &Path) -> io::Result<()> {
    let dir = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    writeln!(file, "#EXTM3U")?;
    for track in &playlist.tracks {
        let track = if track.is_absolute() {
            track.clone()
        } else {
            std::env::current_dir()?.join(track)
        };
        writeln!(file, "{}", track.display())?;
    }
    file.flush()?;
    file.persist(path)?;
    Ok(())
}
