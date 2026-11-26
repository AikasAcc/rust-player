use crate::*;
impl Player {
    pub(crate) fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(mut dialog) = self.dialogs.playlist_dialog.take() {
            let mut open = true;
            match &mut dialog {
                PlaylistDialog::Name { target, name } => {
                    let title = if target.is_some() {
                        "Rename playlist"
                    } else {
                        "New playlist"
                    };
                    let mut submit = false;
                    let mut cancel = false;
                    egui::Window::new(title)
                        .open(&mut open)
                        .collapsible(false)
                        .show(ctx, |ui| {
                            ui.label("Playlist name");
                            let input = ui.text_edit_singleline(name);
                            let enter =
                                input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            ui.horizontal(|ui| {
                                submit = ui
                                    .add_enabled(!name.trim().is_empty(), egui::Button::new("Save"))
                                    .clicked()
                                    || (enter && !name.trim().is_empty());
                                cancel = ui.button("Cancel").clicked();
                            });
                        });
                    if submit {
                        if let Some(list) = *target {
                            self.library.playlists[list].name = name.trim().to_owned();
                        } else {
                            self.library.playlists.push(Playlist {
                                ids: Vec::new(),
                                name: name.trim().to_owned(),
                                tracks: Vec::new(),
                                pinned: false,
                                sidebar_position: None,
                                sort: Sort::Album,
                                descending: false,
                            });
                            self.library.selected = self.library.playlists.len() - 1;
                        }
                        self.save();
                        open = false;
                    }
                    open &= !cancel;
                }
                PlaylistDialog::Delete(list) => {
                    let mut confirmed = false;
                    let mut cancel = false;
                    let name = &self.library.playlists[*list].name;
                    egui::Window::new("Delete playlist?")
                        .open(&mut open)
                        .collapsible(false)
                        .show(ctx, |ui| {
                            ui.label(format!("Delete “{name}” from Rust Player?"));
                            ui.label("Your audio files will stay on disk.");
                            ui.horizontal(|ui| {
                                cancel = ui.button("Cancel").clicked();
                                confirmed = ui.button("Delete playlist").clicked();
                            });
                        });
                    if confirmed {
                        self.delete_playlist(*list);
                    }
                    open &= !confirmed && !cancel;
                }
            }
            if open {
                self.dialogs.playlist_dialog = Some(dialog);
            }
        }

