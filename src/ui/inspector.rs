use crate::*;
impl Player {
    pub(crate) fn inspector(&mut self, ctx: &egui::Context) {
        let mut cover_open = None;
        let mut lyric_seek = None;
        egui::SidePanel::right("now_playing")
            .default_width(235.0)
            .min_width(185.0)
            .frame(desk_surface(&ctx.style().visuals))
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    for (tab, label) in [
                        (InspectorTab::Queue, "Queue"),
                        (InspectorTab::Track, "Track"),
                    ] {
                        let caption = if tab == InspectorTab::Queue && self.queue_count() > 0 {
                            format!("Queue {}", self.queue_count())
                        } else {
                            label.to_owned()
                        };
                        ui.selectable_value(&mut self.inspector_tab, tab, caption);
                    }
                });
                ui.separator();
                if self.inspector_tab == InspectorTab::Queue {
                    self.queue_panel(ui);
                    return;
                }
                if let Some(playing) = &self.playing {
                    let cover_edge = ui
                        .available_width()
                        .min(ui.available_height() * 0.48)
                        .clamp(1.0, 300.0);
                    egui::Frame::new()
                        .fill(ui.visuals().faint_bg_color)
                        .corner_radius(5)
                        .inner_margin(egui::Margin::same(8))
                        .show(ui, |ui| {
                            let cover_edge = cover_edge.min(ui.available_width());
                            if let Some(artwork) = playing.cover() {
                                let size = artwork.texture.size_vec2();
                                let response = ui.add(
                                    egui::Image::new((
                                        artwork.texture.id(),
                                        size * (cover_edge / size.x.max(size.y)),
                                    ))
                                    .sense(egui::Sense::click()),
                                );
                                if response.double_clicked() {
                                    cover_open = Some((
                                        artwork.pixels(),
                                        playing.media.title.clone(),
                                        playing.media.artist.clone(),
                                        self.library
                                            .track_path(playing.id)
                                            .map(|path| path.to_path_buf()),
                                    ));
                                }
                                response.on_hover_text("Double-click to open artwork");
                            } else {
                                ui.allocate_ui_with_layout(
                                    egui::vec2(cover_edge, cover_edge),
                                    egui::Layout::top_down(egui::Align::Center)
                                        .with_main_justify(true),
                                    |ui| ui.weak("No cover artwork"),
                                );
                            }
                            ui.add_space(6.0);
                            ui.label(RichText::new(&playing.media.title).strong().size(16.0));
                            if !playing.media.artist.is_empty() {
                                ui.label(RichText::new(&playing.media.artist).size(13.5));
                            }
                            if !playing.media.album.is_empty() {
                                ui.weak(&playing.media.album);
                            }
                            let connected = self.lastfm.connected() && self.library.lastfm_enabled;
                            ui.horizontal(|ui| {
                                if ui
                                    .add_enabled(connected, egui::Button::new("♥ Last.fm"))
                                    .on_hover_text("Love the current track on Last.fm")
                                    .clicked()
                                {
                                    self.lastfm.love(&playing.media, playing.duration, true);
                                }
                                if ui
                                    .add_enabled(connected, egui::Button::new("♡ Last.fm"))
                                    .on_hover_text(
                                        "Remove the current track from Last.fm loved tracks",
                                    )
                                    .clicked()
                                {
                                    self.lastfm.love(&playing.media, playing.duration, false);
                                }
                            });
                        });
                }
                let mut search = false;
                let mut save = false;
                if self.playing.is_some() {
                    ui.separator();
                    ui.heading("Lyrics");
                    self.online_lyrics_setting(ui);
                }
                if let Some(playing) = &self.playing {
                    ui.horizontal_wrapped(|ui| {
                        search = ui
                            .add_enabled(
                                playing.lyrics_lookup.is_none(),
                                egui::Button::new("Search LRCLIB"),
                            )
                            .on_hover_text("Send this track’s title and artist to LRCLIB.")
                            .clicked();
                        if playing.lyrics_online {
                            save = ui.button("Save lyrics").clicked();
                        }
                    });
                    if !playing.lyrics_status.is_empty() {
                        ui.weak(&playing.lyrics_status);
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("lyrics")
                        .show(ui, |ui| {
                            if !playing.timed_lyrics.is_empty() {
                                let position = playing.sink.get_pos();
                                let current = playing
                                    .timed_lyrics
                                    .partition_point(|(timestamp, _)| *timestamp <= position)
                                    .checked_sub(1);
                                let id = ui.id().with(("lyric-line", playing.id));
                                let last = ctx.data_mut(|data| {
                                    let last = data.get_temp::<usize>(id);
                                    if let Some(current) = current {
                                        data.insert_temp(id, current);
                                    }
                                    last
                                });
                                for (index, (timestamp, line)) in
                                    playing.timed_lyrics.iter().enumerate()
                                {
                                    if current == Some(index) {
                                        let row = ui
                                            .add(
                                                egui::Label::new(RichText::new(line).strong())
                                                    .sense(egui::Sense::click()),
                                            )
                                            .on_hover_text(format!("Jump to {}", time(*timestamp)));
                                        if last != current {
                                            row.scroll_to_me(Some(egui::Align::Center));
                                        }
                                        if row.clicked() {
                                            lyric_seek = Some(*timestamp);
                                        }
                                    } else {
                                        let row = ui
                                            .add(egui::Label::new(line).sense(egui::Sense::click()))
                                            .on_hover_text(format!("Jump to {}", time(*timestamp)));
                                        if row.clicked() {
                                            lyric_seek = Some(*timestamp);
                                        }
                                    }
                                }
                            } else if playing.media.lyrics.is_empty() {
                                if playing.lyrics_status.is_empty() {
                                    ui.weak(
                                        "No local lyrics found. Use Search LRCLIB to look online.",
                                    );
                                }
                            } else {
                                ui.label(&playing.media.lyrics);
                            }
                            if (0..self.library.plugins.len())
                                .any(|index| !self.plugins.output(index).is_empty())
                            {
                                egui::CollapsingHeader::new("Plugin details").show(ui, |ui| {
                                    for (index, plugin) in self.library.plugins.iter().enumerate() {
                                        let output = self.plugins.output(index);
                                        if !output.is_empty() {
                                            ui.label(output).on_hover_text(plugin.name());
                                        }
                                    }
                                });
                            }
                        });
                } else {
                    ui.weak("Nothing playing");
                }
                if search {
                    self.lookup_lyrics(ctx);
                }
                if save {
                    self.save_lyrics();
                }
            });
        if let Some((pixels, title, artist, source)) = cover_open {
            self.open_cover_viewer(ctx, pixels, title, artist, source);
        }
        if let Some(timestamp) = lyric_seek {
            self.seek(timestamp, ctx);
        }
    }
}
