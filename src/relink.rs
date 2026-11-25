use crate::{Player, TrackId, TrackInfo, cue};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::mpsc,
};

pub fn exists(path: &Path) -> bool {
    if cue::reference(path).is_some() {
        cue::resolve(path)
            .ok()
            .flatten()
            .is_some_and(|t| t.audio.is_file())
    } else {
        path.is_file()
    }
}
type Change = (TrackId, PathBuf, PathBuf);
pub struct ResultSet {
    changes: Vec<Change>,
    unresolved: usize,
}
fn name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase()
}
fn choose(paths: &[PathBuf], info: Option<&TrackInfo>) -> Option<PathBuf> {
    if let [only] = paths {
        return Some(only.clone());
    }
    let old = info?;
    if old.artist.is_empty() && old.album.is_empty() {
        return None;
    }
    let mut matched = paths.iter().filter(|p| {
        let new = TrackInfo::read(p);
        [
            (&old.title, &new.title),
            (&old.album, &new.album),
            (&old.artist, &new.artist),
        ]
        .into_iter()
        .all(|(a, b)| a.is_empty() || a.eq_ignore_ascii_case(b))
    });
    let candidate = matched.next()?.clone();
    matched.next().is_none().then_some(candidate)
}
impl Player {
    pub fn find_missing(&mut self, list: usize, ctx: &eframe::egui::Context) {
        if self.relink.is_some() {
            return;
        }
        let Some(root) = rfd::FileDialog::new()
            .set_title("Find missing files in folder")
            .pick_folder()
        else {
            return;
        };
        let p = &self.library.playlists[list];
        let entries = p
            .ids
            .iter()
            .zip(&p.tracks)
            .enumerate()
            .map(|(i, (&id, path))| {
                (
                    id,
                    path.clone(),
                    self.views.get(&list).and_then(|v| v.tracks.get(i)).cloned(),
                )
            })
            .collect::<Vec<_>>();
        let (tx, rx) = mpsc::channel();
        self.relink = Some(rx);
        self.status = "Finding missing files…".into();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let missing: Vec<_> = entries.into_iter().filter(|(_, p, _)| !exists(p)).collect();
            let mut names: HashMap<String, Vec<PathBuf>> = missing
                .iter()
                .map(|(_, p, _)| {
                    (
                        name(cue::reference(p).map_or(p.as_path(), |(s, _)| s)),
                        Vec::new(),
                    )
                })
                .collect();
            if !names.is_empty() {
                for f in walkdir::WalkDir::new(root)
                    .into_iter()
                    .flatten()
                    .filter(|f| f.file_type().is_file())
                {
                    if let Some(candidates) = names.get_mut(&name(f.path())) {
                        candidates.push(f.into_path());
                    }
                }
            }
            let mut result = ResultSet {
                changes: Vec::new(),
                unresolved: 0,
            };
            for (id, old, info) in missing {
                let cue = cue::reference(&old);
                let mut choices = Vec::new();
                if let Some(candidates) = names.get(&name(cue.map_or(old.as_path(), |(s, _)| s))) {
                    choices.extend(
                        candidates
                            .iter()
                            .map(|p| cue.map_or_else(|| p.clone(), |(_, n)| p.join(n.to_string())))
                            .filter(|p| exists(p)),
                    );
                }
                if let Some(new) = choose(&choices, info.as_ref()) {
                    result.changes.push((id, old, new));
                } else {
                    result.unresolved += 1;
                }
            }
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }
    pub fn finish_relink(&mut self) {
        let Some(rx) = &self.relink else { return };
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(_) => {
                self.relink = None;
                self.status = "Folder scan interrupted.".into();
                return;
            }
        };
        self.relink = None;
        let mut lists = HashSet::new();
        let mut count = 0;
        for (id, old, new) in result.changes {
            if let Some(&(list, i)) = self.library.locations.get(&id)
                && self.library.playlists[list].tracks[i] == old
            {
                self.library.playlists[list].tracks[i] = new;
                lists.insert(list);
                count += 1;
            }
        }
        for list in lists {
            self.views.remove(&list);
            self.indexing.remove(&list);
        }
        self.status = format!("Relinked {count} files · {} unresolved", result.unresolved);
        if count > 0 {
            self.save();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unresolved_duplicates_are_never_guessed() {
        let paths: Vec<PathBuf> = ["a/music.wav", "b/music.wav"].map(Into::into).into();
        assert_eq!(choose(&paths, None), None);
        assert_eq!(choose(&paths[..1], None), Some(paths[0].clone()));
        assert!(choose(&[], None).is_none());
    }
}
