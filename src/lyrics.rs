use serde::Deserialize;
use std::{
    io::{self, Write},
    path::Path,
    time::Duration,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    track_name: String,
    artist_name: String,
    album_name: Option<String>,
    duration: Option<f64>,
    instrumental: bool,
    synced_lyrics: Option<String>,
    plain_lyrics: Option<String>,
}

pub fn lookup(
    title: &str,
    artist: &str,
    album: &str,
    duration: Option<Duration>,
) -> Result<Option<String>, String> {
    if title.trim().is_empty() || artist.trim().is_empty() {
        return Err("Online search needs a title and artist tag.".into());
    }
    let entries: Vec<Entry> = crate::artwork::https_agent()
        .get("https://lrclib.net/api/search")
        .header(
            "User-Agent",
            concat!("RustPlayer/", env!("CARGO_PKG_VERSION")),
        )
        .query("track_name", title)
        .query("artist_name", artist)
        .call()
        .map_err(|error| format!("LRCLIB search failed: {error}"))?
        .body_mut()
        .with_config()
        .limit(2 * 1024 * 1024)
        .read_json()
        .map_err(|error| format!("Could not read LRCLIB results: {error}"))?;

    Ok(best_match(entries, title, artist, album, duration))
}

fn best_match(
    entries: Vec<Entry>,
    title: &str,
    artist: &str,
    album: &str,
    duration: Option<Duration>,
) -> Option<String> {
    let normalize = |value: &str| value.trim().to_lowercase();
    let (title, artist, album) = (normalize(title), normalize(artist), normalize(album));
    entries
        .into_iter()
        .filter_map(|entry| {
            if entry.instrumental
                || normalize(&entry.track_name) != title
                || normalize(&entry.artist_name) != artist
            {
                return None;
            }
            let same_album = !album.is_empty()
                && entry
                    .album_name
                    .as_deref()
                    .is_some_and(|name| normalize(name) == album);
            let difference = duration
                .zip(entry.duration)
                .map(|(known, found)| (known.as_secs_f64() - found).abs());
            let same_duration = difference.is_some_and(|delta| delta <= 3.0);
            let different_duration = difference.is_some_and(|delta| delta > 3.0);
            let timed = entry.synced_lyrics.filter(|text| !text.trim().is_empty());
            let synced = timed.is_some() && !different_duration && (same_album || same_duration);
            let text = if synced { timed } else { entry.plain_lyrics }
                .filter(|text| !text.trim().is_empty())?;
            Some(((synced, same_duration, same_album), text))
        })
        .max_by_key(|(rank, _)| *rank)
        .map(|(_, text)| text)
}

pub fn save(path: &Path, text: &str) -> io::Result<()> {
    let dir = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    file.write_all(text.as_bytes())?;
    file.flush()?;
    file.persist_noclobber(path)?;
    Ok(())
}
