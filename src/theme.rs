use eframe::egui::{Color32, Visuals};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Theme {
    pub background: [u8; 3],
    pub surface: [u8; 3],
    pub text: [u8; 3],
    pub accent: [u8; 3],
}

impl Theme {
    pub const PRESETS: [(&'static str, Self); 5] = [
        (
            "Studio",
            Self {
                background: [25, 29, 37],
                surface: [33, 40, 50],
                text: [228, 233, 241],
                accent: [81, 155, 216],
            },
        ),
        (
            "Midnight",
            Self {
                background: [16, 19, 33],
                surface: [26, 30, 50],
                text: [232, 230, 250],
                accent: [157, 132, 248],
            },
        ),
        (
            "Graphite",
            Self {
                background: [25, 25, 28],
                surface: [38, 38, 42],
                text: [235, 233, 229],
                accent: [232, 165, 87],
            },
        ),
        (
            "Forest",
            Self {
                background: [19, 30, 26],
                surface: [30, 46, 38],
                text: [227, 241, 230],
                accent: [111, 192, 157],
            },
        ),
        (
            "Daylight",
            Self {
                background: [245, 247, 251],
                surface: [228, 234, 244],
                text: [29, 39, 56],
                accent: [47, 90, 168],
            },
        ),
    ];

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
        visuals.faint_bg_color = color(blend(self.background, self.surface, 0.5));
        let extreme = if dark { [0; 3] } else { [255; 3] };
        visuals.extreme_bg_color = color(blend(self.background, extreme, 0.4));
        visuals.code_bg_color = surface;
        visuals.override_text_color = Some(text);
        visuals.weak_text_color = Some(color(blend(self.text, self.background, 0.35)));
        visuals.hyperlink_color = accent;
        visuals.selection.bg_fill = color(blend(self.background, self.accent, 0.40));
        visuals.selection.stroke.color = accent;
        visuals.window_stroke.color = color(blend(self.surface, self.text, 0.2));
        visuals.widgets.noninteractive.bg_fill = surface;
        visuals.widgets.noninteractive.fg_stroke.color = text;
        visuals.widgets.inactive.bg_fill = surface;
        visuals.widgets.inactive.weak_bg_fill = surface;
        visuals.widgets.hovered.bg_fill = color(blend(self.surface, self.accent, 0.25));
        visuals.widgets.hovered.weak_bg_fill = visuals.widgets.hovered.bg_fill;
        visuals.widgets.active.bg_fill = visuals.selection.bg_fill;
        visuals.widgets.active.weak_bg_fill = visuals.widgets.active.bg_fill;
        visuals
    }

    pub fn album_colors(self) -> (Color32, Color32) {
        (
            color(blend(self.surface, self.accent, 0.14)),
            color(blend(self.text, self.accent, 0.42)),
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
