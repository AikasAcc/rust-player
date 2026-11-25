use super::*;

#[test]
fn old_save_resolves_and_ids_follow_duplicates_across_library_edits() {
    let mut lib = Library::default();
    let p = &mut lib.playlists[0];
    p.tracks = vec!["b.wav".into(), "same.wav".into(), "same.wav".into()];
    let copy = p.clone();
    lib.playlists.push(copy);
    lib.rebuild_locations();
    let mut saved: SavedTrack = serde_json::from_str(
        r#"{"track":[1,2],"path":"same.wav","position":{"secs":4,"nanos":0}}"#,
    )
    .unwrap();
    let original = saved.resolve(&lib).unwrap();
    saved.id = Some(original);
    let distinct = lib.playlists[1].ids[1];
    let playlist = &mut lib.playlists[1];
    playlist.ids.swap(0, 2);
    playlist.tracks.swap(0, 2);
    playlist.sort = Sort::Title;
    lib.playlists.remove(0);
    lib.rebuild_locations();
    let remapped = saved.resolve(&lib).unwrap();
    assert_eq!(remapped, original);
    assert_eq!(lib.locate(remapped), Some((0, 0)));
    assert_eq!(lib.locations[&distinct], (0, 1));
    lib.playlists[0].tracks.remove(0);
    lib.playlists[0].ids.remove(0);
    lib.rebuild_locations();
    assert!(!lib.locations.contains_key(&saved.id.unwrap()));
    assert_eq!(lib.locations[&distinct], (0, 0));
}

#[test]
fn playback_round_trip_via_save_json() -> std::io::Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("playback.json");
    let saved = SavedTrack {
        id: Some(TrackId(42)),
        track: (0, 0),
        path: PathBuf::from("song.flac"),
        position: Duration::from_millis(1234),
    };

    library::save_json(&Some(saved), &path)?;
    let loaded: Option<SavedTrack> = serde_json::from_slice(&std::fs::read(&path)?)?;
    let loaded = loaded.expect("saved track");
    assert_eq!(loaded.id, Some(TrackId(42)));
    assert_eq!(loaded.track, (0, 0));
    assert_eq!(loaded.path, PathBuf::from("song.flac"));
    assert_eq!(loaded.position, Duration::from_millis(1234));

    library::save_json(&Option::<SavedTrack>::None, &path)?;
    let loaded: Option<SavedTrack> = serde_json::from_slice(&std::fs::read(&path)?)?;
    assert!(loaded.is_none());
    Ok(())
}

#[test]
fn path_comparison_follows_platform_rules() {
    let upper = std::path::Path::new(r"Music\SONG.wav");
    let lower = std::path::Path::new("music/song.wav");
    #[cfg(windows)]
    assert!(same_path(upper, lower));
    #[cfg(not(windows))]
    assert!(!same_path(upper, lower));
}

#[test]
fn old_save_matches_path_with_windows_case_and_separator_rules() {
    let mut lib = Library::default();
    lib.playlists[0].tracks = vec![PathBuf::from(r"Music\Song.wav")];
    lib.rebuild_locations();
    let saved = SavedTrack {
        id: None,
        track: (0, 0),
        path: PathBuf::from("music/song.WAV"),
        position: Duration::ZERO,
    };

    #[cfg(windows)]
    assert_eq!(saved.resolve(&lib), Some(lib.playlists[0].ids[0]));
    #[cfg(not(windows))]
    assert_eq!(saved.resolve(&lib), None);
}

#[test]
fn saved_playback_id_survives_rename_and_reorder() {
    let mut lib = Library::default();
    lib.playlists[0].tracks = vec!["a.wav".into(), "b.wav".into()];
    lib.rebuild_locations();
    let id = lib.playlists[0].ids[0];
    let saved = SavedTrack {
        id: Some(id),
        track: (0, 0),
        path: PathBuf::from("a.wav"),
        position: Duration::ZERO,
    };

    lib.playlists[0].tracks[0] = PathBuf::from("renamed.wav");
    lib.playlists[0].tracks.swap(0, 1);
    lib.playlists[0].ids.swap(0, 1);
    lib.rebuild_locations();

    assert_eq!(saved.resolve(&lib), Some(id));
    assert_eq!(lib.locate(id), Some((0, 1)));
    assert_eq!(
        lib.track_path(id),
        Some(std::path::Path::new("renamed.wav"))
    );
}

