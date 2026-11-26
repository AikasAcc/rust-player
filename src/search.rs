//! Small foobar-style query language for playlist filtering.
//!
//! Plain words are ANDed and match any indexed field or the file path.
//! Field filters (`title:`, `artist:`, `album:`, `albumartist:`, `path:`,
//! `file:`, `missing:`), quoted phrases, `OR`, `NOT` and a leading `-` are
//! supported. This is pure UI logic; no decoder or audio code is involved.

use crate::library::TrackInfo;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Any,
    Title,
    Artist,
    Album,
    AlbumArtist,
    Path,
    Filename,
    Missing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Clause {
    field: Field,
    value: String,
    negate: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    /// OR-ed groups; clauses inside a group are AND-ed.
    groups: Vec<Vec<Clause>>,
}

fn tokens(input: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for character in input.chars() {
        match character {
            '"' => quoted = !quoted,
            character if character.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    result.push(std::mem::take(&mut current));
                }
            }
            character => current.push(character),
        }
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

fn parse_clause(token: &str) -> (Field, String, bool) {
    let (token, leading_not) = match token.strip_prefix('-') {
        Some(rest) => (rest, true),
        None => (token, false),
    };
    if let Some((name, value)) = token.split_once(':') {
        let field = match name.to_ascii_lowercase().as_str() {
            "title" => Field::Title,
            "artist" => Field::Artist,
            "album" => Field::Album,
            "albumartist" | "album_artist" => Field::AlbumArtist,
            "path" => Field::Path,
            "file" | "filename" => Field::Filename,
            "missing" => Field::Missing,
            _ => Field::Any,
        };
        if field != Field::Any {
            let value = value
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .unwrap_or(value);
            return (field, value.to_owned(), leading_not);
        }
    }
    (Field::Any, token.to_owned(), leading_not)
}

impl Query {
    pub fn compile(input: &str) -> Self {
        let mut groups = vec![Vec::new()];
        let mut negate = false;
        for token in tokens(input) {
            if token.eq_ignore_ascii_case("OR") {
                groups.push(Vec::new());
                negate = false;
                continue;
            }
            if token.eq_ignore_ascii_case("AND") || token == "&&" {
                continue;
            }
            if token.eq_ignore_ascii_case("NOT") {
                negate = !negate;
                continue;
            }
            let (field, value, leading_not) = parse_clause(&token);
            if value.is_empty() {
                continue;
            }
            groups.last_mut().expect("at least one group").push(Clause {
                field,
                value: value.to_lowercase(),
                negate: negate ^ leading_not,
            });
            negate = false;
        }
        groups.retain(|group| !group.is_empty());
        Self { groups }
    }

    /// `track` is `None` for rows whose metadata has not been indexed yet;
    /// those are still searchable by path and file name.
    pub fn matches(&self, track: Option<&TrackInfo>, path: &Path) -> bool {
        if self.groups.is_empty() {
            return true;
        }
        self.groups
            .iter()
            .any(|group| group.iter().all(|clause| clause.matches(track, path)))
    }
}

impl Clause {
    fn matches(&self, track: Option<&TrackInfo>, path: &Path) -> bool {
        let contains = |value: &str| value.to_lowercase().contains(&self.value);
        let found = match self.field {
            Field::Title => track.is_some_and(|track| contains(&track.title)),
            Field::Artist => track.is_some_and(|track| contains(&track.artist)),
            Field::Album => track.is_some_and(|track| contains(&track.album)),
            Field::AlbumArtist => track.is_some_and(|track| contains(&track.album_artist)),
            Field::Path => contains(&path.to_string_lossy()),
            Field::Filename => path
                .file_name()
                .is_some_and(|name| contains(&name.to_string_lossy())),
            Field::Missing => {
                let wanted = matches!(self.value.as_str(), "true" | "yes" | "1");
                track.is_some_and(|track| track.missing) == wanted
            }
            Field::Any => {
                track.is_some_and(|track| track.search_key.contains(&self.value))
                    || contains(&path.to_string_lossy())
            }
        };
        found != self.negate
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn track() -> TrackInfo {
        let mut info = TrackInfo {
            title: "Help!".into(),
            artist: "The Beatles".into(),
            album: "Help!".into(),
            album_artist: "The Beatles".into(),
            missing: false,
            ..Default::default()
        };
        info.prepare_search(Path::new("Beatles/Help.flac"));
        info
    }

    #[test]
    fn an_empty_query_matches_everything() {
        let query = Query::compile("   ");
        assert!(query.matches(Some(&track()), Path::new("any.flac")));
    }

    #[test]
    fn plain_words_are_anded_across_fields() {
        let query = Query::compile("beatles help");
        assert!(query.matches(Some(&track()), Path::new("Beatles/Help.flac")));
        let mut other = TrackInfo {
            title: "Other".into(),
            album: "Other".into(),
            ..track()
        };
        other.prepare_search(Path::new("Beatles/Other.flac"));
        assert!(!query.matches(Some(&other), Path::new("Beatles/Other.flac")));
        // Rows without metadata remain searchable by path.
        assert!(query.matches(None, Path::new("Beatles/Help.flac")));
    }

    #[test]
    fn fields_quotes_and_negation_are_exact() {
        let query = Query::compile("artist:\"the beatles\" -album:live");
        assert!(query.matches(Some(&track()), Path::new("x.flac")));
        assert!(Query::compile("title:help AND").matches(Some(&track()), Path::new("x.flac")));

        let missing = TrackInfo {
            missing: true,
            ..track()
        };
        assert!(Query::compile("missing:true").matches(Some(&missing), Path::new("x.flac")));
        assert!(!Query::compile("missing:true").matches(Some(&track()), Path::new("x.flac")));
        assert!(Query::compile("file:help.flac").matches(None, Path::new("a/Help.flac")));
    }

    #[test]
    fn or_groups_and_not_work_without_parentheses() {
        let query = Query::compile("artist:beatles OR file:other.flac");
        assert!(query.matches(Some(&track()), Path::new("x.flac")));
        assert!(query.matches(None, Path::new("a/OTHER.flac")));
        assert!(!query.matches(None, Path::new("a/unrelated.flac")));

        let query = Query::compile("NOT artist:beatles");
        assert!(!query.matches(Some(&track()), Path::new("x.flac")));
        assert!(query.matches(Some(&TrackInfo::default()), Path::new("x.flac")));
    }
}
