use crate::*;
impl Player {
    pub(crate) fn header(&mut self, ctx: &egui::Context) {
        let focus_search =
            ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::K));
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            let [mut brand, mut search_ui, mut actions] = desk_sections(ui, 48.0, 244.0);
            let header = ui.max_rect().intersect(egui::Rect::from_min_max(
                egui::pos2(ui.clip_rect().left(), brand.max_rect().top()),
                egui::pos2(ui.clip_rect().right(), brand.max_rect().bottom()),
            ));
            brand.horizontal_centered(|ui| {
                let (mark, family) = fonts::brand_mark();
                ui.label(
                    RichText::new(mark)
                        .font(egui::FontId::new(18.0, family))
                        .color(ui.visuals().selection.stroke.color),
                );
                ui.strong("Rust Player");
            });
            let search_box = search_ui
                .horizontal_centered(|ui| {
                    egui::Frame::new()
                        .fill(ui.visuals().window_fill)
                        .stroke(ui.visuals().window_stroke)
                        .corner_radius(4)
                        .inner_margin(egui::Margin::symmetric(7, 2))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(14.0, 14.0),
                                    egui::Sense::hover(),
                                );
                                let center = rect.center() - egui::vec2(1.0, 1.0);
                                let stroke =
                                    egui::Stroke::new(1.0_f32, ui.visuals().weak_text_color());
                                ui.painter().circle_stroke(center, 4.0, stroke);
                                ui.painter().line_segment(
                                    [center + egui::vec2(3.0, 3.0), center + egui::vec2(6.0, 6.0)],
                                    stroke,
                                );
                                let search = ui
                                    .add(
                                        egui::TextEdit::singleline(&mut self.track_search)
                                            .frame(false)
                                            .desired_width(142.0)
                                            .hint_text("Search playlist"),
                                    )
                                    .on_hover_text(
                                        "Search titles, artists, albums and file names (Ctrl+K)",
                                    );
                                if search.changed() {
                                    let wait = Duration::from_millis(80);
                                    self.search_deadline = Some(Instant::now() + wait);
                                    ctx.request_repaint_after(wait);
                                }
                                if focus_search {
                                    search.request_focus();
                                }
                                if self.track_search.is_empty() {
                                    ui.label(RichText::new("Ctrl K").small().weak());
                                } else if ui
                                    .small_button("×")
                                    .on_hover_text("Clear search")
                                    .clicked()
                                {
                                    self.track_search.clear();
                                }
                            });
                        })
                        .response
                        .rect
                })
                .inner;
            let action_box = actions
                .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    window_chrome::buttons(ui);
                    ui.separator();
                    self.header_actions(ui);
                    ui.min_rect()
                })
                .inner;
            let zones = window_chrome::drag_zones(header, [search_box, action_box]);
            if let Some(chrome) = &self.window_chrome {
                chrome.set_captions(zones, ctx.pixels_per_point());
            } else {
                for (i, rect) in zones.into_iter().enumerate() {
                    ui.push_id(i, |ui| window_chrome::title_drag(ui, rect));
                }
            }
        });
    }
    pub(crate) fn transport(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let buttons = self.taskbar_buttons();
        let active = self.playing.is_some();
        let paused = self.playing.as_ref().is_some_and(|p| p.sink.is_paused());
        let center_width = (ui.available_width() - 324.0).clamp(245.0, 440.0);
        let [mut track_ui, mut transport_ui, mut volume_ui] = desk_sections(ui, 56.0, center_width);
        track_ui.horizontal_centered(|ui| {
            if let Some(cover) = self.playing.as_ref().and_then(Playing::cover) {
                let size = cover.texture.size_vec2();
                ui.add(egui::Image::new((
                    cover.texture.id(),
                    size * (35.0 / size.x.max(size.y)),
                )));
            }
            let label_width = (ui.available_width() - 32.0).max(35.0);
            ui.allocate_ui_with_layout(
                egui::vec2(label_width, 34.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_max_width(label_width);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    if let Some(playing) = &self.playing {
                        ui.add(
                            egui::Label::new(RichText::new(&playing.media.title).strong())
                                .truncate(),
                        );
                        ui.add(
                            egui::Label::new(RichText::new(&playing.media.artist).weak().small())
                                .truncate(),
                        );
                    } else {
                        ui.weak("Nothing playing");
                    }
                },
            );
            if ui
                .add_enabled(
                    active,
                    egui::Button::new(if self.current_is_favorite() {
                        "♥"
                    } else {
                        "♡"
                    })
                    .frame(false),
                )
                .on_hover_text("Toggle favorite")
                .clicked()
            {
                self.toggle_favorite();
            }
        });
        transport_ui.vertical_centered(|ui| {
            ui.horizontal(|ui| {
                // Equal gutters keep the central play/pause control above the timeline.
                ui.add_space((ui.available_width() - 158.0).max(0.0) * 0.5);
                ui.spacing_mut().item_spacing.x = 2.0;
                if icon(ui, Icon::Stop, "Stop", active, false) {
                    self.stop();
                }
                if icon(
                    ui,
                    Icon::Previous,
                    "Previous track",
                    buttons.previous,
                    false,
                ) {
                    self.previous(ctx);
                }
                let show_pause = active && !paused;
                if icon(
                    ui,
                    if show_pause { Icon::Pause } else { Icon::Play },
                    if show_pause { "Pause" } else { "Play" },
                    buttons.can_play,
                    false,
                ) {
                    if show_pause {
                        self.pause();
                    } else {
                        self.resume(ctx);
                    }
                }
                if icon(ui, Icon::Next, "Next track", buttons.next, false) {
                    self.step(1, ctx);
                }
                if icon(ui, Icon::Shuffle, "Shuffle", true, self.library.shuffle) {
                    self.toggle_shuffle();
                }
            });
            if let Some(playing) = &self.playing {
                let elapsed = playing.sink.get_pos();
                if let Some(duration) = playing.duration.filter(|d| !d.is_zero()) {
                    let target = ui
                        .push_id(("seek", playing.id), |ui| seek_bar(ui, elapsed, duration))
                        .inner;
                    if let Some(target) = target {
                        self.seek(target, ctx);
                    }
                } else {
                    ui.weak(time(elapsed));
                }
            }
        });
        volume_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let volume = ui
                .add_enabled_ui(self.software_volume_available(), |ui| {
                    desk_volume(ui, &mut self.library.volume, self.library.muted)
                })
                .inner;
            if volume.speaker_clicked {
                self.toggle_mute();
            } else if volume.response.changed() {
                self.library.muted = false;
                self.volume_adjusted();
            }
        });
    }

    pub(crate) fn current_is_favorite(&mut self) -> bool {
        let Some(playing) = &self.playing else {
            return false;
        };
        if let Some((id, value)) = self.cached_favorite
            && id == playing.id
        {
            return value;
        }
        let favorite = self.library.is_favorite_id(playing.id);
        self.cached_favorite = Some((playing.id, favorite));
        favorite
    }

    pub(crate) fn header_actions(&mut self, ui: &mut egui::Ui) {
        use HeaderAction::*;
        const ADD: &[(&str, HeaderAction)] = &[("Audio files", Files), ("Folder", Folder)];
        const TOOLS: &[(&str, HeaderAction)] = &[
            ("Theme editor", Theme),
            ("Plugins", Plugins),
            ("Discord", Discord),
            ("GitHub", GitHub),
        ];
        let mut selection = None;
        if ui.available_width() >= 185.0 {
            if ui.button("Settings").clicked() {
                selection = Some(Settings);
            }
            for (name, items) in [("Add…", ADD), ("Tools", TOOLS)] {
                ui.menu_button(name, |ui| {
                    for &(label, action) in items {
                        if ui.button(label).clicked() {
                            selection = Some(action);
                            ui.close();
                        }
                    }
                });
            }
        } else {
            ui.menu_button("Menu", |ui| {
                for &(label, action) in [("Settings", Settings)].iter().chain(ADD).chain(TOOLS) {
                    if ui.button(label).clicked() {
                        selection = Some(action);
                        ui.close();
                    }
                }
            });
        }
        match selection {
            Some(Settings) => {
                self.dialogs
                    .settings_page
                    .get_or_insert_with(settings::Page::default);
            }
            Some(Files) => {
                if let Some(files) = rfd::FileDialog::new()
                    .add_filter("Audio and CUE sheets", library::AUDIO)
                    .pick_files()
                {
                    self.import(files, ui.ctx());
                }
            }
            Some(Folder) => {
                if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                    self.import(vec![folder], ui.ctx());
                }
            }
            Some(Theme) => self.open_theme_editor(),
            Some(Plugins) => self.dialogs.plugins_open = true,
            Some(Discord) => self.dialogs.discord_settings = true,
            Some(GitHub) => ui
                .ctx()
                .open_url(egui::OpenUrl::new_tab(crate::PROJECT_URL)),
            None => {}
        }
    }
}
