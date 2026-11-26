use crate::{Player, Track, TrackId};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, io::ErrorKind, path::Path, time::Duration};

#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    id: u64,
    track: TrackId,
    title: String,
}

#[derive(Debug)]
pub(super) enum LoadError {
    /// No queue has been saved yet. This is a normal first start, not a
    /// corrupt file, so it must not trigger the backup path.
    Missing,
    Invalid(String),
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct PlayQueue {
    entries: VecDeque<Entry>,
    next_id: u64,
}

impl PlayQueue {
    pub fn load(path: &Path) -> Result<Self, LoadError> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => return Err(LoadError::Missing),
            Err(error) => return Err(LoadError::Invalid(error.to_string())),
        };
        let mut queue: Self = serde_json::from_slice(&bytes)
            .map_err(|error| LoadError::Invalid(error.to_string()))?;
        queue.next_id = queue
            .entries
            .iter()
            .map(|entry| entry.id.saturating_add(1))
            .max()
            .unwrap_or_default()
            .max(queue.next_id);
        Ok(queue)
    }

    pub fn push(&mut self, track: TrackId, title: String) {
        self.entries.push_back(Entry {
            id: self.next_id,
            track,
            title,
        });
        self.next_id += 1;
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn next(&self) -> Option<(u64, TrackId)> {
        self.entries.front().map(|entry| (entry.id, entry.track))
    }

    pub fn take(&mut self, id: u64) -> Option<TrackId> {
        let index = self.entries.iter().position(|entry| entry.id == id)?;
        self.entries.remove(index).map(|entry| entry.track)
    }

    fn position(&self, id: u64) -> Option<usize> {
        self.entries.iter().position(|entry| entry.id == id)
    }

    fn move_to(&mut self, id: u64, position: usize) {
        if position <= self.entries.len()
            && let Some(from) = self.position(id)
            && let Some(entry) = self.entries.remove(from)
        {
            self.entries.insert(position.min(self.entries.len()), entry);
        }
    }

    pub fn retain(&mut self, keep: impl Fn(TrackId) -> bool) {
        self.entries.retain(|e| keep(e.track));
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum QueueRow {
    Added(u64),
    Playlist(Track),
}

impl QueueRow {
    fn same_section(self, other: Self) -> bool {
        std::mem::discriminant(&self) == std::mem::discriminant(&other)
    }
}

enum QueueEdit {
    Remove(QueueRow),
    Move(QueueRow, usize),
}

/// Drag targets are row indices captured before the dragged row is removed.
/// Removing an earlier row shifts the target one to the left, so compensate to
/// keep dropping consistently *before* the target row. The context menu passes
/// post-removal indices already and therefore uses `Move` unchanged.
fn drop_target(from: Option<usize>, target: usize) -> usize {
    match from {
        Some(from) if from < target => target - 1,
        _ => target,
    }
}

impl Player {
    pub(super) fn clear_order(&mut self) {
        self.order_list = None;
        self.order.clear();
        self.cursor = usize::MAX;
        self.queue_order_edited = false;
    }

    /// Identity of the current order entry. Locations can move on a reorder;
    /// the ID does not.
    pub(super) fn order_id(&self) -> Option<TrackId> {
        self.order.get(self.cursor).copied()
    }

    pub(super) fn adjacent_cursor(&self, offset: isize) -> Option<usize> {
        // A removed anchor can leave remaining playlist entries before any cursor.
        let target = if self.cursor == usize::MAX && offset == 1 {
            0
        } else {
            self.cursor.checked_add_signed(offset)?
        };
        (target < self.order.len()).then_some(target)
    }

    pub(super) fn play_next_queued(&mut self, ctx: &egui::Context) -> Result<(), Option<String>> {
        let mut warning = None;
        let mut changed = false;
        while let Some(entry) = self.play_queue.entries.pop_front() {
            changed = true;
            if self.library.locate(entry.track).is_none() {
                continue;
            }
            match self.open(entry.track, ctx, None) {
                Ok(()) => {
                    self.status = warning.unwrap_or_default();
                    self.save_playback(Duration::ZERO);
                    self.save_queue();
                    self.sync_presence();
                    self.refresh_gapless();
                    return Ok(());
                }
                Err(error) => warning = Some(format!("Skipped queued track: {error}")),
            }
        }
        if changed {
            self.save_queue();
        }
        Err(warning)
    }

    pub(super) fn queue_count(&self) -> usize {
        self.play_queue.entries.len() + self.order.len() - self.upcoming_start()
    }

    fn upcoming_start(&self) -> usize {
        self.cursor
            .checked_add(1)
            .unwrap_or_default()
            .min(self.order.len())
    }

    /// Current position of a dragged row, before any removal happens.
    fn row_position(&self, row: QueueRow) -> Option<usize> {
        match row {
            QueueRow::Added(id) => self.play_queue.position(id),
            QueueRow::Playlist(entry) if self.order_list == Some(entry.0) => self
                .order
                .iter()
                .position(|&id| self.library.track_id(entry) == Some(id)),
            QueueRow::Playlist(_) => None,
        }
    }

    pub(super) fn queue_panel(&mut self, ui: &mut egui::Ui) {
        let queue = &self.play_queue;
        let start = self.upcoming_start();
        let added = queue.entries.len();
        let count = added + self.order.len() - start;
        let repeat = self.library.repeat;
        let mut edit = None;
        ui.vertical(|ui| {
            if repeat == crate::gapless_ui::Repeat::One && self.playing.is_some() {
                ui.weak("Repeat One plays the current song again after any added tracks.");
            } else if repeat == crate::gapless_ui::Repeat::All && self.order_list.is_some() {
                ui.weak("The playlist repeats after this pass.");
            }
            if count == 0 {
                ui.strong("No more tracks in this pass");
                ui.weak("Play a playlist or right-click a song to add it.");
                return;
            }
            let height = ui.text_style_height(&egui::TextStyle::Body)
                + ui.text_style_height(&egui::TextStyle::Small)
                + 9.0;
            let added_header = usize::from(added > 0);
            let playlist_header = usize::from(count > added);
            egui::ScrollArea::vertical()
                .id_salt("desk queue")
                .auto_shrink([false, false])
                .show_rows(
                    ui,
                    height,
                    count + added_header + playlist_header,
                    |ui, rows| {
                        for display_index in rows {
                            let section = if display_index == 0 && added_header != 0 {
                                Some("PLAY NEXT")
                            } else if display_index == added + added_header && playlist_header != 0
                            {
                                Some("FROM THE PLAYLIST")
                            } else {
                                None
                            };
                            if let Some(section) = section {
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(ui.available_width(), height),
                                    egui::Sense::hover(),
                                );
                                ui.painter().text(
                                    rect.left_bottom() - egui::vec2(0.0, 3.0),
                                    egui::Align2::LEFT_BOTTOM,
                                    section,
                                    egui::TextStyle::Small.resolve(ui.style()),
                                    ui.visuals().weak_text_color(),
                                );
                                continue;
                            }
                            let index = display_index
                                - added_header
                                - usize::from(display_index > added + added_header);
                            let (row, position, section_start, section_end, title) =
                                if index < added {
                                    let entry = &queue.entries[index];
                                    (
                                        QueueRow::Added(entry.id),
                                        index,
                                        0,
                                        added,
                                        std::borrow::Cow::Borrowed(entry.title.as_str()),
                                    )
                                } else {
                                    let Some(list) = self.order_list else {
                                        continue;
                                    };
                                    let position = start + index - added;
                                    let Some(id) = self.order.get(position).copied() else {
                                        continue;
                                    };
                                    let Some((_, track)) = self.library.locate(id) else {
                                        continue;
                                    };
                                    let title = self
                                        .views
                                        .get(&list)
                                        .and_then(|view| view.tracks.get(track))
                                        .map(|info| std::borrow::Cow::Borrowed(info.title.as_str()))
                                        .unwrap_or_else(|| {
                                            crate::file_title(
                                                &self.library.playlists[list].tracks[track],
                                            )
                                            .into()
                                        });
                                    (
                                        QueueRow::Playlist((list, track)),
                                        position,
                                        start,
                                        self.order.len(),
                                        title,
                                    )
                                };
                            let (list, track) = match row {
                                QueueRow::Added(_) => {
                                    let Some(location) =
                                        self.library.locate(queue.entries[index].track)
                                    else {
                                        continue;
                                    };
                                    location
                                }
                                QueueRow::Playlist(track) => track,
                            };
                            let metadata = self
                                .views
                                .get(&list)
                                .and_then(|view| view.tracks.get(track));
                            let artist = metadata.map_or("", |info| info.artist.as_str());
                            let duration = metadata.and_then(|info| info.duration).map(crate::time);
                            let title = metadata.map_or(title.as_ref(), |info| info.title.as_str());
                            let (rect, _) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), height),
                                egui::Sense::hover(),
                            );
                            let response = ui
                                .interact(rect, ui.id().with(row), egui::Sense::click_and_drag())
                                .on_hover_text(title);
                            let painter =
                                ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
                            let muted = ui.visuals().weak_text_color();
                            let small = egui::TextStyle::Small.resolve(ui.style());
                            let body = egui::TextStyle::Body.resolve(ui.style());
                            if response.hovered() {
                                painter.rect_filled(rect, 0.0, ui.visuals().faint_bg_color);
                            }
                            painter.hline(
                                rect.x_range(),
                                rect.bottom(),
                                ui.visuals().window_stroke,
                            );
                            for offset in [-3.0, 0.0, 3.0] {
                                painter.hline(
                                    rect.left() + 3.0..=rect.left() + 10.0,
                                    rect.center().y + offset,
                                    egui::Stroke::new(1.0_f32, muted),
                                );
                            }
                            let label = rect.left_center() + egui::vec2(20.0, 0.0);
                            let mut label_clip = rect;
                            if let Some(duration) = &duration {
                                painter.text(
                                    rect.right_center() - egui::vec2(3.0, 0.0),
                                    egui::Align2::RIGHT_CENTER,
                                    duration,
                                    small.clone(),
                                    muted,
                                );
                                label_clip.max.x -= 43.0;
                            }
                            let label_painter = painter.with_clip_rect(label_clip);
                            let align = if artist.is_empty() {
                                egui::Align2::LEFT_CENTER
                            } else {
                                egui::Align2::LEFT_BOTTOM
                            };
                            label_painter.text(
                                label,
                                align,
                                title,
                                body,
                                ui.visuals().text_color(),
                            );
                            if !artist.is_empty() {
                                label_painter.text(
                                    label,
                                    egui::Align2::LEFT_TOP,
                                    artist,
                                    small,
                                    muted,
                                );
                            }
                            response.dnd_set_drag_payload(row);
                            if let Some(source) = response.dnd_hover_payload::<QueueRow>()
                                && *source != row
                                && source.same_section(row)
                            {
                                ui.painter().rect_stroke(
                                    response.rect,
                                    3.0,
                                    ui.visuals().selection.stroke,
                                    egui::StrokeKind::Inside,
                                );
                                if response.dnd_release_payload::<QueueRow>().is_some() {
                                    let target = drop_target(self.row_position(*source), position);
                                    edit = Some(QueueEdit::Move(*source, target));
                                }
                            }
                            response.context_menu(|ui| {
                                if ui
                                    .add_enabled(
                                        position > section_start,
                                        egui::Button::new("Move up"),
                                    )
                                    .clicked()
                                {
                                    edit = Some(QueueEdit::Move(row, position - 1));
                                    ui.close();
                                }
                                if ui
                                    .add_enabled(
                                        position + 1 < section_end,
                                        egui::Button::new("Move down"),
                                    )
                                    .clicked()
                                {
                                    edit = Some(QueueEdit::Move(row, position + 1));
                                    ui.close();
                                }
                                ui.separator();
                                if ui.button("Remove from queue").clicked() {
                                    edit = Some(QueueEdit::Remove(row));
                                    ui.close();
                                }
                            });
                        }
                    },
                );
        });
        let Some(edit) = edit else { return };
        let (row, destination) = match edit {
            QueueEdit::Remove(row) => (row, None),
            QueueEdit::Move(row, position) => (row, Some(position)),
        };
        let added_row = matches!(row, QueueRow::Added(_));
        match row {
            QueueRow::Added(id) => {
                if let Some(position) = destination {
                    self.play_queue.move_to(id, position);
                } else {
                    self.play_queue.take(id);
                }
            }
            QueueRow::Playlist((list, track)) if self.order_list == Some(list) => {
                if let Some(offset) = self.order[start..]
                    .iter()
                    .position(|&queued| self.library.track_id((list, track)) == Some(queued))
                {
                    let track = self.order.remove(start + offset);
                    if let Some(position) = destination {
                        self.order.insert(position, track);
                    }
                    self.queue_order_edited = true;
                }
            }
            _ => {}
        }
        if added_row {
            self.save_queue();
        }
        self.refresh_gapless();
    }
}

