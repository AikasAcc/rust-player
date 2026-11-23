use eframe::egui::{FontData, FontDefinitions, FontFamily};
use std::{
    fs::File,
    io::{self, Read},
    path::Path,
};

const MAX_FONT_BYTES: u64 = 24 * 1024 * 1024;

pub fn load(path: Option<&Path>) -> io::Result<FontDefinitions> {
    let mut fonts = FontDefinitions::default();
    if let Some(path) = path {
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
