use crate::{
    Player, SavedTrack,
    audio::{Next, NextTarget},
};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub enum Repeat {
    #[default]
    Off,
    One,
    All,
}

impl Player {
    fn next_cursor(&self) -> Option<usize> {
        if self.order.is_empty() {
            return None;
        }
        match self.library.repeat {
            Repeat::All => self.adjacent_cursor(1).or(Some(0)),
            Repeat::Off | Repeat::One => self.adjacent_cursor(1),
        }
    }

    pub(super) fn refresh_gapless(&self) {
        let Some(playing) = &self.playing else {
            return;
        };
        let next = (|| {
            if !self.library.gapless {
                return None;
            }
            let (target, id) = if let Some((entry, queued)) = self.play_queue.next() {
                let track = self.library.locate(queued)?;
                (NextTarget::Queue(entry, track), queued)
            } else if self.library.repeat == Repeat::One {
                (NextTarget::Repeat(playing.track), playing.id)
            } else {
                let cursor = self.next_cursor()?;
                (NextTarget::Playlist(cursor), *self.order.get(cursor)?)
            };
            let path = self.library.track_path(id)?.to_path_buf();
            Some(Next {
                path,
                target,
                generation: self.gapless_generation,
            })
        })();
        playing.sink.queue_next(next);
    }

    pub(super) fn apply_gapless_transition(&mut self, ctx: &egui::Context) {
        let Some(playing) = &mut self.playing else {
            return;
        };
        let Some(event) = playing.sink.take_transition() else {
            return;
        };
        self.lastfm.observe(
            &mut playing.lastfm,
            &playing.media,
            playing.duration,
            event.listened_at_boundary,
            false,
        );
        let resolve = |library: &crate::Library, track: crate::Track| {
            SavedTrack {
                id: None,
                track,
                path: event.next.path.clone(),
                position: Duration::ZERO,
            }
            .resolve(library)
        };
        let id = match event.next.target {
            NextTarget::Playlist(planned) => self.order_list.and_then(|list| {
                let tracks = &self.library.playlists[list].tracks;
                let matches = |cursor| {
                    self.order
                        .get(cursor)
                        .and_then(|id| self.library.locate(*id))
                        .and_then(|(_, i)| tracks.get(i))
                        == Some(&event.next.path)
                };
                let cursor = if matches(planned) {
                    planned
                } else {
                    (0..self.order.len()).find(|&cursor| matches(cursor))?
                };
                self.cursor = cursor;
                self.order.get(cursor).copied()
            }),
            NextTarget::Queue(entry, planned) => {
                let taken = self.play_queue.take(entry);
                let library = &self.library;
                taken
                    .filter(|id| library.locate(*id).is_some())
                    .or_else(|| resolve(library, planned))
            }
            NextTarget::Repeat(track) => resolve(&self.library, track),
        };
        let Some(id) = id else {
            self.advance_automatic(ctx);
            return;
        };
        self.record_play();
        if let Some(playing) = self.playing.take() {
            let position = playing.sink.get_pos();
            self.show_track(id, playing.sink, ctx);
            if let Some(playing) = &mut self.playing {
                playing.lastfm.rebase(event.listened_at_boundary);
            }
            self.save_playback(position);
            self.sync_presence();
            self.refresh_gapless();
        }
    }

    pub(super) fn advance_automatic(&mut self, ctx: &egui::Context) {
        let Some(current) = self.playing.as_ref().map(|playing| playing.id) else {
            return;
        };
        let mut warning = match self.play_next_queued(ctx) {
            Ok(()) => return,
            Err(warning) => warning,
        };
        if self.library.repeat == Repeat::One {
            match self.open_id(current, ctx, None) {
                Ok(()) => {
                    self.status = warning.unwrap_or_default();
                    self.save_playback(Duration::ZERO);
                    self.sync_presence();
                    self.refresh_gapless();
                    return;
                }
                Err(error) => warning = Some(format!("Could not repeat track: {error}")),
            }
        }
        if self.order_list.is_some()
            && self.library.repeat == Repeat::All
            && self.adjacent_cursor(1).is_none()
        {
            for candidate in 0..self.order.len() {
                let Some(id) = self.order.get(candidate).copied() else {
                    continue;
                };
                match self.open_id(id, ctx, None) {
                    Ok(()) => {
                        self.cursor = candidate;
                        self.status = warning.unwrap_or_default();
                        self.save_playback(Duration::ZERO);
                        self.sync_presence();
                        self.refresh_gapless();
                        return;
                    }
                    Err(error) => {
                        warning = Some(format!("Skipped unreadable track: {error}"));
                    }
                }
            }
        }
        self.step_playlist(1, ctx, warning);
    }

    pub(super) fn gapless_settings(&mut self, ui: &mut egui::Ui) -> bool {
        let changed = ui
            .checkbox(&mut self.library.gapless, "Gapless playback")
            .changed();
        let old_repeat = self.library.repeat;
        egui::ComboBox::from_label("Repeat")
            .selected_text(match old_repeat {
                Repeat::Off => "Off",
                Repeat::One => "One track",
                Repeat::All => "Playlist",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.library.repeat, Repeat::Off, "Off");
                ui.selectable_value(&mut self.library.repeat, Repeat::One, "One track");
                ui.selectable_value(&mut self.library.repeat, Repeat::All, "Playlist");
            });
        changed || old_repeat != self.library.repeat
    }
}
