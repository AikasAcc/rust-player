use std::path::Path;

use lofty::{
    config::WriteOptions,
    file::{AudioFile, TaggedFileExt},
    tag::{ItemKey, Tag},
};

/// The five fields the properties dialog can edit.
#[derive(Clone, Copy, Debug, Default)]
pub struct Edit<'a> {
    pub title: &'a str,
    pub artist: &'a str,
    pub album: &'a str,
    pub album_artist: &'a str,
    pub track: &'a str,
}

/// Fields for a batch edit. `None` keeps the existing value, `Some("")` clears
/// it, and any other value is written. Unlike [`Edit`], the batch writer also
/// supports genre and year, which are the most useful mass-tagging fields.
#[derive(Clone, Copy, Debug, Default)]
pub struct BatchEdit<'a> {
    pub title: Option<&'a str>,
    pub artist: Option<&'a str>,
    pub album: Option<&'a str>,
    pub album_artist: Option<&'a str>,
    pub genre: Option<&'a str>,
    pub year: Option<&'a str>,
}

/// Write only the selected fields into the original file. CUE entries are
/// virtual tracks and must be edited through their source audio file.
pub fn write_batch(path: &Path, edit: BatchEdit<'_>) -> Result<(), String> {
    if crate::cue::reference(path).is_some() {
        return Err("Edit the source audio file instead of a CUE track".into());
    }
    if let Some(year) = edit.year
        && !year.trim().is_empty()
        && !year.trim().parse::<u16>().is_ok_and(|year| year > 0)
    {
        return Err("Year must be a positive number".into());
    }
    let mut file = lofty::read_from_path(path).map_err(|error| error.to_string())?;
    if file.primary_tag().is_none() {
        let tag_type = file.primary_tag_type();
        file.insert_tag(Tag::new(tag_type));
    }
    let tag = file
        .primary_tag_mut()
        .ok_or_else(|| "The file has no writable tag".to_string())?;
    let mut apply = |key: ItemKey, value: Option<&str>| {
        let Some(value) = value else { return };
        let value = value.trim();
        if value.is_empty() {
            tag.remove_key(key);
        } else {
            tag.insert_text(key, value.to_owned());
        }
    };
    apply(ItemKey::TrackTitle, edit.title);
    apply(ItemKey::TrackArtist, edit.artist);
    apply(ItemKey::AlbumTitle, edit.album);
    apply(ItemKey::AlbumArtist, edit.album_artist);
    apply(ItemKey::Genre, edit.genre);
    // ID3v2.4 stores dates in TDRC; `TrackInfo::read` also prefers this key.
    apply(ItemKey::RecordingDate, edit.year);
    file.save_to_path(path, WriteOptions::default())
        .map_err(|error| error.to_string())
}

