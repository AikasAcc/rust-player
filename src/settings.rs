use crate::{Player, ThemeDraft};
use eframe::egui;

const BUG_REPORT_URL: &str = "https://github.com/AikasAcc/rust-player/issues/new/choose";

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
    #[default]
    General,
    Audio,
    Playback,
    Appearance,
    Integrations,
    About,
}

impl Player {
    pub(super) fn settings_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut page) = self.settings_page else {
            return;
        };
        let mut open = true;
        let mut changed = false;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .default_width(480.0)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (section, label) in [
                        (Page::General, "General"),
                        (Page::Audio, "Audio"),
                        (Page::Playback, "Playback"),
                        (Page::Appearance, "Appearance"),
                        (Page::Integrations, "Integrations"),
                        (Page::About, "About"),
                    ] {
                        ui.selectable_value(&mut page, section, label);
                    }
                });
                ui.separator();
                match page {
                    Page::General => {
                        changed |= ui
                            .checkbox(&mut self.library.media_keys, "Use keyboard media keys")
                            .changed();
                        self.media_key_status(ui);
                    }
                    Page::Audio => {
                        let height = (ctx.content_rect().height() - 120.0).clamp(100.0, 560.0);
                        egui::ScrollArea::vertical()
                            .id_salt("audio settings")
                            .max_height(height)
                            .show(ui, |ui| self.audio_settings(ui));
                    }
                    Page::Playback => {
                        changed |= self.gapless_settings(ui);
                        changed |= ui
                            .checkbox(
                                &mut self.library.restore_last_track,
                                "Restore last track and position on startup",
                            )
                            .changed();
                        changed |= ui
                            .checkbox(
                                &mut self.library.restart_track_on_previous,
                                "Restart the current track when pressing Previous after 3 seconds",
                            )
                            .changed();
                    }
                    Page::Appearance => {
                        self.begin_theme_edit();
                        self.theme_editor(ui);
                    }
                    Page::Integrations => {
                        self.online_lyrics_setting(ui);
                        ui.horizontal_wrapped(|ui| {
                            if ui.button("Discord").clicked() {
                                self.discord_settings = true;
                            }
                            if ui.button("Plugins").clicked() {
                                self.plugins_open = true;
                            }
                        });
                        ui.separator();
                        self.lastfm_settings(ui);
                    }
                    Page::About => {
                        ui.label(concat!("Rust Player ", env!("CARGO_PKG_VERSION")));
                        self.diagnostic_settings(ui);
                        if ui.button("Report a bug").clicked() {
                            ctx.open_url(egui::OpenUrl::new_tab(BUG_REPORT_URL));
                        }
                        ui.weak(
                            "Use Copy diagnostics to include audio format details in a bug report.",
                        );
                    }
                }
            });
        self.settings_page = open.then_some(page);
        if !open && let Some(draft) = self.theme_draft.take() {
            self.restore_applied_theme(ctx, &draft);
        }
        if changed {
            self.save();
        }
    }

    fn media_key_status(&self, ui: &mut egui::Ui) {
        if !self.library.media_keys {
            return;
        }
        match &self.media_keys {
            Ok(_) => {
                ui.weak("Windows handles Play/Pause, Previous, and Next.");
            }
            Err(error) if self.taskbar.is_some() => {
                ui.weak("Media keys are available while Rust Player has focus.")
                    .on_hover_text(error);
            }
            Err(error) => {
                ui.weak("Windows media keys are unavailable.")
                    .on_hover_text(error);
            }
        }
    }

    pub(super) fn online_lyrics_setting(&mut self, ui: &mut egui::Ui) {
        let changed = ui
            .checkbox(
                &mut self.library.online_lyrics,
                "Find missing lyrics online",
            )
            .on_hover_text("Send track metadata to LRCLIB when local lyrics are missing.")
            .changed();
        if changed {
            self.save();
            if self.library.online_lyrics
                && self
                    .playing
                    .as_ref()
                    .is_some_and(|p| p.media.lyrics.trim().is_empty())
            {
                self.lookup_lyrics(ui.ctx());
            }
        }
    }

    pub(super) fn open_theme_editor(&mut self) {
        self.settings_page = Some(Page::Appearance);
    }

    fn begin_theme_edit(&mut self) {
        if self.theme_draft.is_none() {
            self.theme_draft = Some(ThemeDraft {
                theme: self.library.theme,
                font: self.library.font.clone(),
                font_error: String::new(),
                background: self.library.background.clone(),
                themes: self.library.named_themes.clone(),
                selected: None,
                name: String::new(),
                theme_error: String::new(),
                copy_background: false,
            });
        }
    }
}