        if self.dialogs.discord_settings {
            self.discord_dialog(ctx);
        }
        if self.dialogs.activity_open {
            self.activity_dialog(ctx);
        }
        if self.dialogs.plugins_open {
            self.plugin_manager(ctx);
        }
        self.track_properties_dialog(ctx);
        self.cover_viewer_dialog(ctx);
        self.settings_dialog(ctx);
        self.audio_setup_dialog(ctx);
    }
    pub(crate) fn track_properties_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut properties) = self.dialogs.track_properties.take() else {
            return;
        };
        let mut keep_open = true;
        let mut open_folder = false;
        let mut save_tags = false;
        let mut scan_gain = false;
        let mut close = false;
        egui::Window::new("Track properties")
            .open(&mut keep_open)
            .collapsible(false)
            .resizable(true)
            .show(ctx, |ui| {
                let value = |ui: &mut egui::Ui, label: &str, value: &str| {
                    ui.horizontal_wrapped(|ui| {
                        ui.strong(label);
                        ui.label(if value.trim().is_empty() {
                            "Unknown"
                        } else {
                            value
                        });
                    });
                };
                let field = |ui: &mut egui::Ui, label: &str, value: &mut String| -> bool {
                    ui.horizontal(|ui| {
                        ui.add_sized([90.0, 0.0], egui::Label::new(label));
                        ui.text_edit_singleline(value).changed()
                    })
                    .inner
                };
                // A user edit wins over a late worker result.
                let mut edited = false;
                edited |= field(ui, "Title", &mut properties.title);
                edited |= field(ui, "Artist", &mut properties.artist);
                edited |= field(ui, "Album", &mut properties.album);
                edited |= field(ui, "Album artist", &mut properties.album_artist);
                edited |= field(ui, "Track", &mut properties.track);
                if edited {
                    properties.loading = false;
                }
                value(ui, "Duration", &properties.duration);
                value(ui, "Format", &properties.format);
                value(ui, "File size", &properties.size);
                ui.separator();
                let cue_track = crate::cue::reference(&properties.path).is_some();
                save_tags = ui
                    .add_enabled(!cue_track, egui::Button::new("Save tags"))
                    .clicked();
                if ui.button("Scan and write ReplayGain").clicked() {
                    scan_gain = true;
                }
                if cue_track {
                    ui.weak("CUE entries are virtual; edit the source audio file instead.");
                }
                if !properties.status.is_empty() {
                    ui.weak(&properties.status);
                }
                ui.separator();
                ui.strong("Location");
                ui.label(properties.path.display().to_string());
                ui.horizontal(|ui| {
                    open_folder = ui.button("Open containing folder").clicked();
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });
            });
        if close {
            keep_open = false;
        }
        if open_folder {
            self.open_containing_folder(properties.path.clone());
        }
        if save_tags {
            match crate::tagging::write(
                &properties.path,
                crate::tagging::Edit {
                    title: &properties.title,
                    artist: &properties.artist,
                    album: &properties.album,
                    album_artist: &properties.album_artist,
                    track: &properties.track,
                },
            ) {
                Ok(()) => {
                    properties.status = "Tags saved".into();
                    properties.loading = false;
                    self.show_notice("Tags saved");
                    self.refresh_metadata(ctx);
                }
                Err(error) => {
                    properties.status = format!("Save failed: {error}");
                    properties.loading = false;
                    self.show_notice(format!("Could not save tags: {error}"));
                }
            }
        }
        if scan_gain {
            let path = properties.path.clone();
            let (send, receive) = std::sync::mpsc::channel();
            self.gain_jobs.insert(path.clone(), receive);
            std::thread::spawn(move || {
                let _ = send.send(crate::gain::scan_and_write(&path));
            });
            properties.status = "Scanning in the background…".into();
            properties.loading = false;
        }
        if keep_open {
            self.dialogs.track_properties = Some(properties);
        }
    }

    pub(crate) fn open_cover_viewer(
        &mut self,
        ctx: &egui::Context,
        pixels: std::sync::Arc<image::RgbaImage>,
        title: String,
        artist: String,
        source: Option<PathBuf>,
    ) {
        self.dialogs.cover_viewer = Some(CoverViewer {
            cover: background::Cover::new(ctx, pixels.as_ref().clone()),
            title,
            artist,
            source,
        });
    }

    pub(crate) fn cover_viewer_dialog(&mut self, ctx: &egui::Context) {
        let Some(viewer) = self.dialogs.cover_viewer.take() else {
            return;
        };
        let mut keep_open = true;
        let mut close = false;
        let mut save = false;
        let mut save_sidecar = false;
        egui::Window::new("Album artwork")
            .open(&mut keep_open)
            .resizable(true)
            .default_width(620.0)
            .default_height(680.0)
            .show(ctx, |ui| {
                ui.heading(&viewer.title);
                if !viewer.artist.is_empty() {
                    ui.weak(&viewer.artist);
                }
                ui.add_space(8.0);
                let size = viewer.cover.texture.size_vec2();
                let available = ui.available_size();
                let limit = available.x.min((available.y - 58.0).max(120.0)).min(700.0);
                let response = ui.add(
                    egui::Image::new((
                        viewer.cover.texture.id(),
                        size * (limit / size.x.max(size.y)),
                    ))
                    .sense(egui::Sense::click()),
                );
                response.on_hover_text("Artwork preview");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    save = ui.button("Save cover as…").clicked();
                    save_sidecar = ui
                        .add_enabled(
                            viewer.source.is_some(),
                            egui::Button::new("Save next to track"),
                        )
                        .on_hover_text("Write cover.png beside the audio file without overwriting")
                        .clicked();
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });
            });
        if close {
            keep_open = false;
        }
        if save {
            let title: String = viewer
                .title
                .chars()
                .filter(|character| {
                    !matches!(
                        character,
                        '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                    )
                })
                .take(80)
                .collect();
            if let Some(path) = rfd::FileDialog::new()
                .set_file_name(format!(
                    "{}.png",
                    if title.is_empty() { "cover" } else { &title }
                ))
                .add_filter("PNG image", &["png"])
                .save_file()
            {
                match viewer.cover.save_png(&path) {
                    Ok(()) => self.status = format!("Saved cover to {}", path.display()),
                    Err(error) => self.status = format!("Could not save cover: {error}"),
                }
            }
        }
        if save_sidecar && let Some(source) = viewer.source.as_deref() {
            match viewer.cover.save_sidecar(source) {
                Ok(path) => self.status = format!("Saved cover to {}", path.display()),
                Err(error) => self.status = format!("Could not save cover: {error}"),
            }
        }
        if keep_open {
            self.dialogs.cover_viewer = Some(viewer);
        }
    }
    pub(crate) fn discord_dialog(&mut self, ctx: &egui::Context) {
        let mut open = self.dialogs.discord_settings;
        let mut save = false;
        let mut lookup_changed = false;
        let mut refresh = false;
        egui::Window::new("Discord Rich Presence").open(&mut open).show(ctx, |ui| {
            save |= ui.checkbox(&mut self.library.discord_enabled, "Share what I’m listening to").changed();
            lookup_changed = ui.checkbox(&mut self.library.online_artwork, "Look up artwork via Apple Music").changed();
            ui.weak("Artwork lookup sends track metadata to Apple's music catalog.");
            ui.label("Discord Application ID");
            save |= ui.text_edit_singleline(&mut self.library.discord_id).lost_focus();
            if self.library.discord_enabled && (self.library.discord_id.trim().is_empty()
                || !self.library.discord_id.trim().bytes().all(|b| b.is_ascii_digit())) {
                ui.label("Enter the numeric Application ID from the Discord Developer Portal.");
            }
            ui.label("Default artwork asset key or public HTTPS image URL");
            save |= ui.text_edit_singleline(&mut self.library.fallback_asset).lost_focus();
            if let Some(playing) = &self.playing {
                ui.label("Public HTTPS artwork URL for this track");
                ui.text_edit_singleline(&mut self.artwork_url);
                if ui.button("Save track artwork URL").clicked()
                    && let Some(path) = self
                        .library
                        .track_path(playing.id)
                        .map(|path| path.to_path_buf())
                {
                    let url = self.artwork_url.trim();
                    if url.is_empty() {
                        self.library.artwork_urls.remove(&path);
                        save = true;
                    } else if presence::public_artwork_url(url) {
                        self.library.artwork_urls.insert(path.clone(), url.to_owned());
                        save = true;
                    } else {
                        ui.label("Enter a public HTTPS image URL.");
                    }
                }
                if !playing.artwork_status.is_empty() {
                    ui.weak(playing.artwork_status);
                }
            }
            ui.label("Discord displays public images or uploaded assets. Local cover files stay on this device.");
            ui.weak("Discord never shows activity buttons to you on your own profile. To check them, join a voice channel and hover over yourself, or open your full profile from a second account.");
            ui.label(self.rpc.status());
            ui.horizontal(|ui| {
                if ui.button("Save settings").clicked() { save = true; }
                if ui.button("Refresh presence").clicked() { refresh = true; }
                if ui.button("GitHub").clicked() {
                    ctx.open_url(egui::OpenUrl::new_tab(crate::PROJECT_URL));
                }
            });
        });
        self.dialogs.discord_settings = open;
        if lookup_changed || refresh {
            self.lookup_artwork(ctx);
            save |= lookup_changed;
        }
        if save {
            self.save();
        }
        if save || refresh {
            self.sync_presence();
        }
    }

    pub(crate) fn activity_dialog(&mut self, ctx: &egui::Context) {
        egui::Window::new("Listening activity")
            .open(&mut self.dialogs.activity_open)
            .default_width(540.0)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.selectable_value(
                        &mut self.dialogs.activity_tab,
                        ActivityTab::History,
                        "History",
                    );
                    ui.selectable_value(
                        &mut self.dialogs.activity_tab,
                        ActivityTab::Songs,
                        "Most played songs",
                    );
                    ui.selectable_value(
                        &mut self.dialogs.activity_tab,
                        ActivityTab::Artists,
                        "Most played artists",
                    );
                });
                ui.separator();
                if self.listening.history.is_empty() {
                    ui.label("Play a track to start your listening history.");
                    return;
                }
                let ranks = if self.dialogs.activity_tab == ActivityTab::History {
                    None
                } else {
                    Some(
                        self.listening_ranks
                            .get_or_insert_with(|| self.listening.rankings()),
                    )
                };
                let count = match (&self.dialogs.activity_tab, ranks.as_deref()) {
                    (ActivityTab::Songs, Some(ranks)) => ranks.songs.len(),
                    (ActivityTab::Artists, Some(ranks)) => ranks.artists.len(),
                    _ => self.listening.history.len(),
                };
                if count == 0 {
                    ui.label("No tagged artists in your listening history.");
                }
                let now = listening::now();
                let row_height = ui.text_style_height(&egui::TextStyle::Body) + 10.0;
                egui::ScrollArea::vertical().max_height(420.0).show_rows(
                    ui,
                    row_height,
                    count,
                    |ui, visible| {
                        for index in visible {
                            let (title, artist, value) =
                                match (&self.dialogs.activity_tab, ranks.as_deref()) {
                                    (ActivityTab::Songs, Some(ranks)) => {
                                        let song = &self.listening.songs[&ranks.songs[index]];
                                        (
                                            song.title.as_str(),
                                            song.artist.as_str(),
                                            song.plays.to_string(),
                                        )
                                    }
                                    (ActivityTab::Artists, Some(ranks)) => {
                                        let artist = &self.listening.artists[&ranks.artists[index]];
                                        (artist.name.as_str(), "", artist.plays.to_string())
                                    }
                                    _ => {
                                        let entry = &self.listening.history[count - 1 - index];
                                        (
                                            entry.title.as_str(),
                                            entry.artist.as_str(),
                                            listening::age(now.saturating_sub(entry.time)),
                                        )
                                    }
                                };
                            ui.allocate_ui_with_layout(
                                egui::vec2(ui.available_width(), row_height),
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.weak(value);
                                    let text = if artist.is_empty() {
                                        (*title).to_owned()
                                    } else {
                                        format!("{title} · {artist}")
                                    };
                                    ui.with_layout(
                                        egui::Layout::left_to_right(egui::Align::Center),
                                        |ui| {
                                            ui.add(egui::Label::new(&text).truncate())
                                                .on_hover_text(text);
                                        },
                                    );
                                },
                            );
                        }
                    },
                );
            });
    }

    pub(crate) fn theme_editor(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let Some(mut draft) = self.theme_draft.take() else {
            return;
        };
        let previous = draft.theme;
        let max_height = (ctx.content_rect().height() - 155.0).clamp(100.0, 520.0);
        egui::ScrollArea::vertical()
            .id_salt("appearance settings")
            .max_height(max_height)
            .show(ui, |ui| {
                draft.saved_theme_settings(ui);
                ui.separator();
                let name = theme::Theme::PRESETS
                    .iter()
                    .find(|(_, preset)| *preset == draft.theme)
                    .map_or("Custom", |(name, _)| *name);
                egui::ComboBox::from_label("Preset")
                    .selected_text(name)
                    .show_ui(ui, |ui| {
                        for (name, preset) in theme::Theme::PRESETS {
                            ui.selectable_value(&mut draft.theme, preset, name);
                        }
                    });
                ui.separator();
                ui.group(|ui| {
                    ui.strong("Colors");
                    ui.weak("Tune the overall palette. Changes preview immediately.");
                    egui::Grid::new("theme colors")
                        .spacing([20.0, 8.0])
                        .show(ui, |ui| {
                            for (label, color) in [
                                ("Background", &mut draft.theme.background),
                                ("Panels and buttons", &mut draft.theme.surface),
                                ("Text", &mut draft.theme.text),
                                ("Accent", &mut draft.theme.accent),
                            ] {
                                ui.label(label);
                                ui.color_edit_button_srgb(color);
                                ui.end_row();
                            }
                        });
                });
                ui.separator();
                ui.group(|ui| {
                    ui.strong("Layout");
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Density");
                        for density in theme::Density::ALL {
                            ui.selectable_value(&mut draft.theme.density, density, density.label());
                        }
                    });
                    ui.add(
                        egui::Slider::new(&mut draft.theme.corner_radius, 0..=20)
                            .text("Corner roundness"),
                    );
                    ui.add(
                        egui::Slider::new(&mut draft.theme.surface_strength_percent, 45..=90)
                            .text("Panel contrast")
                            .suffix("%"),
                    );
                    ui.add(
                        egui::Slider::new(&mut draft.theme.accent_strength_percent, 15..=75)
                            .text("Highlight strength")
                            .suffix("%"),
                    );
                    ui.checkbox(&mut draft.theme.high_contrast, "Increase border contrast")
                        .on_hover_text("Makes panel and button boundaries more visible.");
                });
                ui.separator();
                ui.group(|ui| {
                    ui.strong("Typography");
                    ui.add(
                        egui::Slider::new(&mut draft.theme.font_scale_percent, 85..=135)
                            .text("Text size")
                            .suffix("%"),
                    );
                    let font = draft
                        .font
                        .as_ref()
                        .and_then(|path| path.file_name())
                        .map(|name| name.to_string_lossy())
                        .unwrap_or_else(|| "Default font".into());
                    ui.label(format!("Font: {font}"));
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Choose font…").clicked()
                            && let Some(path) = rfd::FileDialog::new()
                                .add_filter("Fonts", &["ttf", "otf"])
                                .pick_file()
                        {
                            match fonts::load(Some(&path)) {
                                Ok(fonts) => {
                                    ctx.set_fonts(fonts);
                                    draft.font = Some(path);
                                    draft.font_error.clear();
                                }
                                Err(error) => {
                                    draft.font_error = format!("Could not load font: {error}")
                                }
                            }
                        }
                        if ui.button("Use default font").clicked() {
                            ctx.set_fonts(fonts::defaults());
                            draft.font = None;
                            draft.font_error.clear();
                        }
                    });
                    if !draft.font_error.is_empty() {
                        ui.label(&draft.font_error);
                    }
                });
                ui.separator();
                ui.group(|ui| {
                    ui.strong("Background");
                    ui.weak("Use a wallpaper or let the current album artwork set the mood.");
                    draft.background.settings(
                        ui,
                        &mut draft.copy_background,
                        self.background.status(),
                    );
                });
            });
        ui.separator();
        let (apply, cancel) = ui
            .horizontal(|ui| (ui.button("Apply").clicked(), ui.button("Cancel").clicked()))
            .inner;
        if apply {
            if draft.copy_background
                && draft.background.uses_file()
                && let Some(path) = &draft.background.path
            {
                match background::copy_to_profile(path, &self.library_path) {
                    Ok(copied) => {
                        for profile in &mut draft.themes {
                            if profile.background.path.as_ref() == Some(path) {
                                profile.background.path = Some(copied.clone());
                            }
                        }
                        draft.background.path = Some(copied);
                    }
                    Err(error) => {
                        draft.theme_error = format!("Could not copy the background: {error}");
                        self.theme_draft = Some(draft);
                        return;
                    }
                }
            }
            self.library.theme = draft.theme;
            self.library.font = draft.font.clone();
            self.library.background = draft.background.clone();
            self.library.named_themes = std::mem::take(&mut draft.themes);
            self.save();
        }
        if apply {
            apply_ui_chrome(&ctx, self.library.theme);
            ctx.request_repaint();
        } else if cancel {
            self.restore_applied_theme(&ctx, &draft);
        } else {
            if draft.theme != previous {
                apply_ui_chrome(&ctx, draft.theme);
                ctx.request_repaint();
            }
            self.theme_draft = Some(draft);
        }
    }

    pub(crate) fn restore_applied_theme(&self, ctx: &egui::Context, draft: &ThemeDraft) {
        if draft.font != self.library.font {
            ctx.set_fonts(
                fonts::load(self.library.font.as_deref()).unwrap_or_else(|_| fonts::defaults()),
            );
        }
        apply_ui_chrome(ctx, self.library.theme);
        ctx.request_repaint();
    }

    pub(crate) fn plugin_manager(&mut self, ctx: &egui::Context) {
        let mut changed = false;
        let mut remove = None;
        egui::Window::new("Plugins")
            .open(&mut self.dialogs.plugins_open)
            .default_width(460.0)
            .min_width(420.0)
            .min_height(230.0)
            .show(ctx, |ui| {
                ui.weak("Add, enable, and manage local Rhai extensions.");
                ui.add_space(3.0);
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            self.library.plugins.len() < 32,
                            egui::Button::new("Add plugin…"),
                        )
                        .clicked()
                    {
                        let mut picker =
                            rfd::FileDialog::new().add_filter("Rhai plugin", &["rhai"]);
                        if let Ok(folder) = plugins::create_folder(&self.library_path) {
                            picker = picker.set_directory(folder);
                        }
                        if let Some(path) = picker.pick_file()
                            && !self
                                .library
                                .plugins
                                .iter()
                                .any(|plugin| plugin.path == path)
                        {
                            self.library.plugins.push(plugins::PluginConfig {
                                path,
                                enabled: false,
                            });
                            changed = true;
                        }
                    }
                    if ui.button("Open Plugin Folder").clicked() {
                        let result =
                            plugins::create_folder(&self.library_path).and_then(|folder| {
                                std::process::Command::new("explorer.exe")
                                    .arg(folder)
                                    .spawn()
                            });
                        if let Err(error) = result {
                            self.status = format!("Could not open plugin folder: {error}");
                        }
                    }
                    changed |= ui.button("Reload").clicked();
                });
                if self.library.plugins.is_empty() {
                    ui.add_space(18.0);
                    ui.vertical_centered(|ui| {
                        ui.strong("No plugins installed");
                        ui.weak("Add a .rhai plugin to extend Rust Player.");
                    });
                    ui.add_space(18.0);
                }
                egui::ScrollArea::vertical()
                    .max_height(340.0)
                    .show(ui, |ui| {
                        for (index, config) in self.library.plugins.iter_mut().enumerate() {
                            ui.horizontal_wrapped(|ui| {
                                let name = config.name();
                                changed |= ui
                                    .checkbox(&mut config.enabled, name)
                                    .on_hover_text(config.path.display().to_string())
                                    .changed();
                                ui.weak(self.plugins.status(index));
                                if ui.button("Remove").clicked() {
                                    remove = Some(index);
                                    changed = true;
                                }
                            });
                        }
                    });
            });
        if let Some(index) = remove {
            self.library.plugins.remove(index);
        }
        if changed {
            self.plugins = plugins::Host::load(&self.library.plugins);
            if let Some(playing) = &self.playing {
                self.plugins.track_changed(&playing.media);
            }
            self.save();
            ctx.request_repaint();
        }
    }
}
