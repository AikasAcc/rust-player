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
    collection_name: Option<String>,
    artwork_url100: Option<String>,
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

pub fn lookup(title: &str, artist: &str, album: &str, download_image: bool) -> Option<Artwork> {
    let agent = https_agent();
    let term = format!("{artist} {title}");
    let search: Search = agent
        .get("https://itunes.apple.com/search")
        .query("term", term.as_str())
        .query("entity", "song")
        .query("limit", "25")
        .call()
        .ok()?
        .body_mut()
        .with_config()
        .limit(1024 * 1024)
        .read_json()
        .ok()?;
    let title = normalize(title);
    let artist = normalize(artist);
    let album = normalize(album);
    let found = search.results.into_iter().find(|item| {
        item.track_name
            .as_deref()
            .is_some_and(|value| normalize(value) == title)
            && item
                .artist_name
                .as_deref()
                .is_some_and(|value| normalize(value) == artist)
            && (album.is_empty()
                || item
                    .collection_name
                    .as_deref()
                    .is_some_and(|value| normalize(value) == album))
    })?;
    let url = found.artwork_url100?.replacen("http://", "https://", 1);
    let uri: ureq::http::Uri = url.parse().ok()?;
    if uri.scheme_str() != Some("https")
        || !uri
            .host()
            .is_some_and(|host| host.ends_with(".mzstatic.com"))
    {
        return None;
    }
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
    Some(Artwork { url, image })
}

fn normalize(value: &str) -> String {
    value.trim().to_lowercase()
}