#[test]
fn remap_order_keeps_cursor_on_playing_entry_or_advances_to_next() {
    let mut lib = Library::default();
    lib.playlists[0].tracks = vec![
        "a.wav".into(),
        "b.wav".into(),
        "c.wav".into(),
        "d.wav".into(),
    ];
    lib.rebuild_locations();
    let ids = lib.playlists[0].ids.clone();
    let mut order = ids.clone();

    assert_eq!(remap_order(&mut order, &lib.locations, 2), 2);
    assert_eq!(order, ids);

    lib.playlists[0].tracks.remove(1);
    lib.playlists[0].ids.remove(1);
    lib.rebuild_locations();
    assert_eq!(remap_order(&mut order, &lib.locations, 2), 1);
    assert_eq!(order[1], ids[2]);

    lib.playlists[0].tracks.remove(1);
    lib.playlists[0].ids.remove(1);
    lib.rebuild_locations();
    assert_eq!(remap_order(&mut order, &lib.locations, 1), 1);
    assert_eq!(order[1], ids[3]);

    lib.playlists[0].tracks.clear();
    lib.playlists[0].ids.clear();
    lib.rebuild_locations();
    assert_eq!(remap_order(&mut order, &lib.locations, 1), usize::MAX);
    assert!(order.is_empty());
}

#[test]
fn duplicate_paths_and_cue_entries_get_distinct_stable_ids() {
    use std::collections::HashSet;

    let mut lib = Library::default();
    let sheet = PathBuf::from("Disc/disc.cue");
    lib.playlists[0].tracks = vec![
        PathBuf::from("same.wav"),
        PathBuf::from("same.wav"),
        PathBuf::from("a/same.wav"),
        sheet.join("1"),
        sheet.join("2"),
    ];
    lib.rebuild_locations();

    let ids = lib.playlists[0].ids.clone();
    let unique: HashSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len());
    for (index, &id) in ids.iter().enumerate() {
        assert_eq!(lib.locate(id), Some((0, index)));
        assert_eq!(lib.track_id((0, index)), Some(id));
    }
    assert_eq!(lib.track_path(ids[3]), Some(sheet.join("1").as_path()));
    assert_eq!(lib.track_path(ids[4]), Some(sheet.join("2").as_path()));
}

#[test]
fn reordering_keeps_ids_attached_to_tracks() {
    let mut lib = Library::default();
    lib.playlists[0].tracks = vec!["a.wav".into(), "b.wav".into(), "c.wav".into()];
    lib.rebuild_locations();
    let a = lib.playlists[0].ids[0];
    let b = lib.playlists[0].ids[1];
    let c = lib.playlists[0].ids[2];

    lib.playlists[0].ids.swap(0, 2);
    lib.playlists[0].tracks.swap(0, 2);
    lib.rebuild_locations();

    assert_eq!(lib.playlists[0].ids, vec![c, b, a]);
    assert_eq!(lib.locate(a), Some((0, 2)));
    assert_eq!(lib.locate(b), Some((0, 1)));
    assert_eq!(lib.track_path(a), Some(std::path::Path::new("a.wav")));
}

#[test]
fn favorites_appears_only_in_the_library_section() {
    let mut lib = Library::default();
    let mut second = lib.playlists[0].clone();
    second.name = "Second".into();
    second.ids.clear();
    lib.playlists.push(second);
    lib.rebuild_locations();
    let favorites = lib.favorites_index_or_create();

    assert!(lib.sidebar_order().contains(&favorites));
    let visible = lib.sidebar_playlists();
    assert!(!visible.contains(&favorites));
    assert!(visible.contains(&0));
    assert!(visible.contains(&1));
}

#[test]
fn sidebar_reorder_does_not_change_track_identity() {
    let mut lib = Library::default();
    lib.playlists[0].tracks = vec!["a.wav".into()];
    let mut second = lib.playlists[0].clone();
    second.name = "Second".into();
    second.tracks = vec!["b.wav".into()];
    second.ids.clear();
    lib.playlists.push(second);
    lib.rebuild_locations();
    let before: Vec<_> = lib.playlists.iter().flat_map(|p| p.ids.clone()).collect();

    assert!(lib.move_playlist(0, 1, true));

    let after: Vec<_> = lib.playlists.iter().flat_map(|p| p.ids.clone()).collect();
    assert_eq!(after, before);
    assert_eq!(lib.locate(before[0]), Some((0, 0)));
    assert_eq!(lib.locate(before[1]), Some((1, 0)));
}

#[test]
fn legacy_library_ids_are_persisted_on_first_save() -> std::io::Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("library.json");
    std::fs::write(
        &path,
        r#"{"playlists":[{"name":"Default","tracks":["a.wav","b.wav"]}],"selected":0}"#,
    )?;

    let lib = Library::load(&path)?;
    assert_eq!(lib.playlists[0].ids.len(), 2);
    lib.save(&path)?;
    let reloaded = Library::load(&path)?;

    assert_eq!(reloaded.playlists[0].ids, lib.playlists[0].ids);
    assert_eq!(reloaded.locate(lib.playlists[0].ids[1]), Some((0, 1)));
    Ok(())
}

