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
            if self.selection_list != Some(list) {
                self.track_selection.clear();
                self.track_anchor = None;
                self.selection_list = Some(list);
            }
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
                self.track_selection.clear();
                self.track_anchor = None;
                if let Some(view) = self.views.remove(&list) {
                    self.views.insert(
                        list,
                        PlaylistView::build(view.tracks, playlist.sort, playlist.descending),
                    );
                }
                if !self.library.shuffle
                    && self.order_list == Some(list)
                    && let Some(id) = self.order_id()
                {
                    self.plan(id);
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
            if self.facets.as_ref().is_none_or(|cache| cache.list != list) {
                let values = view
                    .map(|view| search::collect_facets(&view.tracks, 6))
                    .unwrap_or_default();
                self.facets = Some(FacetCache { list, values });
            }
            let toggled = self.facets.as_ref().and_then(|cache| {
                if cache.values.is_empty() {
                    return None;
                }
                let mut toggled = None;
                egui::ScrollArea::horizontal().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for facet in &cache.values {
                            let clause = facet.clause();
                            let active = self.track_search.contains(&clause);
                            let label = format!(
                                "{}: {} · {}",
                                facet.kind.label(),
                                facet.value,
                                facet.count
                            );
                            if ui
                                .selectable_label(active, label)
                                .on_hover_text(format!("Filter: {clause}"))
                                .clicked()
                            {
                                toggled = Some((facet.kind.field(), facet.value.clone()));
                            }
                        }
                    });
                });
                toggled
            });
            if let Some((field, value)) = toggled {
                self.track_search = search::toggle_clause(&self.track_search, field, &value);
            }
            let query = self.track_search.trim();
            if query.is_empty() {
                self.search_results = None;
            } else {
                // Search rows are shadows of playlist entries; selecting them
                // would make bulk actions operate on unrelated tracks later.
                self.track_selection.clear();
                self.track_anchor = None;
            }
            if !query.is_empty()
                && self
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
            // Reordering is only meaningful in "Date added" order, and only
            // when the visible rows map one-to-one to stored playlist entries.
            let can_reorder = playlist.smart_query.is_none()
                && playlist.sort == Sort::Added
                && filtered_rows.is_none()
                && view.is_none_or(|view| view.tracks.len() == playlist.tracks.len());
            let height = ui
                .spacing()
                .interact_size
                .y
                .max(if show_art { 32.0 } else { 0.0 });
            let mut enqueue_many = None;
            let mut favorite_many = None;
            let mut tag_many = None;
            let mut remove_many = None;
            playlist_columns(
                ui,
                height,
                ["", "#", "Title", "Time"],
                true,
                RowFlags::default(),
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
                        let album = filtered_rows
                            .and_then(|rows| rows.get(row_index))
                            .or_else(|| view.and_then(|view| view.rows.get(row_index)))
                            .and_then(|row| match row {
                                PlaylistRow::Album(label) => Some(label),
                                PlaylistRow::Track(_) => None,
                            });
                        if let Some(album) = album {
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
                        // A cached view can still describe rows of a shorter
                        // playlist after Favorites were pruned. Skip those
                        // rows instead of indexing past the end.
                        let Some(index) =
                            track_row_index(view, filtered_rows, row_index, playlist.tracks.len())
                        else {
                            continue;
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
                        let selected = self.track_selection.contains(&index);
                        let response = playlist_columns(
                            ui,
                            height,
                            [marker, &number, &title, &duration],
                            false,
                            RowFlags { active, selected },
                            &path.display().to_string(),
                            show_art.then(|| self.thumbs.get(ctx, path)),
                        );
                        if can_reorder {
                            response.dnd_set_drag_payload(index);
                            if let Some(source) = response.dnd_hover_payload::<usize>()
                                && *source != index
                            {
                                ui.painter().rect_stroke(
                                    response.rect,
                                    3.0,
                                    ui.visuals().selection.stroke,
                                    egui::StrokeKind::Inside,
                                );
                                if response.dnd_release_payload::<usize>().is_some() {
                                    // The dragged row is still present until the
                                    // drop is applied, so compensate here.
                                    let target = if *source < index { index - 1 } else { index };
                                    edit = Some((*source, TrackEdit::Reorder(target)));
                                }
                            }
                        }
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
                                favorite_many = Some(vec![index]);
                            }
                        }
                        if response.clicked() && !heart_hit {
                            let modifiers = ui.ctx().input(|state| state.modifiers);
                            if modifiers.shift
                                && let Some(anchor) = self.track_anchor
                            {
                                self.track_selection.clear();
                                for target in anchor.min(index)..=anchor.max(index) {
                                    self.track_selection.insert(target);
                                }
                            } else if modifiers.ctrl || modifiers.command {
                                if !self.track_selection.remove(&index) {
                                    self.track_selection.insert(index);
                                }
                                self.track_anchor = Some(index);
                            } else {
                                self.track_selection.clear();
                                self.track_selection.insert(index);
                                self.track_anchor = Some(index);
                            }
                        }
                        if response.double_clicked() && !heart_hit {
                            play = Some(index);
                        }
                        let mut targets: Vec<_> = if selected {
                            self.track_selection.iter().copied().collect()
                        } else {
                            vec![index]
                        };
                        targets.retain(|target| *target < playlist.tracks.len());
                        if targets.is_empty() {
                            targets.push(index);
                        }
                        let suffix = if targets.len() > 1 {
                            format!(" · {} tracks", targets.len())
                        } else {
                            String::new()
                        };
                        response.context_menu(|ui| {
                            if ui.button(format!("Add to Play Queue{suffix}")).clicked() {
                                enqueue_many = Some(targets.clone());
                                ui.close();
                            }
                            if ui.button(format!("Toggle favorite{suffix}")).clicked() {
                                favorite_many = Some(targets.clone());
                                ui.close();
                            }
                            if ui.button(format!("Edit tags…{suffix}")).clicked() {
                                tag_many = Some(targets.clone());
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
                            if playlist.smart_query.is_none()
                                && ui.button(format!("Remove from playlist{suffix}")).clicked()
                            {
                                remove_many = Some(targets.clone());
                                ui.close();
                            }
                            if playlist.smart_query.is_none() && playlist.sort == Sort::Added {
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
            let tag_paths = tag_many.map(|indices| {
                indices
                    .iter()
                    .filter_map(|&index| playlist.tracks.get(index).cloned())
                    .collect::<Vec<_>>()
            });
            if let Some(indices) = enqueue_many {
                self.enqueue_tracks(list, &indices);
            }
            if let Some(indices) = favorite_many {
                let ids: Vec<_> = indices
                    .iter()
                    .filter_map(|&index| self.library.track_id((list, index)))
                    .collect();
                for id in ids {
                    self.toggle_favorite_id(id);
                }
            }
            if let Some(paths) = tag_paths {
                self.open_tag_editor(paths);
            }
            if let Some(indices) = remove_many {
                self.remove_tracks(list, &indices);
                self.clear_track_selection();
            }
            if let Some((index, change)) = edit {
                self.edit_track((list, index), change);
                if matches!(change, TrackEdit::Reorder(_)) {
                    self.clear_track_selection();
                }
            } else if let Some(index) = play
                && let Some(id) = self.library.track_id((list, index))
            {
                self.start(id, ctx);
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

/// Resolve a visible row to a playlist index. `None` means an album header or
/// a stale row left by a cached view that no longer matches the playlist.
fn track_row_index(
    view: Option<&PlaylistView>,
    filtered: Option<&Vec<PlaylistRow>>,
    row: usize,
    total: usize,
) -> Option<usize> {
    let index = if let Some(rows) = filtered {
        match rows.get(row) {
            Some(PlaylistRow::Track(index)) => *index,
            Some(PlaylistRow::Album(_)) | None => return None,
        }
    } else if let Some(view) = view {
        match view.rows.get(row) {
            Some(PlaylistRow::Album(_)) => return None,
            Some(PlaylistRow::Track(index)) => *index,
            None => row
                .checked_sub(view.rows.len())?
                .checked_add(view.tracks.len())?,
        }
    } else {
        row
    };
    (index < total).then_some(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(album: &str) -> TrackInfo {
        TrackInfo {
            album: album.into(),
            ..Default::default()
        }
    }

    #[test]
    fn stale_view_rows_are_not_resolved_past_the_playlist() {
        let view = PlaylistView::build(vec![info("A"), info("A")], Sort::Album, false);
        assert_eq!(view.rows.len(), 3);
        assert!(matches!(view.rows[0], PlaylistRow::Album(_)));
        // Album header, then the two tracks of the cached view.
        assert_eq!(track_row_index(Some(&view), None, 0, 2), None);
        assert_eq!(track_row_index(Some(&view), None, 1, 2), Some(0));
        assert_eq!(track_row_index(Some(&view), None, 2, 2), Some(1));
        // A row for a track added after the view was built...
        assert_eq!(track_row_index(Some(&view), None, 3, 3), Some(2));
        // ...but not when the playlist was shortened instead.
        assert_eq!(track_row_index(Some(&view), None, 3, 2), None);
        assert_eq!(track_row_index(None, None, 1, 3), Some(1));
    }

    #[test]
    fn filtered_rows_win_over_a_stale_view() {
        let view = PlaylistView::build(vec![info("A"), info("B")], Sort::Album, false);
        let filtered = vec![PlaylistRow::Track(1), PlaylistRow::Track(0)];
        assert_eq!(track_row_index(Some(&view), Some(&filtered), 0, 2), Some(1));
        assert_eq!(track_row_index(Some(&view), Some(&filtered), 1, 2), Some(0));
        assert_eq!(track_row_index(Some(&view), Some(&filtered), 2, 2), None);
    }
}