/// Write tags into the original file. Empty text fields remove an existing
/// value; the track number must stay a positive integer. CUE entries are
/// virtual tracks, so their source audio file must be edited instead.
pub fn write(path: &Path, edit: Edit<'_>) -> Result<(), String> {
    if crate::cue::reference(path).is_some() {
        return Err("Edit the source audio file instead of a CUE track".into());
    }
    if !edit
        .track
        .trim()
        .parse::<u32>()
        .is_ok_and(|number| number > 0)
    {
        return Err("Track must be a positive number".into());
    }
    let mut file = lofty::read_from_path(path).map_err(|error| error.to_string())?;
    if file.primary_tag().is_none() {
        let tag_type = file.primary_tag_type();
        file.insert_tag(Tag::new(tag_type));
    }
    let tag = file
        .primary_tag_mut()
        .ok_or_else(|| "The file has no writable tag".to_string())?;
    let mut set = |key: ItemKey, value: &str| {
        let value = value.trim();
        if value.is_empty() {
            tag.remove_key(key);
        } else {
            tag.insert_text(key, value.to_owned());
        }
    };
    set(ItemKey::TrackTitle, edit.title);
    set(ItemKey::TrackArtist, edit.artist);
    set(ItemKey::AlbumTitle, edit.album);
    set(ItemKey::AlbumArtist, edit.album_artist);
    set(ItemKey::TrackNumber, edit.track);
    file.save_to_path(path, WriteOptions::default())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lofty::tag::Accessor;

    #[test]
    fn writes_and_removes_tags_without_moving_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("song.wav");
        crate::audio::setup::tone(&path, 48_000).unwrap();
        write(
            &path,
            Edit {
                title: "New title",
                artist: "New artist",
                album: "New album",
                album_artist: "New album artist",
                track: "7",
            },
        )
        .unwrap();

        let file = lofty::read_from_path(&path).unwrap();
        let tag = file.primary_tag().unwrap();
        assert_eq!(tag.title().unwrap(), "New title");
        assert_eq!(tag.artist().unwrap(), "New artist");
        assert_eq!(tag.album().unwrap(), "New album");
        assert_eq!(
            tag.get_string(ItemKey::AlbumArtist),
            Some("New album artist")
        );
        assert_eq!(tag.track(), Some(7));

        write(&path, Edit::default()).unwrap_err();
        write(
            &path,
            Edit {
                track: "7",
                ..Edit::default()
            },
        )
        .unwrap();
        let file = lofty::read_from_path(&path).unwrap();
        let tag = file.primary_tag().unwrap();
        assert!(tag.title().is_none());
        assert!(tag.artist().is_none());
    }

    #[test]
    fn refuses_zero_and_negative_track_numbers() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("song.wav");
        crate::audio::setup::tone(&path, 48_000).unwrap();
        for invalid in ["0", "-1", "abc", ""] {
            let error = write(
                &path,
                Edit {
                    track: invalid,
                    ..Edit::default()
                },
            )
            .unwrap_err();
            assert!(error.contains("positive"), "accepted track {invalid:?}");
        }
    }

    #[test]
    fn batch_edit_only_touches_selected_fields() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("song.wav");
        crate::audio::setup::tone(&path, 48_000).unwrap();
        write(
            &path,
            Edit {
                title: "Original",
                artist: "Keep me",
                album: "Original album",
                album_artist: "Keep me",
                track: "3",
            },
        )
        .unwrap();

        write_batch(
            &path,
            BatchEdit {
                album: Some("New album"),
                genre: Some("Jazz"),
                year: Some("1959"),
                ..BatchEdit::default()
            },
        )
        .unwrap();
        let file = lofty::read_from_path(&path).unwrap();
        let tag = file.primary_tag().unwrap();
        assert_eq!(tag.title().unwrap(), "Original");
        assert_eq!(tag.artist().unwrap(), "Keep me");
        assert_eq!(tag.album().unwrap(), "New album");
        assert_eq!(tag.get_string(ItemKey::Genre), Some("Jazz"));
        assert_eq!(tag.date().map(|date| date.year), Some(1959));

        write_batch(
            &path,
            BatchEdit {
                genre: Some(""),
                ..BatchEdit::default()
            },
        )
        .unwrap();
        let file = lofty::read_from_path(&path).unwrap();
        assert!(
            file.primary_tag()
                .unwrap()
                .get_string(ItemKey::Genre)
                .is_none()
        );
    }

    #[test]
    fn batch_edit_validates_year_and_cue_tracks() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("song.wav");
        crate::audio::setup::tone(&path, 48_000).unwrap();
        for invalid in ["0", "abcd", "-1"] {
            let error = write_batch(
                &path,
                BatchEdit {
                    year: Some(invalid),
                    ..BatchEdit::default()
                },
            )
            .unwrap_err();
            assert!(error.contains("Year must be"), "accepted year {invalid:?}");
        }
        let error = write_batch(
            Path::new("Album/album.cue/1"),
            BatchEdit {
                genre: Some("Jazz"),
                ..BatchEdit::default()
            },
        )
        .unwrap_err();
        assert!(error.contains("source audio"));
    }

    #[test]
    fn refuses_to_edit_a_virtual_cue_track() {
        let error = write(
            Path::new("Album/album.cue/1"),
            Edit {
                track: "1",
                ..Edit::default()
            },
        )
        .unwrap_err();
        assert!(error.contains("source audio"));
    }
}
