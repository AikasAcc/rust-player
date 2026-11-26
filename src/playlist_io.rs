use crate::library::{Playlist, Sort, supported};
use std::{
    fs,
    io::{self, Write},
    path::{Component, Path, PathBuf},
};

pub const EXTENSIONS: &[&str] = &["m3u", "m3u8", "pls"];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PathMode {
    #[default]
    Absolute,
    Relative,
}

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
            if crate::cue::is_sheet(&track) {
                match crate::cue::entries(&track) {
                    Ok(entries) => tracks.extend(entries),
                    Err(_) => skipped += 1,
                }
            } else {
                tracks.push(track);
            }
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
            ids: Vec::new(),
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
    supported(&path).then(|| lexical(&path))
}

fn absolute(path: &Path) -> io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn lexical(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}

/// Lexical relative path, so it also works when the playlist is not written yet.
fn relative_to(path: &Path, base: &Path) -> Option<PathBuf> {
    let path = lexical(path);
    let base = lexical(base);
    // A different drive or UNC share cannot be expressed as a relative path.
    if path.components().next() != base.components().next() || path.has_root() != base.has_root() {
        return None;
    }
    let mut left = path.components().peekable();
    let mut right = base.components().peekable();
    while left.peek().is_some_and(|part| right.peek() == Some(part)) {
        left.next();
        right.next();
    }
    let mut result = PathBuf::new();
    for _ in right {
        result.push("..");
    }
    for part in left {
        result.push(part.as_os_str());
    }
    (!result.as_os_str().is_empty()).then_some(result)
}

pub fn export(playlist: &Playlist, path: &Path, mode: PathMode) -> io::Result<()> {
    let dir = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let target_dir = absolute(dir)?;
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    writeln!(file, "#EXTM3U")?;
    for track in &playlist.tracks {
        // A CUE virtual track is written as its sheet so a re-import expands
        // the whole disc instead of skipping an extensionless path.
        let source = crate::cue::reference(track).map_or(track.as_path(), |(sheet, _)| sheet);
        let source = absolute(source)?;
        let entry = match mode {
            PathMode::Absolute => source,
            PathMode::Relative => relative_to(&source, &target_dir).unwrap_or(source),
        };
        writeln!(file, "{}", entry.display())?;
    }
    file.flush()?;
    file.persist(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn playlist(name: &str, contents: &[u8]) -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join(name), contents).unwrap();
        directory
    }

    #[test]
    fn m3u_import_ignores_comments_and_skips_remote_entries() {
        let directory = playlist(
            "list.m3u8",
            b"\xEF\xBB\xBF#EXTM3U\n# a comment\nfirst.flac\n  second.wav  \nhttps://example.com/stream.mp3\n",
        );
        let (loaded, skipped) = import(&directory.path().join("list.m3u8")).unwrap();
        assert_eq!(skipped, 1);
        assert_eq!(loaded.name, "list");
        assert_eq!(
            loaded.tracks,
            vec![
                directory.path().join("first.flac"),
                directory.path().join("second.wav")
            ]
        );
    }

    #[test]
    fn pls_entries_follow_their_numbers_not_the_file_order() {
        let directory = playlist(
            "list.pls",
            b"[playlist]\nFile2=second.wav\nFile1=first.flac\nNumberOfEntries=2\n",
        );
        let (loaded, skipped) = import(&directory.path().join("list.pls")).unwrap();
        assert_eq!(skipped, 0);
        assert_eq!(
            loaded.tracks,
            vec![
                directory.path().join("first.flac"),
                directory.path().join("second.wav")
            ]
        );
    }

    #[test]
    fn a_cue_entry_expands_to_virtual_tracks() {
        let directory = playlist("disc.m3u", b"album.cue\n");
        fs::write(
            directory.path().join("album.cue"),
            b"FILE \"disc.flac\" WAVE\nTRACK 01 AUDIO\nINDEX 01 00:00:00\nTRACK 02 AUDIO\nINDEX 01 01:00:00\n",
        )
        .unwrap();
        let (loaded, skipped) = import(&directory.path().join("disc.m3u")).unwrap();
        assert_eq!(skipped, 0);
        assert_eq!(loaded.tracks.len(), 2);
        assert_eq!(
            loaded.tracks[0],
            directory.path().join("album.cue").join("1")
        );
        assert_eq!(
            loaded.tracks[1],
            directory.path().join("album.cue").join("2")
        );
    }

    #[test]
    fn exported_playlist_round_trips() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("saved.m3u8");
        let tracks = vec![
            directory.path().join("a.flac"),
            directory.path().join("b.wav"),
        ];
        let source = Playlist {
            ids: Vec::new(),
            name: "saved".into(),
            tracks: tracks.clone(),
            pinned: false,
            sidebar_position: None,
            sort: Sort::Added,
            descending: false,
        };
        export(&source, &path, PathMode::Absolute).unwrap();
        let (loaded, skipped) = import(&path).unwrap();
        assert_eq!(skipped, 0);
        assert_eq!(loaded.tracks, tracks);
    }

    #[test]
    fn relative_export_round_trips_from_the_playlist_directory() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("saved.m3u8");
        let tracks = vec![
            directory.path().join("a.flac"),
            directory.path().join("nested/b.wav"),
        ];
        let source = Playlist {
            ids: Vec::new(),
            name: "saved".into(),
            tracks: tracks.clone(),
            pinned: false,
            sidebar_position: None,
            sort: Sort::Added,
            descending: false,
        };
        export(&source, &path, PathMode::Relative).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("a.flac"));
        assert!(!text.contains(&directory.path().display().to_string()));
        let (loaded, skipped) = import(&path).unwrap();
        assert_eq!(skipped, 0);
        assert_eq!(loaded.tracks, tracks);
    }

    #[test]
    fn relative_export_climbs_out_of_a_subdirectory() {
        let directory = tempfile::tempdir().unwrap();
        let playlists = directory.path().join("playlists");
        fs::create_dir(&playlists).unwrap();
        let path = playlists.join("saved.m3u");
        let source = Playlist {
            ids: Vec::new(),
            name: "saved".into(),
            tracks: vec![directory.path().join("music/a.flac")],
            pinned: false,
            sidebar_position: None,
            sort: Sort::Added,
            descending: false,
        };
        export(&source, &path, PathMode::Relative).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains(".."));
        let (loaded, skipped) = import(&path).unwrap();
        assert_eq!(skipped, 0);
        assert_eq!(loaded.tracks, source.tracks);
    }
}