#[test]
fn favorites_follow_source_id_across_rename_and_reorder() {
    let mut lib = Library::default();
    lib.playlists[0].tracks = vec!["a.wav".into(), "b.wav".into()];
    lib.rebuild_locations();
    let a = lib.playlists[0].ids[0];

    assert!(lib.toggle_favorite_id(a));
    assert!(lib.is_favorite_id(a));
    assert_eq!(lib.favorite_ids, [Some(a)]);

    lib.playlists[0].tracks[0] = PathBuf::from("renamed.wav");
    lib.rebuild_locations();
    lib.prune_favorites();
    assert!(lib.is_favorite_id(a));
    assert!(!lib.is_favorite(std::path::Path::new("a.wav")));
    assert!(lib.is_favorite(std::path::Path::new("renamed.wav")));

    lib.playlists[0].ids.swap(0, 1);
    lib.playlists[0].tracks.swap(0, 1);
    lib.rebuild_locations();
    lib.prune_favorites();
    assert_eq!(lib.locate(a), Some((0, 1)));
    assert_eq!(lib.track_path(a), Some(std::path::Path::new("renamed.wav")));
    assert!(lib.is_favorite_id(a));
}

#[test]
fn active_row_uses_identity_for_duplicates_and_favorites_copy() {
    let mut lib = Library::default();
    lib.playlists[0].tracks = vec!["a.wav".into(), "b.wav".into()];
    lib.rebuild_locations();
    let a = lib.playlists[0].ids[0];
    let b = lib.playlists[0].ids[1];
    assert!(lib.row_is_playing(0, 0, a));
    assert!(!lib.row_is_playing(0, 0, b));

    assert!(lib.toggle_favorite_id(a));
    let favorites = lib.favorites.unwrap();
    let copy = lib.playlists[favorites].ids[0];
    assert_ne!(copy, a);
    assert!(lib.row_is_playing(favorites, 0, a));
    assert!(lib.row_is_playing(0, 0, a));
    assert!(lib.is_favorite_id(copy));

    let duplicate_playlist = lib.playlists[0].clone();
    lib.playlists.push(duplicate_playlist);
    let list = lib.playlists.len() - 1;
    lib.playlists[list].ids.clear();
    lib.rebuild_locations();
    let duplicate = lib.playlists[list].ids[0];
    assert_ne!(duplicate, a);
    assert!(!lib.row_is_playing(list, 0, a));
    assert!(lib.row_is_playing(list, 0, duplicate));

    assert!(!lib.toggle_favorite_id(copy));
    assert!(!lib.is_favorite_id(a));
    assert!(!lib.is_favorite_id(copy));
}

#[test]
fn favorites_drop_when_source_track_is_deleted() {
    let mut lib = Library::default();
    lib.playlists[0].tracks = vec!["a.wav".into(), "b.wav".into()];
    lib.rebuild_locations();
    let a = lib.playlists[0].ids[0];
    let b = lib.playlists[0].ids[1];
    assert!(lib.toggle_favorite_id(a));
    assert!(lib.toggle_favorite_id(b));

    lib.playlists[0].ids.remove(0);
    lib.playlists[0].tracks.remove(0);
    lib.rebuild_locations();
    lib.prune_favorites();

    assert!(!lib.is_favorite_id(a));
    assert!(lib.is_favorite_id(b));
    assert_eq!(lib.favorite_ids, [Some(b)]);
    assert!(!lib.locations.contains_key(&a));
}

#[test]
fn legacy_favorites_migrate_by_path_to_stable_ids() -> std::io::Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("library.json");
    std::fs::write(
        &path,
        r#"{
            "favorites": 1,
            "selected": 0,
            "playlists": [
                {"name":"Default","tracks":["a.wav","b.wav"]},
                {"name":"Favorites","tracks":["b.wav"],"sort":"Album"}
            ]
        }"#,
    )?;

    let lib = Library::load(&path)?;
    let source = lib.playlists[0].ids[1];
    assert_eq!(lib.favorite_ids, [Some(source)]);
    assert!(lib.is_favorite_id(source));
    assert!(!lib.is_favorite_id(lib.playlists[0].ids[0]));
    Ok(())
}

#[test]
fn corrupt_playback_json_backup_preserves_bytes() -> std::io::Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("playback.json");
    let broken = b"{oops";
    std::fs::write(&path, broken)?;
    assert!(serde_json::from_slice::<Option<SavedTrack>>(broken).is_err());

    let backup = Library::back_up_invalid_json(&path, "playback.invalid-")?;
    assert_eq!(std::fs::read(&backup)?, broken);
    assert_eq!(std::fs::read(&path)?, broken);

    library::save_json(&Option::<SavedTrack>::None, &path)?;
    assert_eq!(std::fs::read(&backup)?, broken);
    assert_ne!(std::fs::read(&path)?, broken);
    Ok(())
}
