use eframe::egui::{ColorImage, Context, TextureHandle, TextureId, TextureOptions};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::mpsc,
};

pub struct Cache {
    jobs: mpsc::SyncSender<PathBuf>,
    results: mpsc::Receiver<(PathBuf, Option<ColorImage>)>,
    pending: HashSet<PathBuf>,
    images: HashMap<PathBuf, Option<TextureHandle>>,
    order: VecDeque<PathBuf>,
}

impl Cache {
    pub fn new(ctx: &Context) -> Self {
        let (jobs, input) = mpsc::sync_channel::<PathBuf>(32);
        let (output, results) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            for path in input {
                let image = thumbnail(&path);
                if output.send((path, image)).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
        Self {
            jobs,
            results,
            pending: Default::default(),
            images: Default::default(),
            order: Default::default(),
        }
    }
    pub fn get(&mut self, ctx: &Context, path: &Path) -> Option<TextureId> {
        while let Ok((path, image)) = self.results.try_recv() {
            self.pending.remove(&path);
            if self.order.len() >= 256
                && let Some(old) = self.order.pop_front()
            {
                self.images.remove(&old);
            }
            self.order.push_back(path.clone());
            self.images.insert(
                path,
                image.map(|img| ctx.load_texture("playlist cover", img, TextureOptions::LINEAR)),
            );
        }
        if let Some(img) = self.images.get(path) {
            return img.as_ref().map(TextureHandle::id);
        }
        if !self.pending.contains(path) && self.jobs.try_send(path.to_owned()).is_ok() {
            self.pending.insert(path.to_owned());
        }
        None
    }
}

fn thumbnail(path: &Path) -> Option<ColorImage> {
    let bytes = crate::library::Media::read(path).cover?;
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(96 * 1024 * 1024);
    reader.limits(limits);
    let image = reader
        .decode()
        .ok()?
        .resize_to_fill(48, 48, image::imageops::FilterType::Triangle)
        .to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied([48, 48], image.as_raw()))
}
