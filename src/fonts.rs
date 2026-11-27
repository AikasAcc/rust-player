use eframe::egui::{FontData, FontDefinitions, FontFamily};
use std::{
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

const MAX_FONT_BYTES: u64 = 24 * 1024 * 1024;
const BRAND_FONT: &str = "Guifx v2 Transports";

/// Windows fonts used as fallbacks for glyphs the bundled epaint fonts do not
/// cover. The first two are small and handle symbols plus Cyrillic, Greek and
/// Arabic; the others add Chinese/Japanese and Indic scripts.
#[cfg(windows)]
const SYSTEM_FALLBACKS: &[&str] = &[
    "seguisym.ttf", // ♫ ♥ ▶ ★ ❯ and other symbols
    "segoeui.ttf",  // Cyrillic, Greek, Arabic, Hebrew
    "msyh.ttc",     // Chinese and Japanese
    "Nirmala.ttc",  // Devanagari and other Indic scripts
];

fn read_font(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_FONT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FONT_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Font file exceeds 24 MB",
        ));
    }
    Ok(bytes)
}

/// Load the system fallback fonts once. Missing fonts are simply skipped: the
/// bundled epaint fonts still cover the Latin UI, only exotic scripts would
/// show replacement boxes on a system without these fonts.
#[cfg(windows)]
fn system_fallbacks() -> &'static [(String, Arc<FontData>)] {
    static FONTS: OnceLock<Vec<(String, Arc<FontData>)>> = OnceLock::new();
    FONTS.get_or_init(|| {
        let directory = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
            .join("Fonts");
        SYSTEM_FALLBACKS
            .iter()
            .filter_map(|file| {
                let bytes = read_font(&directory.join(file)).ok()?;
                ttf_parser::Face::parse(&bytes, 0).ok()?;
                Some((
                    format!("system_{file}"),
                    Arc::new(FontData::from_owned(bytes)),
                ))
            })
            .collect()
    })
}

fn brand_font() -> Option<&'static Arc<FontData>> {
    static FONT: OnceLock<Arc<FontData>> = OnceLock::new();
    if let Some(font) = FONT.get() {
        return Some(font);
    }
    let found = [
        ("LOCALAPPDATA", "Microsoft/Windows/Fonts"),
        ("WINDIR", "Fonts"),
    ]
    .into_iter()
    .find_map(|(variable, directory)| {
        let path = PathBuf::from(std::env::var_os(variable)?)
            .join(directory)
            .join("Guifx v2 Transports.ttf");
        let bytes = read_font(&path).ok()?;
        ttf_parser::Face::parse(&bytes, 0).ok()?.glyph_index('>')?;
        Some(Arc::new(FontData::from_owned(bytes)))
    });
    if let Some(font) = found {
        let _ = FONT.set(font);
    }
    // A missing font is not cached: installing it later can take effect without
    // a restart. A found font is cached so discovery is not repeated per frame.
    FONT.get()
}

pub fn defaults() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    if let Some(font) = brand_font() {
        fonts.font_data.insert(BRAND_FONT.into(), font.clone());
        fonts
            .families
            .insert(FontFamily::Name(BRAND_FONT.into()), vec![BRAND_FONT.into()]);
    }
    // Append, never prepend: the bundled Ubuntu/emoji fonts keep their look for
    // characters they already cover, system fonts only fill the gaps.
    #[cfg(windows)]
    for (name, data) in system_fallbacks() {
        fonts.font_data.insert(name.clone(), data.clone());
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            fonts.families.entry(family).or_default().push(name.clone());
        }
    }
    fonts
}

pub fn brand_mark() -> (&'static str, FontFamily) {
    if brand_font().is_some() {
        (">", FontFamily::Name(BRAND_FONT.into()))
    } else {
        ("❯", FontFamily::Proportional)
    }
}

pub fn load(path: Option<&Path>) -> io::Result<FontDefinitions> {
    let mut fonts = defaults();
    if let Some(path) = path {
        let bytes = read_font(path)?;
        ttf_parser::Face::parse(&bytes, 0).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "Unreadable TTF or OTF font")
        })?;
        let name = "custom_ui".to_owned();
        fonts
            .font_data
            .insert(name.clone(), FontData::from_owned(bytes).into());
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, name);
    }
    Ok(fonts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_define_both_text_families() {
        let fonts = defaults();
        assert!(fonts.families.contains_key(&FontFamily::Proportional));
        assert!(fonts.families.contains_key(&FontFamily::Monospace));
    }

    #[test]
    #[cfg(windows)]
    fn fallback_fonts_cover_japanese_genre_values() {
        // The screenshot that prompted this fallback showed six tofu boxes for
        // "ヒップホップ". Keep the coverage guard so a Windows update cannot
        // silently regress it.
        const SAMPLE: &str = "ヒップホップ";
        let fonts = defaults();
        let proportional = &fonts.families[&FontFamily::Proportional];
        let covered = proportional.iter().any(|name| {
            let data = &fonts.font_data[name];
            ttf_parser::Face::parse(data.font.as_ref(), data.index)
                .is_ok_and(|face| SAMPLE.chars().all(|c| face.glyph_index(c).is_some()))
        });
        assert!(covered, "no proportional font covers {SAMPLE:?}");
    }
}
