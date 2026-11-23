// Rust Player plugin API v1: return a line for the current track.
fn on_track(title, artist, album) {
    let performer = if artist == "" { "Unknown artist" } else { artist };
    let record = if album == "" { "Unknown album" } else { album };
    performer + " / " + record + " / " + title
}
