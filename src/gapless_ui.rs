use crate::{Player, audio::Next};
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
            Repeat::One => Some(self.cursor),
            Repeat::All => Some((self.cursor + 1) % self.order.len()),
            Repeat::Off => (self.cursor + 1 < self.order.len()).then_some(self.cursor + 1),
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
            let cursor = self.next_cursor()?;
            let index = *self.order.get(cursor)?;
            let path = self
                .library
                .playlists
                .get(playing.track.0)?
                .tracks
                .get(index)?
                .clone();
            Some(Next {
                path,
                cursor,
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
        let list = playing.track.0;
        let tracks = &self.library.playlists[list].tracks;
        let cursor = [
            Some(event.next.cursor),
            self.order
                .iter()
                .position(|&i| tracks.get(i) == Some(&event.next.path)),
        ]
        .into_iter()
        .flatten()
        .find(|&cursor| {
            self.order.get(cursor).and_then(|&i| tracks.get(i)) == Some(&event.next.path)
        });
        let Some(cursor) = cursor else {
            self.advance_automatic(ctx);
            return;
        };
        self.record_play();
        if let Some(playing) = self.playing.take() {
            self.cursor = cursor;
            let position = playing.sink.get_pos();
            self.show_track((list, self.order[cursor]), playing.sink, ctx);
            if let Some(playing) = &mut self.playing {
                playing.lastfm.rebase(event.listened_at_boundary);
            }
            self.save_playback(position);
            self.sync_presence();
            self.refresh_gapless();
        }
    }

    pub(super) fn advance_automatic(&mut self, ctx: &egui::Context) {
        let Some(playing) = &self.playing else {
            return;
        };
        let list = playing.track.0;
        let wrapped = self.next_cursor().filter(|&cursor| cursor <= self.cursor);
        if let Some(cursor) = wrapped {
            let end = if self.library.repeat == Repeat::One {
                cursor + 1
            } else {
                self.order.len()
            };
            let mut warning = String::new();
            for candidate in cursor..end {
                let index = self.order[candidate];
                match self.open((list, index), ctx, None) {
                    Ok(()) => {
                        self.cursor = candidate;
                        self.status = warning;
                        self.save_playback(Duration::ZERO);
                        self.sync_presence();
                        self.refresh_gapless();
                        return;
                    }
                    Err(error) => {
                        warning = format!("Skipped unreadable track: {error}");
                        self.status.clone_from(&warning);
                    }
                }
            }
        }
        self.step(1, ctx);
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
