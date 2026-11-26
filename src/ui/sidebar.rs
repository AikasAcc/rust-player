use crate::*;
impl Player {
    pub(crate) fn sidebar(&mut self, ctx: &egui::Context) -> Option<Sort> {
        let mut requested_sort = None;
        let mut play = None;
        egui::SidePanel::left("playlists")
            .default_width(147.0)
            .min_width(125.0)
            .frame(desk_surface(&ctx.style().visuals))
            .show(ctx, |ui| {
                ui.add_space(12.0);
                ui.label(RichText::new("LIBRARY").strong().small().weak());
                let by_album = self.library.playlists[self.library.selected].sort == Sort::Album;
                if ui
                    .selectable_label(!by_album, "♫  Tracks")
                    .on_hover_text("Show this playlist sorted by track title")
                    .clicked()
                {
                    requested_sort = Some(Sort::Title);
                }
                if ui
                    .selectable_label(by_album, "◎  Albums")
                    .on_hover_text("Show this playlist grouped by album")
                    .clicked()
                {
                    requested_sort = Some(Sort::Album);
                }
                let favorites_selected = self
                    .library
                    .favorites
                    .is_some_and(|index| self.library.selected == index);
                let favorites =
                    ui.add(egui::Button::selectable(favorites_selected, "♡  Favorites"));
                if favorites.clicked() {
                    let index = self.library.favorites_index_or_create();
                    self.library.selected = index;
                    self.save();
                }
                if favorites.double_clicked() {
                    let index = self.library.favorites_index_or_create();
                    self.library.selected = index;
                    play = Some(index);
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("PLAYLISTS").strong().small().weak());
                    if ui.button("+").on_hover_text("New playlist").clicked() {
                        self.dialogs.playlist_dialog = Some(PlaylistDialog::Name {
                            target: None,
                            name: String::new(),
                        });
                    }
                });
                let mut selected = self.library.selected;
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .small_button("Import…")
                        .on_hover_text("Import playlists")
                        .clicked()
                    {
                        self.import_playlists(ctx);
                        selected = self.library.selected;
                    }
                    if ui
                        .small_button("Activity")
                        .on_hover_text("Listening history and statistics")
                        .clicked()
                    {
                        self.dialogs.activity_open = true;
                    }
                });
                ui.separator();
                let mut action = None;
                let mut reorder = None;
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for index in self.library.sidebar_playlists() {
                        let list = &self.library.playlists[index];
                        let pinned = list.pinned || Some(index) == self.library.favorites;
                        let response = ui
                            .horizontal(|ui| {
                                let (rect, gutter) = ui.allocate_exact_size(
                                    egui::vec2(13.0, ui.spacing().interact_size.y),
                                    if pinned {
                                        egui::Sense::click()
                                    } else {
                                        egui::Sense::hover()
                                    },
                                );
                                if pinned {
                                    let center = rect.center();
                                    let stroke =
                                        egui::Stroke::new(1.3_f32, ui.visuals().text_color());
                                    for ((x1, y1), (x2, y2)) in [
                                        ((0.0, -5.0), (5.0, 0.0)),
                                        ((2.5, -2.5), (0.0, 0.0)),
                                        ((-2.5, -2.5), (2.5, 2.5)),
                                        ((0.0, 0.0), (-4.5, 4.5)),
                                    ] {
                                        ui.painter().line_segment(
                                            [
                                                center + egui::vec2(x1, y1),
                                                center + egui::vec2(x2, y2),
                                            ],
                                            stroke,
                                        );
                                    }
                                }
                                let sense = if Some(index) == self.library.favorites {
                                    egui::Sense::click()
                                } else {
                                    egui::Sense::click_and_drag()
                                };
                                ui.add_sized(
                                    [ui.available_width(), ui.spacing().interact_size.y],
                                    egui::Button::selectable(selected == index, &list.name)
                                        .wrap_mode(egui::TextWrapMode::Truncate)
                                        .sense(sense),
                                )
                                .on_hover_text(&list.name)
                                .union(gutter)
                            })
                            .inner;
                        if response.clicked() {
                            selected = index;
                        }
                        if response.double_clicked() {
                            selected = index;
                            play = Some(index);
                        }
                        if response.drag_started_by(egui::PointerButton::Primary) {
                            response.dnd_set_drag_payload(index);
                        }
                        if let Some(from) = response.dnd_hover_payload::<usize>()
                            && self.library.can_move_playlist(*from, index)
                            && let Some(pointer) = ctx.pointer_interact_pos()
                        {
                            let after = pointer.y >= response.rect.center().y;
                            let y = if after {
                                response.rect.bottom()
                            } else {
                                response.rect.top()
                            };
                            ui.painter().hline(
                                response.rect.left()..=ui.max_rect().right(),
                                y,
                                egui::Stroke::new(2.0_f32, ui.visuals().selection.stroke.color),
                            );
                            if response.dnd_release_payload::<usize>().is_some() {
                                reorder = Some((*from, index, after));
                            }
                        }
                        response.context_menu(|ui| {
                            if Some(index) != self.library.favorites {
                                let label = if list.pinned { "Unpin" } else { "Pin" };
                                if ui.button(label).clicked() {
                                    action = Some((index, PlaylistAction::TogglePin));
                                    ui.close();
                                }
                                ui.separator();
                            }
                            for (label, item) in [
                                ("Rename", PlaylistAction::Rename),
                                ("Copy", PlaylistAction::Copy),
                                ("Delete", PlaylistAction::Delete),
                                ("Export (absolute paths)", PlaylistAction::ExportAbsolute),
                                ("Export (relative paths)", PlaylistAction::ExportRelative),
                            ] {
                                if ui.button(label).clicked() {
                                    action = Some((index, item));
                                    ui.close();
                                }
                            }
                        });
                    }
                });
                let order_changed = reorder
                    .is_some_and(|(from, to, after)| self.library.move_playlist(from, to, after));
                if selected != self.library.selected || order_changed {
                    self.library.selected = selected;
                    self.save();
                }
                if order_changed {
                    // Snapshots refer to playlist positions, which just moved.
                    self.undo.clear();
                    self.redo.clear();
                }
                if let Some((index, action)) = action {
                    self.playlist_action(index, action, ctx);
                }
                if let Some(index) = play {
                    self.start_playlist(index, ctx);
                }
            });
        requested_sort
    }
}
