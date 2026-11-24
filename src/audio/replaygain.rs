use std::path::Path;

use lofty::{file::TaggedFileExt, tag::ItemKey};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub enum Mode {
    #[default]
    Off,
    Track,
    Album,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default)]
pub struct Settings {
    pub mode: Mode,
    pub preamp_db: i8,
}

impl Settings {
    pub fn enabled(self) -> bool {
        self.mode != Mode::Off
    }

    pub fn factor(self, path: &Path) -> Option<f32> {
        if !self.enabled() {
            return None;
        }
        let file = lofty::read_from_path(path).ok()?;
        let tags = || file.primary_tag().into_iter().chain(file.tags());
        let find = |key| tags().find_map(|tag| tag.get_string(key).and_then(number));
        let track_gain = find(ItemKey::ReplayGainTrackGain);
        let track_peak = find(ItemKey::ReplayGainTrackPeak);
        let album_gain = find(ItemKey::ReplayGainAlbumGain);
        let album_peak = find(ItemKey::ReplayGainAlbumPeak);
        let (gain, peak) = if self.mode == Mode::Album && album_gain.is_some() {
            (album_gain, album_peak)
        } else {
            (track_gain, track_peak)
        };
        Some(linear(gain?, self.preamp_db, peak))
    }
}

fn number(value: &str) -> Option<f32> {
    let value = value.trim();
    let numeric = if value
        .as_bytes()
        .get(value.len().saturating_sub(2)..)
        .is_some_and(|suffix| suffix.eq_ignore_ascii_case(b"db"))
    {
        value.get(..value.len() - 2)?.trim_end()
    } else {
        value
    };
    numeric
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
}

fn linear(gain_db: f32, preamp_db: i8, peak: Option<f32>) -> f32 {
    let db = (gain_db + f32::from(preamp_db.clamp(-24, 12))).clamp(-60.0, 30.0);
    let peak = peak
        .filter(|&peak| peak > 0.0 && peak.is_finite())
        .unwrap_or(1.0);
    10_f32.powf(db / 20.0).min(1.0 / peak)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_decibels_and_prevents_peak_clipping() {
        assert_eq!(number(" -6.0 DB "), Some(-6.0));
        assert_eq!(number("NaN"), None);
        assert!((linear(-6.0, 0, Some(0.9)) - 0.501_187).abs() < 0.000_001);
        assert!((linear(6.0, 3, Some(0.8)) - 1.25).abs() < 0.000_001);
        assert_eq!(linear(12.0, 0, None), 1.0);
    }
}
