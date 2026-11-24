use serde::Deserialize;
use std::time::Duration;

#[derive(Deserialize)]
struct Search {
    results: Vec<Match>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Match {
    track_name: Option<String>,
    artist_name: Option<String>,
    artist_id: Option<u64>,
    collection_name: Option<String>,
    artwork_url100: Option<String>,
    track_time_millis: Option<u64>,
}

pub struct Artwork {
    pub url: String,
    pub image: Option<Vec<u8>>,
}

pub(crate) fn https_agent() -> ureq::Agent {
    use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

    ureq::Agent::config_builder()
        .https_only(true)
        .timeout_global(Some(Duration::from_secs(8)))
        .tls_config(
            TlsConfig::builder()
                .provider(TlsProvider::NativeTls)
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .new_agent()
}

pub fn lookup(
    title: &str,
    artist: &str,
    album: &str,
    duration: Option<Duration>,
    download_image: bool,
) -> Result<Option<Artwork>, &'static str> {
    let agent = https_agent();
    let term = format!("{artist} {title}");
    let songs = query(&agent, "search", "term", &term, "song", "25")?;
    let plain_artist = artist.replace('$', "s");
    let mut found = song_cover(songs, title, artist, album, duration);
    if found.is_none() {
        // Some releases exist in the artist catalog but are absent from text search.
        let names = query(&agent, "search", "term", &plain_artist, "musicArtist", "25")?;
        let key = normalize(artist);
        let id = names
            .into_iter()
            .filter(|item| same(item.artist_name.as_deref(), &key))
            .filter_map(|item| {
                Some((
                    !item.artist_name?.trim().eq_ignore_ascii_case(artist.trim()),
                    item.artist_id?,
                ))
            })
            .min_by_key(|(alias, _)| *alias)
            .map(|(_, id)| id.to_string());
        if let Some(id) = id {
            let catalog = query(&agent, "lookup", "id", &id, "song", "200")?;
            found = song_cover(catalog, title, artist, album, duration);
            if found.is_none() && !album.trim().is_empty() {
                let catalog = query(&agent, "lookup", "id", &id, "album", "200")?;
                found = album_cover(catalog, artist, album);
            }
        }
    }
    if found.is_none() && !album.trim().is_empty() {
        let term = format!("{plain_artist} {album}");
        let albums = query(&agent, "search", "term", &term, "album", "25")?;
        found = album_cover(albums, artist, album);
    }
    let Some(url) = found else { return Ok(None) };
    let image = if download_image {
        agent.get(&url).call().ok().and_then(|mut response| {
            response
                .body_mut()
                .with_config()
                .limit(2 * 1024 * 1024)
                .read_to_vec()
                .ok()
        })
    } else {
        None
    };
    Ok(Some(Artwork { url, image }))
}

fn query(
    agent: &ureq::Agent,
    endpoint: &str,
    key: &str,
    value: &str,
    entity: &str,
    limit: &str,
) -> Result<Vec<Match>, &'static str> {
    let search: Search = agent
        .get(format!("https://itunes.apple.com/{endpoint}"))
        .query(key, value)
        .query("entity", entity)
        .query("limit", limit)
        .call()
        .map_err(|_| "Apple Music artwork lookup could not connect.")?
        .body_mut()
        .with_config()
        .limit(1024 * 1024)
        .read_json()
        .map_err(|_| "Apple Music returned an unreadable artwork list.")?;
    Ok(search.results)
}

fn album_cover(items: Vec<Match>, artist: &str, album: &str) -> Option<String> {
    let artist = normalize(artist);
    let album = album_key(album);
    items
        .into_iter()
        .filter(|item| {
            same(item.artist_name.as_deref(), &artist)
                && item
                    .collection_name
                    .as_deref()
                    .is_some_and(|name| album_key(name) == album)
        })
        .find_map(|item| image_url(item.artwork_url100.as_deref()?))
}

fn song_cover(
    items: Vec<Match>,
    title: &str,
    artist: &str,
    album: &str,
    duration: Option<Duration>,
) -> Option<String> {
    let title = normalize(title);
    let artist = normalize(artist);
    let album = album_key(album);
    items
        .into_iter()
        .filter(|item| {
            same(item.track_name.as_deref(), &title) && same(item.artist_name.as_deref(), &artist)
        })
        .filter_map(|item| {
            let url = image_url(item.artwork_url100.as_deref()?)?;
            let album_match = !album.is_empty()
                && item
                    .collection_name
                    .as_deref()
                    .is_some_and(|name| album_key(name) == album);
            let distance = duration
                .zip(item.track_time_millis)
                .map(|(duration, millis)| duration.abs_diff(Duration::from_millis(millis)));
            // Keep the exact song/artist match when a local tag names a different release.
            if !album_match && distance.is_some_and(|delta| delta > Duration::from_secs(12)) {
                return None;
            }
            Some(((!album_match, distance.unwrap_or(Duration::MAX)), url))
        })
        .min_by_key(|(rank, _)| *rank)
        .map(|(_, url)| url)
}

fn image_url(raw: &str) -> Option<String> {
    let url = raw.replacen("http://", "https://", 1);
    let uri: ureq::http::Uri = url.parse().ok()?;
    if uri.scheme_str() != Some("https")
        || !uri
            .host()
            .is_some_and(|host| host.ends_with(".mzstatic.com"))
    {
        return None;
    }
    Some(url)
}

fn same(value: Option<&str>, key: &str) -> bool {
    !key.is_empty() && value.is_some_and(|value| normalize(value) == key)
}

fn album_key(value: &str) -> String {
    let value = value.trim().to_lowercase();
    normalize(
        value
            .strip_suffix(" - ep")
            .or_else(|| value.strip_suffix(" - single"))
            .unwrap_or(&value),
    )
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .map(|character| if character == '$' { 's' } else { character })
        .filter(|character| character.is_alphanumeric())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_song_and_artist_can_supply_artwork_when_release_tags_differ() {
        let results = || {
            serde_json::from_str::<Search>(r#"{"results":[{"trackName":"Styrofoam","artistName":"Suicideboys","collectionName":"The Coast of Ashes Saga - EP","trackTimeMillis":82000,"artworkUrl100":"https://is1-ssl.mzstatic.com/image/cover.jpg"}]}"#).unwrap().results
        };
        let duration = Some(Duration::from_secs(82));
        assert_eq!(
            song_cover(
                results(),
                "Styrofoam",
                "$uicideboy$",
                "Local album",
                duration
            )
            .as_deref(),
            Some("https://is1-ssl.mzstatic.com/image/cover.jpg")
        );
        assert!(
            song_cover(
                results(),
                "Styrofoam",
                "Another artist",
                "Local album",
                duration
            )
            .is_none()
        );
        assert!(
            song_cover(
                results(),
                "Styrofoam",
                "$uicideboy$",
                "Local album",
                Some(Duration::from_secs(150))
            )
            .is_none()
        );
    }
}
