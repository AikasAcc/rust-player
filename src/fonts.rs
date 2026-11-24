use eframe::egui::{FontData, FontDefinitions, FontFamily};
use std::{
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

const MAX_FONT_BYTES: u64 = 24 * 1024 * 1024;
const BRAND_FONT: &str = "Guifx v2 Transports";

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

fn brand_font() -> Option<&'static Arc<FontData>> {
    static FONT: OnceLock<Option<Arc<FontData>>> = OnceLock::new();
    FONT.get_or_init(|| {
        [
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
        })
    })
    .as_ref()
}

pub fn defaults() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    if let Some(font) = brand_font() {
        fonts.font_data.insert(BRAND_FONT.into(), font.clone());
        fonts
            .families
            .insert(FontFamily::Name(BRAND_FONT.into()), vec![BRAND_FONT.into()]);
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