#[cfg(test)]
mod tests {
    use super::{LoadError, PlayQueue, TrackId, drop_target};

    #[test]
    fn a_missing_queue_file_is_not_reported_as_corrupt() {
        let directory = tempfile::tempdir().unwrap();
        assert!(matches!(
            PlayQueue::load(&directory.path().join("queue.json")),
            Err(LoadError::Missing)
        ));
    }

    #[test]
    fn a_drop_before_the_source_keeps_its_target_index() {
        assert_eq!(drop_target(Some(3), 1), 1);
        assert_eq!(drop_target(Some(1), 1), 1);
        assert_eq!(drop_target(None, 2), 2);
    }

    #[test]
    fn a_drop_after_the_source_shifts_the_target_left() {
        assert_eq!(drop_target(Some(0), 2), 1);
        assert_eq!(drop_target(Some(1), 3), 2);
    }

    #[test]
    fn a_dropped_row_lands_before_the_target() {
        let mut queue = PlayQueue::default();
        queue.push(TrackId(1), "A".into());
        queue.push(TrackId(2), "B".into());
        queue.push(TrackId(3), "C".into());
        let a = queue.next().unwrap().0;
        let c = queue.entries[2].id;
        // Drag A onto C: from 0, target 2 becomes insert index 1.
        queue.move_to(a, drop_target(Some(0), 2));
        let titles: Vec<_> = queue.entries.iter().map(|entry| entry.track.0).collect();
        assert_eq!(titles, vec![2, 1, 3]);
        assert_eq!(c, queue.entries[2].id);
    }

