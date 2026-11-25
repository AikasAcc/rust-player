use crate::*;
impl Player {
    pub(crate) fn playlist_panel(&mut self, ctx: &egui::Context, requested_sort: Option<Sort>) {
        let background = self
            .theme_draft
            .as_ref()
            .map_or(&self.library.background, |draft| &draft.background)
            .clone();
        self.background.update(
            &background,
            self.playing.as_ref().and_then(Playing::cover),
            ctx,
        );
        egui::CentralPanel::default().show(ctx, |ui| {
            self.background.paint(&background, ui);
            let list = self.library.selected;
            let playlist = &mut self.library.playlists[list];
            let old_sort = (playlist.sort, playlist.descending);
            if let Some(sort) = requested_sort {
                playlist.sort = sort;
                playlist.descending = false;
            }
            ui.add_space(5.0);
            ui.horizontal(|ui| {
                let name_width = (ui.available_width() - 137.0).max(55.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(name_width, 48.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_max_width(name_width);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.add(
                            egui::Label::new(RichText::new(&playlist.name).strong().size(23.0))
                                .truncate(),
                        )
                        .on_hover_text(&playlist.name);
                        ui.weak(format!("{} tracks", playlist.tracks.len()));
                    },
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if sort_direction_button(ui, playlist.sort != Sort::Added, playlist.descending)
                    {
                        playlist.descending = !playlist.descending;
                    }
                    egui::ComboBox::from_id_salt("playlist sort")
                        .width(88.0)
                        .selected_text(playlist.sort.label())
                        .show_ui(ui, |ui| {
                            for sort in [
                                Sort::Added,
                                Sort::Title,
                                Sort::Artist,
                                Sort::Album,
                                Sort::Duration,
                            ] {
                                ui.selectable_value(&mut playlist.sort, sort, sort.label());
                            }
                        });
                });
            });
            if old_sort != (playlist.sort, playlist.descending) {
                if let Some(view) = self.views.remove(&list) {
                    self.views.insert(
                        list,
                        PlaylistView::build(view.tracks, playlist.sort, playlist.descending),
                    );
                }
                if !self.library.shuffle
                    && let Some(track) = self.order_track().filter(|track| track.0 == list)
                {
                    self.plan(track);
                }
                self.request_index(list, ctx);
                self.save();
            }
            let playlist = &self.library.playlists[list];
            ui.add_space(6.0);
            if self.indexing.contains_key(&list) {
                ui.weak("Reading track information…");
            }
            if playlist.tracks.is_empty() {
                ui.label("Drop audio files or folders here.");
            }
            let mut play = None;
            let mut edit = None;
            let mut properties = None;
            let mut find_missing = false;
            let mut folder = None;
            let (_, album_text) = self
                .theme_draft
                .as_ref()
                .map(|draft| draft.theme)
                .unwrap_or(self.library.theme)
                .album_colors();
            let view = self.views.get(&list);
            let query = self.track_search.trim();
            if query.is_empty() {
                self.search_results = None;
            } else if self
                .search_deadline
                .is_none_or(|deadline| Instant::now() >= deadline)
                && !self
                    .search_results
                    .as_ref()
                    .is_some_and(|cached| cached.list == list && cached.query == query)
            {
                self.search_results = Some(SearchResults::new(list, query, playlist, view));
            }
            let filtered_rows = self
                .search_results
                .as_ref()
                .filter(|cached| cached.list == list)
                .map(|results| {
                    ui.weak(format!("{} matching tracks", results.count));
                    &results.rows
                });
            let show_art = self.library.track_artwork;
            let height = ui
                .spacing()
                .interact_size
                .y
                .max(if show_art { 32.0 } else { 0.0 });
            let mut enqueue = None;
            let mut toggle = None;
            playlist_columns(
                ui,
                height,
                ["", "#", "Title", "Time"],
                true,
                false,
                "",
                show_art.then_some(None),
            );
            egui::ScrollArea::vertical().show_rows(
                ui,
                height,
                filtered_rows.map_or_else(
                    || {
                        view.map_or(playlist.tracks.len(), |view| {
                            view.row_count(playlist.tracks.len())
                        })
                    },
                    Vec::len,
                ),
                |ui, rows| {
                    for row_index in rows {
                        let row = filtered_rows
                            .and_then(|rows| rows.get(row_index))
                            .or_else(|| view.and_then(|view| view.rows.get(row_index)));
                        let index = match row {
                            Some(PlaylistRow::Album(album)) => {
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(ui.available_width(), height),
                                    egui::Sense::hover(),
                                );
                                let painter =
                                    ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
                                painter.hline(
                                    rect.x_range(),
                                    rect.bottom(),
                                    ui.visuals().window_stroke,
                                );
                                painter.text(
                                    rect.left_center() + egui::vec2(9.0, 0.0),
                                    egui::Align2::LEFT_CENTER,
                                    album,
                                    egui::FontId::proportional(13.0),
                                    album_text,
                                );
                                continue;
                            }
                            Some(PlaylistRow::Track(index)) => *index,
                            None => view.map_or(row_index, |view| {
                                view.tracks.len() + row_index - view.rows.len()
                            }),
                        };
                        let path = &playlist.tracks[index];
                        let active = self
                            .playing
                            .as_ref()
                            .is_some_and(|p| self.library.row_is_playing(list, index, p.id));
                        let metadata = view.and_then(|view| view.tracks.get(index));
                        let title = metadata
                            .map(|track| track.title.clone())
                            .unwrap_or_else(|| file_title(path));
                        let title = match metadata {
                            Some(track)
                                if !track.artist.is_empty()
                                    && track.artist != track.album_artist =>
                            {
                                format!("{title}  /  {}", track.artist)
                            }
                            _ => title,
                        };
                        let title = if metadata.is_some_and(|t| t.missing) {
                            format!("Missing · {title}")
                        } else {
                            title
                        };
                        let number = metadata.map(TrackInfo::track_number).unwrap_or_default();
                        let duration = metadata
                            .and_then(|track| track.duration)
                            .map(time)
                            .unwrap_or_default();
                        let marker = if active { "▶" } else { "" };
                        let response = playlist_columns(
                            ui,
                            height,
                            [marker, &number, &title, &duration],
                            false,
                            active,
                            &path.display().to_string(),
                            show_art.then(|| self.thumbs.get(ctx, path)),
                        );
                        let mut heart_hit = false;
                        if let Some(id) = self.library.track_id((list, index)) {
                            let heart = if self.library.is_favorite_id(id) {
                                RichText::new("♥").color(egui::Color32::from_rgb(255, 96, 128))
                            } else {
                                RichText::new("♡").weak()
                            };
                            let heart_response = ui.put(
                                favorite_column_rect(response.rect),
                                egui::Button::new(heart).frame(false),
                            );
                            heart_hit = heart_response.hovered();
                            if heart_response.on_hover_text("Toggle favorite").clicked() {
                                toggle = Some(id);
                            }
                        }
                        if response.double_clicked() && !heart_hit {
                            play = Some(index);
                        }
                        response.context_menu(|ui| {
                            if ui.button("Add to Play Queue").clicked() {
                                enqueue = Some((index, title.clone()));
                                ui.close();
                            }
                            ui.separator();
                            if ui
                                .add_enabled(
                                    self.relink.is_none(),
                                    egui::Button::new("Find missing files…"),
                                )
                                .clicked()
                            {
                                find_missing = true;
                                ui.close();
                            }
                            if ui.button("Properties").clicked() {
                                properties = Some(path.clone());
                                ui.close();
                            }
                            if ui.button("Open containing folder").clicked() {
                                folder = Some(path.clone());
                                ui.close();
                            }
                            ui.separator();
                            if ui.button("Remove from playlist").clicked() {
                                edit = Some((index, TrackEdit::Remove));
                                ui.close();
                            }
                            if playlist.sort == Sort::Added {
                                if ui
                                    .add_enabled(index > 0, egui::Button::new("Move up"))
                                    .clicked()
                                {
                                    edit = Some((index, TrackEdit::Move(index - 1)));
                                    ui.close();
                                }
                                if ui
                                    .add_enabled(
                                        index + 1 < playlist.tracks.len(),
                                        egui::Button::new("Move down"),
                                    )
                                    .clicked()
                                {
                                    edit = Some((index, TrackEdit::Move(index + 1)));
                                    ui.close();
                                }
                            }
                        });
                    }
                },
            );
            if let Some(id) = toggle {
                self.toggle_favorite_id(id);
            }
            if let Some((index, title)) = enqueue
                && let Some(id) = self.library.track_id((list, index))
            {
                self.play_queue.push(id, title);
                self.status.clear();
            }
            if let Some((index, change)) = edit {
                self.edit_track((list, index), change);
            } else if let Some(index) = play {
                self.start((list, index), ctx);
            }
            if let Some(path) = properties {
                self.open_track_properties(path, ctx);
            }
            if find_missing {
                self.find_missing(list, ctx);
            }
            if let Some(path) = folder {
                self.open_containing_folder(path);
            }
        });
    }
}
