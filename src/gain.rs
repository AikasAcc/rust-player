use std::path::Path;

use lofty::{
    config::WriteOptions,
    file::{AudioFile, TaggedFileExt},
    tag::{ItemKey, Tag},
};

/// ReplayGain 2.0 reference loudness (EBU R128).
pub const REFERENCE_LUFS: f32 = -18.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Report {
    pub gain_db: f32,
    pub loudness_lufs: f32,
    pub peak: f32,
}

/// Measure a file and write track ReplayGain tags. This is an explicit user
/// action: the file itself is only changed in its metadata, never moved.
pub fn scan_and_write(path: &Path) -> Result<Report, String> {
    if crate::cue::reference(path).is_some() {
        return Err("Scan the source audio file instead of a CUE track".into());
    }
    let measured = crate::audio::loudness::measure(path)?;
    let gain_db = (REFERENCE_LUFS - measured.integrated_lufs).clamp(-60.0, 30.0);

    let mut file = lofty::read_from_path(path).map_err(|error| error.to_string())?;
    if file.primary_tag().is_none() {
        let tag_type = file.primary_tag_type();
        file.insert_tag(Tag::new(tag_type));
    }
    let tag = file
        .primary_tag_mut()
        .ok_or_else(|| "The file has no writable tag".to_string())?;
    tag.insert_text(ItemKey::ReplayGainTrackGain, format!("{gain_db:+.2} dB"));
    tag.insert_text(
        ItemKey::ReplayGainTrackPeak,
        format!("{:.6}", measured.peak),
    );
    file.save_to_path(path, WriteOptions::default())
        .map_err(|error| error.to_string())?;

    Ok(Report {
        gain_db,
        loudness_lufs: measured.integrated_lufs,
        peak: measured.peak,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_and_writes_replaygain_tags() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("tone.wav");
        crate::audio::setup::tone(&path, 48_000).unwrap();
        let report = scan_and_write(&path).unwrap();
        assert!(report.gain_db.is_finite());
        assert!(report.peak > 0.0);

        let file = lofty::read_from_path(&path).unwrap();
        let gain = file
            .primary_tag()
            .and_then(|tag| tag.get_string(ItemKey::ReplayGainTrackGain));
        assert!(gain.is_some(), "ReplayGain gain was not written");
        assert!(gain.unwrap().contains("dB"));
    }
}