    #[test]
    fn queue_round_trips_and_prunes_stale_tracks() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("queue.json");
        let mut queue = PlayQueue::default();
        queue.push(TrackId(1), "A".into());
        queue.push(TrackId(2), "B".into());
        std::fs::write(&path, serde_json::to_vec(&queue).unwrap()).unwrap();

        let mut loaded = PlayQueue::load(&path).unwrap();
        assert_eq!(loaded.next(), queue.next());
        loaded.retain(|id| id != TrackId(1));
        assert_eq!(loaded.next().unwrap().1, TrackId(2));
        queue.push(TrackId(3), "C".into());
        assert_ne!(queue.next().unwrap().0, loaded.next().unwrap().0);
    }

    #[test]
    fn load_repairs_a_stale_next_id() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("queue.json");
        let mut queue = PlayQueue::default();
        queue.push(TrackId(7), "A".into());
        let bytes = serde_json::to_vec(&queue).unwrap().to_vec();
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        value["next_id"] = serde_json::json!(0);
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();

        let mut loaded = PlayQueue::load(&path).unwrap();
        let existing = loaded.next().unwrap().0;
        loaded.push(TrackId(8), "B".into());
        assert_ne!(loaded.entries[1].id, existing);
    }

    #[test]
    fn reorder_and_remap_keep_duplicate_entries_independent() {
        let mut queue = PlayQueue::default();
        queue.push(TrackId(2), "First".into());
        let first = queue.next().unwrap().0;
        queue.push(TrackId(3), "Second".into());
        queue.push(TrackId(2), "First again".into());
        queue.move_to(first, 2);
        assert_eq!(queue.next().unwrap().1, TrackId(3));

        queue.retain(|id| id != TrackId(3));
        let (duplicate, track) = queue.next().unwrap();
        assert_ne!(duplicate, first);
        assert_eq!(track, TrackId(2));
        assert_eq!(queue.take(first), Some(track));
        assert_eq!(queue.next(), Some((duplicate, track)));
        assert_eq!(queue.take(duplicate), Some(track));
        assert!(queue.is_empty());
    }
}
