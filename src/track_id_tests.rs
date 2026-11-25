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
    saved.id = Some(lib.playlists[original.0].ids[original.1]);
    let distinct = lib.playlists[1].ids[1];
    let playlist = &mut lib.playlists[1];
    playlist.ids.swap(0, 2);
    playlist.tracks.swap(0, 2);
    playlist.sort = Sort::Title;
    lib.playlists.remove(0);
    lib.rebuild_locations();
    assert_eq!(saved.resolve(&lib), Some((0, 0)));
    assert_eq!(lib.locations[&distinct], (0, 1));
    lib.playlists[0].tracks.remove(0);
    lib.playlists[0].ids.remove(0);
    lib.rebuild_locations();
    assert!(!lib.locations.contains_key(&saved.id.unwrap()));
    assert_eq!(lib.locations[&distinct], (0, 0));
}
