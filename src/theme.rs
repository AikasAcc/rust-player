use eframe::egui::{Color32, Visuals};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Serialize, Deserialize)]
pub struct NamedTheme {
    pub name: String,
    pub theme: Theme,
    pub font: Option<PathBuf>,
    #[serde(default)]
    pub background: crate::background::Settings,
}

impl crate::ThemeDraft {
    pub(super) fn saved_theme_settings(&mut self, ui: &mut eframe::egui::Ui) {
        use eframe::egui::{Button, ComboBox, TextEdit};
        self.selected = self.selected.filter(|&index| index < self.themes.len());
        let title = self.selected.map_or("Choose a saved theme", |index| {
            self.themes[index].name.as_str()
        });
        ComboBox::from_label("Saved themes")
            .selected_text(title)
            .show_ui(ui, |ui| {
                for (index, theme) in self.themes.iter().enumerate() {
                    if ui
                        .selectable_value(&mut self.selected, Some(index), &theme.name)
                        .clicked()
                    {
                        self.name.clone_from(&theme.name);
                        self.theme_error.clear();
                    }
                }
            });
        ui.add(
            TextEdit::singleline(&mut self.name)
                .hint_text("Theme name")
                .char_limit(48),
        );
        let name = self.name.trim().to_owned();
        let taken = self
            .themes
            .iter()
            .position(|theme| theme.name.to_lowercase() == name.to_lowercase());
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !name.is_empty() && taken.is_none(),
                    Button::new("Save as new"),
                )
                .clicked()
            {
                let theme = NamedTheme {
                    name: name.clone(),
                    theme: self.theme,
                    font: self.font.clone(),
                    background: self.background.clone(),
                };
                self.selected = Some(self.themes.len());
                self.themes.push(theme);
                self.theme_error.clear();
            }
            if let Some(index) = self.selected {
                if ui.button("Load").clicked() {
                    let theme = &self.themes[index];
                    self.theme = theme.theme;
                    self.background = theme.background.clone();
                    match crate::fonts::load(theme.font.as_deref()) {
                        Ok(fonts) => {
                            ui.ctx().set_fonts(fonts);
                            self.font_error.clear();
                        }
                        Err(error) => {
                            ui.ctx().set_fonts(Default::default());
                            self.font_error = format!("Could not load font: {error}");
                        }
                    }
                    self.font = theme.font.clone();
                    self.copy_background = false;
                }
                if ui
                    .button("Update")
                    .on_hover_text(
                        "Save the current colors, font and background to the selected theme.",
                    )
                    .clicked()
                {
                    let saved = &mut self.themes[index];
                    saved.theme = self.theme;
                    saved.font.clone_from(&self.font);
                    saved.background.clone_from(&self.background);
                }
                if ui
                    .add_enabled(
                        !name.is_empty()
                            && (taken.is_none() || taken == Some(index))
                            && name != self.themes[index].name,
                        Button::new("Rename"),
                    )
                    .clicked()
                {
                    self.themes[index].name = name.clone();
                }
                if ui.button("Duplicate").clicked() {
                    let mut duplicate = self.themes[index].clone();
                    let base: String = duplicate.name.chars().take(34).collect();
                    duplicate.name = (1..)
                        .map(|number| format!("{base} copy {number}"))
                        .find(|name| {
                            self.themes
                                .iter()
                                .all(|theme| theme.name.to_lowercase() != name.to_lowercase())
                        })
                        .expect("an unused theme name");
                    self.name.clone_from(&duplicate.name);
                    self.selected = Some(self.themes.len());
                    self.themes.push(duplicate);
                }
                if ui
                    .button("Delete")
                    .on_hover_text("Remove this saved theme when you apply the editor changes.")
                    .clicked()
                {
                    self.themes.remove(index);
                    self.selected = None;
                    self.name.clear();
                }
            }
        });
        if !self.theme_error.is_empty() {
            ui.weak(&self.theme_error);
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Density {
    Compact,
    #[default]
    Comfortable,
    Relaxed,
}

impl Density {
    pub const ALL: [Self; 3] = [Self::Compact, Self::Comfortable, Self::Relaxed];

    pub fn label(self) -> &'static str {
        match self {
            Self::Compact => "Compact",
            Self::Comfortable => "Comfortable",
            Self::Relaxed => "Relaxed",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Theme {
    pub background: [u8; 3],
    pub surface: [u8; 3],
    pub text: [u8; 3],
    pub accent: [u8; 3],
    pub density: Density,
    pub corner_radius: u8,
    pub font_scale_percent: u8,
    pub surface_strength_percent: u8,
    pub accent_strength_percent: u8,
    pub high_contrast: bool,
}

impl Theme {
    pub const PRESETS: [(&'static str, Self); 9] = [
        (
            "Studio",
            Self {
                background: [14, 18, 27],
                surface: [25, 31, 45],
                text: [236, 240, 247],
                accent: [92, 161, 255],
                ..Self::DEFAULT_STYLE
            },
        ),
        (
            "Midnight",
            Self {
                background: [16, 19, 33],
                surface: [26, 30, 50],
                text: [232, 230, 250],
                accent: [157, 132, 248],
                ..Self::DEFAULT_STYLE
            },
        ),
        (
            "Graphite",
            Self {
                background: [25, 25, 28],
                surface: [38, 38, 42],
                text: [235, 233, 229],
                accent: [232, 165, 87],
                ..Self::DEFAULT_STYLE
            },
        ),
        (
            "Forest",
            Self {
                background: [19, 30, 26],
                surface: [30, 46, 38],
                text: [227, 241, 230],
                accent: [111, 192, 157],
                ..Self::DEFAULT_STYLE
            },
        ),
        (
            "Daylight",
            Self {
                background: [245, 247, 251],
                surface: [228, 234, 244],
                text: [29, 39, 56],
                accent: [47, 90, 168],
                ..Self::DEFAULT_STYLE
            },
        ),
        (
            "Ocean",
            Self {
                background: [10, 24, 35],
                surface: [19, 43, 58],
                text: [229, 244, 249],
                accent: [48, 190, 204],
                ..Self::DEFAULT_STYLE
            },
        ),
        (
            "Rosewood",
            Self {
                background: [35, 19, 29],
                surface: [55, 30, 44],
                text: [250, 236, 241],
                accent: [239, 112, 156],
                ..Self::DEFAULT_STYLE
            },
        ),
        (
            "Mono",
            Self {
                background: [20, 21, 23],
                surface: [33, 35, 38],
                text: [243, 243, 243],
                accent: [224, 224, 224],
                density: Density::Compact,
                corner_radius: 3,
                high_contrast: true,
                ..Self::DEFAULT_STYLE
            },
        ),
        (
            "Windows XP",
            Self {
                background: [210, 232, 255],
                surface: [126, 184, 237],
                text: [0, 42, 104],
                accent: [56, 151, 56],
                density: Density::Compact,
                corner_radius: 6,
                high_contrast: true,
                ..Self::DEFAULT_STYLE
            },
        ),
    ];

    const DEFAULT_STYLE: Self = Self {
        background: [0; 3],
        surface: [0; 3],
        text: [0; 3],
        accent: [0; 3],
        density: Density::Comfortable,
        corner_radius: 10,
        font_scale_percent: 100,
        surface_strength_percent: 72,
        accent_strength_percent: 32,
        high_contrast: false,
    };

    pub fn visuals(self) -> Visuals {
        let [r, g, b] = self.background.map(u32::from);
        let dark = 2126 * r + 7152 * g + 722 * b < 128 * 10_000;
        let mut visuals = if dark {
            Visuals::dark()
        } else {
            Visuals::light()
        };
        let background = color(self.background);
        let surface = color(self.surface);
        let text = color(self.text);
        let accent = color(self.accent);

        visuals.panel_fill = background;
        visuals.window_fill = surface;
        let surface_strength = f32::from(self.surface_strength_percent.clamp(45, 90)) / 100.0;
        let accent_strength = f32::from(self.accent_strength_percent.clamp(15, 75)) / 100.0;
        visuals.faint_bg_color = color(blend(self.background, self.surface, surface_strength));
        let extreme = if dark { [0; 3] } else { [255; 3] };
        visuals.extreme_bg_color = color(blend(self.background, extreme, 0.24));
        visuals.code_bg_color = surface;
        visuals.text_edit_bg_color = Some(color(blend(self.background, self.surface, 0.42)));
        visuals.override_text_color = Some(text);
        visuals.weak_text_color = Some(color(blend(self.text, self.background, 0.35)));
        visuals.hyperlink_color = accent;
        visuals.selection.bg_fill = color(blend(self.background, self.accent, accent_strength));
        visuals.selection.stroke.color = accent;
        let corner_radius = self.corner_radius.min(24);
        visuals.window_corner_radius = eframe::egui::CornerRadius::same(corner_radius);
        visuals.menu_corner_radius =
            eframe::egui::CornerRadius::same(corner_radius.saturating_sub(2));
        visuals.window_stroke.color = color(blend(
            self.surface,
            self.text,
            if self.high_contrast { 0.26 } else { 0.13 },
        ));
        visuals.window_stroke.width = 1.0;
        visuals.widgets.noninteractive.bg_fill = background;
        visuals.widgets.noninteractive.fg_stroke.color = text;
        visuals.widgets.noninteractive.corner_radius =
            eframe::egui::CornerRadius::same(corner_radius.saturating_sub(3));
        visuals.widgets.inactive.bg_fill =
            color(blend(self.surface, self.background, 1.0 - surface_strength));
        visuals.widgets.inactive.weak_bg_fill = visuals.widgets.inactive.bg_fill;
        visuals.widgets.inactive.corner_radius =
            eframe::egui::CornerRadius::same(corner_radius.saturating_sub(3));
        visuals.widgets.inactive.bg_stroke.color = color(blend(
            self.surface,
            self.text,
            if self.high_contrast { 0.20 } else { 0.10 },
        ));
        visuals.widgets.hovered.bg_fill =
            color(blend(self.surface, self.accent, accent_strength * 0.55));
        visuals.widgets.hovered.weak_bg_fill = visuals.widgets.hovered.bg_fill;
        visuals.widgets.hovered.corner_radius =
            eframe::egui::CornerRadius::same(corner_radius.saturating_sub(3));
        visuals.widgets.hovered.bg_stroke.color = color(blend(
            self.surface,
            self.accent,
            if self.high_contrast { 0.72 } else { 0.50 },
        ));
        visuals.widgets.active.bg_fill = visuals.selection.bg_fill;
        visuals.widgets.active.weak_bg_fill = visuals.widgets.active.bg_fill;
        visuals.widgets.active.corner_radius =
            eframe::egui::CornerRadius::same(corner_radius.saturating_sub(3));
        visuals
    }

    pub fn album_colors(self) -> (Color32, Color32) {
        (
            color(blend(self.surface, self.accent, 0.045)),
            color(blend(self.text, self.accent, 0.18)),
        )
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::PRESETS[0].1
    }
}

fn color([r, g, b]: [u8; 3]) -> Color32 {
    Color32::from_rgb(r, g, b)
}

fn blend(base: [u8; 3], other: [u8; 3], amount: f32) -> [u8; 3] {
    std::array::from_fn(|channel| {
        let from = f32::from(base[channel]);
        (from + (f32::from(other[channel]) - from) * amount).round() as u8
    })
}
