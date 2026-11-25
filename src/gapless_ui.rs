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
            let (target, track) = if let Some((id, track)) = self.play_queue.next() {
                let track = *self.library.locations.get(&track)?;
                (NextTarget::Queue(id, track), track)
            } else if self.library.repeat == Repeat::One {
                (NextTarget::Repeat(playing.track), playing.track)
            } else {
                let cursor = self.next_cursor()?;
                (
                    NextTarget::Playlist(cursor),
                    *self.library.locations.get(self.order.get(cursor)?)?,
                )
            };
            let path = self
                .library
                .playlists
                .get(track.0)?
                .tracks
                .get(track.1)?
                .clone();
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
        let resolve = |track| {
            SavedTrack {
                id: None,
                track,
                path: event.next.path.clone(),
                position: Duration::ZERO,
            }
            .resolve(&self.library)
        };
        let track = match event.next.target {
            NextTarget::Playlist(planned) => self.order_list.and_then(|list| {
                let tracks = &self.library.playlists[list].tracks;
                let matches = |cursor| {
                    self.order
                        .get(cursor)
                        .and_then(|id| self.library.locations.get(id))
                        .and_then(|&(_, i)| tracks.get(i))
                        == Some(&event.next.path)
                };
                let cursor = if matches(planned) {
                    planned
                } else {
                    (0..self.order.len()).find(|&cursor| matches(cursor))?
                };
                self.cursor = cursor;
                self.library.locations.get(&self.order[cursor]).copied()
            }),
            NextTarget::Queue(id, planned) => resolve(
                self.play_queue
                    .take(id)
                    .and_then(|id| self.library.locations.get(&id).copied())
                    .unwrap_or(planned),
            ),
            NextTarget::Repeat(track) => resolve(track),
        };
        let Some(track) = track else {
            self.advance_automatic(ctx);
            return;
        };
        self.record_play();
        if let Some(playing) = self.playing.take() {
            let position = playing.sink.get_pos();
            self.show_track(track, playing.sink, ctx);
            if let Some(playing) = &mut self.playing {
                playing.lastfm.rebase(event.listened_at_boundary);
            }
            self.save_playback(position);
            self.sync_presence();
            self.refresh_gapless();
        }
    }

    pub(super) fn advance_automatic(&mut self, ctx: &egui::Context) {
        let Some(current) = self.playing.as_ref().map(|playing| playing.track) else {
            return;
        };
        let mut warning = match self.play_next_queued(ctx) {
            Ok(()) => return,
            Err(warning) => warning,
        };
        if self.library.repeat == Repeat::One {
            match self.open(current, ctx, None) {
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
                let location = self.library.locations[&self.order[candidate]];
                match self.open(location, ctx, None) {
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
