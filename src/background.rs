use eframe::egui::{
    self, Color32, Mesh, Pos2, Rect, Shape, Vec2,
    epaint::{Vertex, WHITE_UV},
};
use image::{AnimationDecoder, DynamicImage, ImageFormat, ImageReader, Limits};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{self, BufReader, Cursor, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering::Relaxed},
        mpsc,
    },
    time::{Duration, Instant},
};

const EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff", "ico",
];
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_FRAME_BYTES: usize = 48 * 1024 * 1024;
const MAX_BLUR: u8 = 24;
const BLUR_SETTLE: Duration = Duration::from_millis(90);
const FRAME_INTERVAL: Duration = Duration::from_millis(34);
const BLUR_CYCLE: f64 = 6.0;
const LIQUID_CYCLE: f64 = 16.0;
const LIQUID_AMPLITUDE: f32 = 0.012;

pub fn supported(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            EXTENSIONS
                .iter()
                .any(|candidate| ext.eq_ignore_ascii_case(candidate))
        })
}

#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub enum Fit {
    #[default]
    Cover,
    Contain,
    Stretch,
    Tile,
    Center,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
enum SourceKind {
    #[default]
    ImageFile,
    CurrentCover,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub path: Option<PathBuf>,
    pub enabled: bool,
    source: SourceKind,
    fit: Fit,
    dim: f32,
    opacity: f32,
    blur: u8,
    animate_blur: bool,
    liquid: bool,
    liquid_strength: f32,
    liquid_speed: f32,
    pause_unfocused: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            path: None,
            enabled: false,
            source: SourceKind::ImageFile,
            fit: Fit::Cover,
            dim: 0.35,
            opacity: 1.0,
            blur: 0,
            animate_blur: false,
            liquid: false,
            liquid_strength: 0.6,
            liquid_speed: 1.0,
            pause_unfocused: true,
        }
    }
}

impl Settings {
    pub fn select_file(&mut self, path: PathBuf) {
        self.path = Some(path);
        self.source = SourceKind::ImageFile;
        self.enabled = true;
    }

    pub fn uses_file(&self) -> bool {
        self.source == SourceKind::ImageFile
    }

    pub fn settings(&mut self, ui: &mut egui::Ui, copy: &mut bool, status: &str) {
        ui.checkbox(&mut self.enabled, "Use a playlist background");
        egui::ComboBox::from_label("Background source")
            .selected_text(match self.source {
                SourceKind::ImageFile => "Image file",
                SourceKind::CurrentCover => "Current song cover",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.source, SourceKind::ImageFile, "Image file");
                if ui
                    .selectable_value(
                        &mut self.source,
                        SourceKind::CurrentCover,
                        "Current song cover",
                    )
                    .clicked()
                {
                    *copy = false;
                }
            });
        if self.uses_file() {
            ui.horizontal_wrapped(|ui| {
                if ui.button("Choose image…").clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Background images", EXTENSIONS)
                        .pick_file()
                {
                    self.select_file(path);
                }
                if ui
                    .add_enabled(self.path.is_some(), egui::Button::new("Remove background"))
                    .clicked()
                {
                    self.path = None;
                    self.enabled = false;
                    *copy = false;
                }
            });
            if let Some(path) = &self.path {
                ui.label(
                    path.file_name()
                        .unwrap_or(path.as_os_str())
                        .to_string_lossy(),
                );
                ui.checkbox(copy, "Copy the image into the player profile on Apply");
            }
            ui.weak("You can also drop an image while the Theme Editor is open.");
        }
        if self.enabled && (!self.uses_file() || self.path.is_some()) {
            egui::ComboBox::from_label("Image fit")
                .selected_text(self.fit.label())
                .show_ui(ui, |ui| {
                    for fit in [
                        Fit::Cover,
                        Fit::Contain,
                        Fit::Stretch,
                        Fit::Tile,
                        Fit::Center,
                    ] {
                        ui.selectable_value(&mut self.fit, fit, fit.label());
                    }
                });
            ui.add(egui::Slider::new(&mut self.dim, 0.0..=1.0).text("Darken"));
            ui.add(egui::Slider::new(&mut self.opacity, 0.0..=1.0).text("Opacity"));
            ui.add(
                egui::Slider::new(&mut self.blur, 0..=MAX_BLUR)
                    .text("Blur")
                    .suffix(" px"),
            );
            ui.checkbox(&mut self.animate_blur, "Animate blur");
            ui.checkbox(&mut self.liquid, "Liquid glass")
                .on_hover_text("Adds a slow refractive sheen, like light moving over glass.");
            if self.liquid {
                ui.add(
                    egui::Slider::new(&mut self.liquid_strength, 0.0..=1.0).text("Glass strength"),
                );
                ui.add(
                    egui::Slider::new(&mut self.liquid_speed, 0.1..=3.0)
                        .text("Flow speed")
                        .suffix("×"),
                );
            }
            ui.checkbox(
                &mut self.pause_unfocused,
                "Pause animation when the player loses focus",
            );
            if !status.is_empty() {
                ui.weak(status);
            }
        }
    }
}

impl Fit {
    fn label(self) -> &'static str {
        match self {
            Self::Cover => "Cover",
            Self::Contain => "Contain",
            Self::Stretch => "Stretch",
            Self::Tile => "Tile",
            Self::Center => "Center",
        }
    }
}

pub fn copy_to_profile(source: &Path, library_path: &Path) -> io::Result<PathBuf> {
    let folder = library_path
        .parent()
        .expect("profile directory")
        .join("backgrounds");
    fs::create_dir_all(&folder)?;
    let source = source.canonicalize()?;
    if source.starts_with(folder.canonicalize()?) {
        return Ok(source);
    }
    let mut image = File::open(&source)?;
    if image.metadata()?.len() > MAX_FILE_BYTES {
        return Err(io::Error::other("background files are limited to 64 MiB"));
    }
    let suffix = format!(
        ".{}",
        source
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("img")
    );
    let mut temporary = tempfile::Builder::new()
        .prefix("background-")
        .suffix(&suffix)
        .tempfile_in(folder)?;
    io::copy(&mut image, &mut temporary)?;
    let (_, path) = temporary.keep()?;
    Ok(path)
}

pub struct Cover {
    pub texture: egui::TextureHandle,
    pixels: Arc<image::RgbaImage>,
}

impl Cover {
    pub fn new(ctx: &egui::Context, pixels: image::RgbaImage) -> Self {
        Self {
            texture: texture(ctx, &pixels),
            pixels: Arc::new(pixels),
        }
    }

    pub fn pixels(&self) -> Arc<image::RgbaImage> {
        Arc::clone(&self.pixels)
    }

    pub fn save_png(&self, path: &Path) -> image::ImageResult<()> {
        self.pixels.save_with_format(path, ImageFormat::Png)
    }

    pub fn save_sidecar(&self, source: &Path) -> Result<PathBuf, String> {
        save_sidecar_png(&self.pixels, source)
    }
}

/// Write `cover.png` next to a track without overwriting an existing file.
pub fn save_sidecar_png(pixels: &image::RgbaImage, source: &Path) -> Result<PathBuf, String> {
    let dir = source
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let path = dir.join("cover.png");
    if path.exists() {
        return Err(format!("{} already exists", path.display()));
    }
    let mut bytes = Vec::new();
    pixels
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .map_err(|error| error.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(dir).map_err(|error| error.to_string())?;
    file.write_all(&bytes)
        .and_then(|()| file.flush())
        .map_err(|error| error.to_string())?;
    file.persist_noclobber(&path)
        .map_err(|error| error.to_string())?;
    Ok(path)
}

#[derive(Clone)]
enum Source {
    File(PathBuf),
    Cover(Arc<image::RgbaImage>),
}

impl PartialEq for Source {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::File(left), Self::File(right)) => left == right,
            (Self::Cover(left), Self::Cover(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq)]
struct Effect {
    blur: u8,
    animated: bool,
}

struct Frame {
    base: egui::TextureHandle,
    blurred: Option<egui::TextureHandle>,
}

struct Clip {
    frames: Vec<Frame>,
    ends: Vec<f64>,
    seconds: f64,
}

#[derive(Default)]
pub struct Preview {
    source: Option<Source>,
    effect: Effect,
    reload_at: Option<Instant>,
    receiver: Option<mpsc::Receiver<Result<Clip, String>>>,
    cancellation: Option<Arc<AtomicBool>>,
    clip: Option<Clip>,
    frame: usize,
    elapsed: f64,
    motion_time: f64,
    liquid_time: f64,
    updated_at: Option<Instant>,
    status: String,
}

impl Drop for Preview {
    fn drop(&mut self) {
        if let Some(cancel) = &self.cancellation {
            cancel.store(true, Relaxed);
        }
    }
}

impl Preview {
    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn update(&mut self, settings: &Settings, cover: Option<&Cover>, ctx: &egui::Context) {
        let source = if !settings.enabled {
            None
        } else {
            match settings.source {
                SourceKind::ImageFile => settings.path.clone().map(Source::File),
                SourceKind::CurrentCover => {
                    cover.map(|cover| Source::Cover(Arc::clone(&cover.pixels)))
                }
            }
        };
        let effect = Effect {
            blur: settings.blur.min(MAX_BLUR),
            animated: settings.animate_blur,
        };
        let changed_source = source != self.source;
        if changed_source || (source.is_some() && effect != self.effect) {
            if let Some(cancel) = self.cancellation.take() {
                cancel.store(true, Relaxed);
            }
            self.source = source;
            self.effect = effect;
            self.receiver = None;
            if changed_source {
                self.clip = None;
                self.frame = 0;
                self.elapsed = 0.0;
                self.motion_time = 0.0;
                self.liquid_time = 0.0;
                self.updated_at = None;
            }
            self.status.clear();
            self.reload_at = self.source.as_ref().map(|_| {
                Instant::now()
                    + if changed_source {
                        Duration::ZERO
                    } else {
                        BLUR_SETTLE
                    }
            });
        }
        if let Some(deadline) = self.reload_at {
            let wait = deadline.saturating_duration_since(Instant::now());
            if !wait.is_zero() {
                ctx.request_repaint_after(wait);
            } else if let Some(source) = self.source.clone() {
                self.reload_at = None;
                let (tx, rx) = mpsc::channel();
                let cancel = Arc::new(AtomicBool::new(false));
                self.cancellation = Some(Arc::clone(&cancel));
                self.receiver = Some(rx);
                self.status = if self.clip.is_some() {
                    "Rendering blur…"
                } else {
                    "Loading background…"
                }
                .into();
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    if cancel.load(Relaxed) {
                        return;
                    }
                    let mut result = match source {
                        Source::File(path) => load(&path, effect, &ctx, &cancel),
                        Source::Cover(pixels) => Ok(still_clip(&ctx, (*pixels).clone(), effect)),
                    };
                    if let Err(error) = &mut result {
                        *error = format!("Could not load background: {error}");
                    }
                    if tx.send(result).is_ok() {
                        ctx.request_repaint();
                    }
                });
            }
        }
        if let Some(receiver) = &self.receiver {
            match receiver.try_recv() {
                Ok(result) => {
                    self.receiver = None;
                    match result {
                        Ok(clip) => {
                            self.frame = self.frame.min(clip.frames.len() - 1);
                            self.clip = Some(clip);
                            self.status.clear();
                        }
                        Err(error) => self.status = error,
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.receiver = None;
                    self.status = "Background decoding was interrupted.".into();
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        let Some(clip) = &self.clip else {
            if settings.enabled
                && settings.source == SourceKind::CurrentCover
                && self.source.is_none()
            {
                self.status = "No cover artwork for the current song.".into();
            } else if self.source.is_none() {
                self.status.clear();
            }
            return;
        };
        let strength = settings.liquid_strength.clamp(0.0, 1.0);
        let speed = settings.liquid_speed.clamp(0.0, 3.0);
        let liquid = settings.liquid && strength > 0.0 && speed > 0.0;
        let motion = settings.animate_blur && clip.frames[self.frame].blurred.is_some();
        let animated_image = clip.frames.len() > 1;
        if !animated_image && !motion && !liquid {
            self.updated_at = None;
            return;
        }
        let paused = ctx.input(|input| {
            input.viewport().minimized == Some(true) || (settings.pause_unfocused && !input.focused)
        });
        if paused {
            self.updated_at = None;
            return;
        }
        let now = Instant::now();
        if let Some(previous) = self.updated_at.replace(now) {
            let delta = now.duration_since(previous).as_secs_f64();
            if animated_image {
                self.elapsed = (self.elapsed + delta) % clip.seconds;
            }
            if motion {
                self.motion_time = (self.motion_time + delta) % BLUR_CYCLE;
            }
            if liquid {
                // Never wrap this phase: a modulo restart makes the sheen jump.
                self.liquid_time += delta * f64::from(speed);
            }
        }
        if animated_image {
            self.frame = clip
                .ends
                .partition_point(|&end| end <= self.elapsed)
                .min(clip.frames.len() - 1);
        }
        if motion || liquid {
            ctx.request_repaint_after(FRAME_INTERVAL);
        } else {
            let wait = Duration::from_secs_f64((clip.ends[self.frame] - self.elapsed).max(0.0));
            ctx.request_repaint_after(wait.max(FRAME_INTERVAL));
        }
    }

    pub fn paint(&self, settings: &Settings, ui: &egui::Ui) {
        let Some(frame) = self
            .clip
            .as_ref()
            .and_then(|clip| clip.frames.get(self.frame))
        else {
            return;
        };
        let rect = ui.max_rect();
        if !rect.is_positive() {
            return;
        }
        let painter = ui.painter().with_clip_rect(rect);
        let size = frame.base.size_vec2();
        let full = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        let opacity = settings.opacity.clamp(0.0, 1.0);
        let blurred = frame.blurred.as_ref().filter(|_| settings.animate_blur);
        let (base, overlay) = blend_opacity(
            opacity,
            blurred.map_or(0.0, |_| blur_weight(self.motion_time)),
        );
        let strength = settings.liquid_strength.clamp(0.0, 1.0);
        let liquid = settings.liquid && strength > 0.0;
        let phase = (self.liquid_time / LIQUID_CYCLE) as f32;
        let render_layer = |id, area, uv, color| {
            if liquid {
                painter.add(Shape::Mesh(Arc::new(liquid_mesh(
                    id, area, uv, color, phase, strength,
                ))));
            } else {
                painter.image(id, area, uv, color);
            }
        };
        let render = |area, uv| {
            if base > 0.0 {
                render_layer(frame.base.id(), area, uv, tint(base));
            }
            if let Some(blurred) = blurred
                && overlay > 0.0
            {
                render_layer(blurred.id(), area, uv, tint(overlay));
            }
        };
        let scale = rect.size() / size;
        match settings.fit {
            Fit::Cover => {
                let crop = scale / scale.x.max(scale.y);
                render(rect, Rect::from_center_size(Pos2::new(0.5, 0.5), crop));
            }
            Fit::Contain => render(
                Rect::from_center_size(rect.center(), size * scale.x.min(scale.y)),
                full,
            ),
            Fit::Stretch => render(rect, full),
            Fit::Center => render(Rect::from_center_size(rect.center(), size), full),
            Fit::Tile => {
                let tile = size * (64.0 / size.x.min(size.y)).max(1.0);
                for y in 0..(rect.height() / tile.y).ceil().min(80.0) as u32 {
                    for x in 0..(rect.width() / tile.x).ceil().min(120.0) as u32 {
                        render(
                            Rect::from_min_size(
                                rect.min + Vec2::new(x as f32 * tile.x, y as f32 * tile.y),
                                tile,
                            ),
                            full,
                        );
                    }
                }
            }
        }
        painter.rect_filled(
            rect,
            0.0,
            Color32::from_black_alpha((settings.dim.clamp(0.0, 1.0) * 255.0) as u8),
        );
        if liquid {
            painter.add(Shape::Mesh(Arc::new(glass_mesh(rect, phase, strength))));
        }
    }
}

fn tint(opacity: f32) -> Color32 {
    Color32::from_white_alpha((opacity * 255.0).round() as u8)
}

/// Build a lightly distorted copy of an image draw call. The on-screen quad
/// stays in place while its texture coordinates flow, so there are no gaps at
/// the edges and no extra texture uploads per frame.
fn liquid_mesh(
    texture: egui::TextureId,
    area: Rect,
    uv: Rect,
    color: Color32,
    phase: f32,
    strength: f32,
) -> Mesh {
    let columns = ((area.width() / 28.0).round() as u32).clamp(2, 48);
    let rows = ((area.height() / 28.0).round() as u32).clamp(2, 32);
    let mut mesh = Mesh::with_texture(texture);
    mesh.reserve_vertices(((columns + 1) * (rows + 1)) as usize);
    mesh.reserve_triangles((columns * rows * 2) as usize);

    for row in 0..=rows {
        let t = row as f32 / rows as f32;
        for column in 0..=columns {
            let s = column as f32 / columns as f32;
            mesh.vertices.push(Vertex {
                pos: Pos2::new(
                    area.min.x + area.width() * s,
                    area.min.y + area.height() * t,
                ),
                uv: displaced_uv(uv, s, t, phase, strength),
                color,
            });
        }
    }

    let stride = columns + 1;
    for row in 0..rows {
        for column in 0..columns {
            let index = row * stride + column;
            mesh.add_triangle(index, index + 1, index + stride);
            mesh.add_triangle(index + stride, index + 1, index + stride + 1);
        }
    }
    mesh
}

fn displaced_uv(uv: Rect, s: f32, t: f32, phase: f32, strength: f32) -> Pos2 {
    let tau = std::f32::consts::TAU;
    let horizontal = 0.7 * (tau * (t * 0.8 + phase)).sin()
        + 0.3 * (tau * (s * 0.45 - t * 0.3 + phase * 0.6)).sin();
    let vertical = 0.7 * (tau * (s * 0.7 - phase * 0.8)).sin()
        + 0.3 * (tau * (s * 0.3 + t * 0.5 + phase * 0.5)).sin();
    let base_x = uv.min.x + s * uv.width();
    let base_y = uv.min.y + t * uv.height();
    Pos2::new(
        (base_x + horizontal * strength * LIQUID_AMPLITUDE * uv.width()).clamp(uv.min.x, uv.max.x),
        (base_y + vertical * strength * LIQUID_AMPLITUDE * uv.height()).clamp(uv.min.y, uv.max.y),
    )
}

/// A translucent highlight mesh drawn above the darkened wallpaper. The alpha
/// gradient forms a soft diagonal sheen plus a faint rim light, which reads as
/// glossy glass instead of moving water.
fn glass_mesh(area: Rect, phase: f32, strength: f32) -> Mesh {
    let columns = ((area.width() / 32.0).round() as u32).clamp(2, 40);
    let rows = ((area.height() / 32.0).round() as u32).clamp(2, 28);
    let mut mesh = Mesh::with_texture(egui::TextureId::default());
    mesh.reserve_vertices(((columns + 1) * (rows + 1)) as usize);
    mesh.reserve_triangles((columns * rows * 2) as usize);

    for row in 0..=rows {
        let t = row as f32 / rows as f32;
        for column in 0..=columns {
            let s = column as f32 / columns as f32;
            mesh.vertices.push(Vertex {
                pos: Pos2::new(
                    area.min.x + area.width() * s,
                    area.min.y + area.height() * t,
                ),
                uv: WHITE_UV,
                color: glass_tint(s, t, phase, strength),
            });
        }
    }

    let stride = columns + 1;
    for row in 0..rows {
        for column in 0..columns {
            let index = row * stride + column;
            mesh.add_triangle(index, index + 1, index + stride);
            mesh.add_triangle(index + stride, index + 1, index + stride + 1);
        }
    }
    mesh
}

fn glass_tint(s: f32, t: f32, phase: f32, strength: f32) -> Color32 {
    let tau = std::f32::consts::TAU;
    let diagonal = s * 0.7 + t * 0.45 - phase * 0.22;
    let sheen = (0.5 + 0.5 * (tau * diagonal).cos()).powf(10.0);
    let edge_x = (s * (1.0 - s) * 4.0).max(0.0).sqrt();
    let edge_y = (t * (1.0 - t) * 4.0).max(0.0).sqrt();
    let rim = (1.0 - edge_x * edge_y).powf(4.0);
    let alpha = ((0.14 * sheen + 0.07 * rim) * strength * 255.0)
        .round()
        .clamp(0.0, 255.0) as u8;
    Color32::from_rgba_unmultiplied(210, 235, 255, alpha)
}

fn blur_weight(time: f64) -> f32 {
    (0.5 - 0.5 * (std::f64::consts::TAU * time / BLUR_CYCLE).cos()) as f32
}

fn blend_opacity(opacity: f32, amount: f32) -> (f32, f32) {
    // Keep the combined opacity stable for opaque artwork across the two layers.
    let overlay = amount * opacity;
    let base = if overlay == 1.0 {
        0.0
    } else {
        (opacity - overlay) / (1.0 - overlay)
    };
    (base, overlay)
}

fn reader(path: &Path) -> io::Result<ImageReader<BufReader<File>>> {
    ImageReader::open(path)?.with_guessed_format()
}

fn limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    limits
}

fn load(
    path: &Path,
    effect: Effect,
    ctx: &egui::Context,
    cancel: &AtomicBool,
) -> Result<Clip, String> {
    let load_error = |error: image::ImageError| error.to_string();
    if fs::metadata(path).map_err(|error| error.to_string())?.len() > MAX_FILE_BYTES {
        return Err("choose a background file under 64 MiB.".into());
    }
    let mut still = reader(path).map_err(|error| error.to_string())?;
    let animated = match still.format() {
        Some(ImageFormat::Gif) => {
            let mut decoder =
                image::codecs::gif::GifDecoder::new(still.into_inner()).map_err(load_error)?;
            image::ImageDecoder::set_limits(&mut decoder, limits()).map_err(load_error)?;
            return collect_frames(decoder.into_frames(), effect, ctx, cancel);
        }
        Some(ImageFormat::WebP) => {
            let mut decoder = image::codecs::webp::WebPDecoder::new(
                reader(path)
                    .map_err(|error| error.to_string())?
                    .into_inner(),
            )
            .map_err(load_error)?;
            image::ImageDecoder::set_limits(&mut decoder, limits()).map_err(load_error)?;
            if decoder.has_animation() {
                Some(decoder.into_frames())
            } else {
                None
            }
        }
        _ => None,
    };
    if let Some(frames) = animated {
        return collect_frames(frames, effect, ctx, cancel);
    }
    still.limits(limits());
    let rgba = downscale(still.decode().map_err(load_error)?, 1920, 1080);
    if cancel.load(Relaxed) {
        return Err("selection changed.".into());
    }
    Ok(still_clip(ctx, rgba, effect))
}

fn still_clip(ctx: &egui::Context, rgba: image::RgbaImage, effect: Effect) -> Clip {
    Clip {
        frames: vec![prepare_frame(ctx, rgba, effect)],
        ends: vec![0.0],
        seconds: 0.0,
    }
}

fn collect_frames(
    frames: image::Frames<'_>,
    effect: Effect,
    ctx: &egui::Context,
    cancel: &AtomicBool,
) -> Result<Clip, String> {
    let mut clip = Clip {
        frames: Vec::new(),
        ends: Vec::new(),
        seconds: 0.0,
    };
    let mut bytes = 0;
    for frame in frames {
        if cancel.load(Relaxed) {
            return Err("selection changed.".into());
        }
        let frame = frame.map_err(|error| error.to_string())?;
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        let seconds = f64::from(numerator) / f64::from(denominator.max(1)) / 1_000.0;
        let rgba = downscale(DynamicImage::ImageRgba8(frame.into_buffer()), 800, 600);
        bytes += rgba.as_raw().len();
        if bytes > MAX_FRAME_BYTES || clip.frames.len() >= 600 {
            return Err(
                "the animation exceeds the 48 MiB or 600-frame limit. Choose a smaller animation."
                    .into(),
            );
        }
        clip.seconds += if seconds > 0.0 { seconds } else { 0.1 };
        clip.ends.push(clip.seconds);
        clip.frames.push(prepare_frame(ctx, rgba, effect));
    }
    if clip.frames.is_empty() {
        return Err("the file contains no usable animation frames.".into());
    }
    Ok(clip)
}

fn prepare_frame(ctx: &egui::Context, rgba: image::RgbaImage, effect: Effect) -> Frame {
    let blurred = effect.animated.then(|| {
        // Half resolution bounds the extra cache for animated GIF and WebP frames.
        let width = (rgba.width() / 2).max(1);
        let height = (rgba.height() / 2).max(1);
        let reduced =
            image::imageops::resize(&rgba, width, height, image::imageops::FilterType::Triangle);
        texture(ctx, &blur_image(reduced, effect.blur / 2 + 9))
    });
    Frame {
        base: texture(ctx, &blur_image(rgba, effect.blur)),
        blurred,
    }
}

fn blur_image(mut rgba: image::RgbaImage, amount: u8) -> image::RgbaImage {
    if amount == 0 {
        return rgba;
    }
    let sigma = f32::from(amount);
    if rgba.pixels().all(|pixel| pixel.0[3] == 255) {
        return image::imageops::fast_blur(&rgba, sigma);
    }
    // Blur premultiplied colors so transparent pixels do not bleed into visible edges.
    for pixel in rgba.pixels_mut() {
        let [red, green, blue, alpha] = pixel.0;
        let scale = |value| ((u16::from(value) * u16::from(alpha) + 127) / 255) as u8;
        pixel.0 = [scale(red), scale(green), scale(blue), alpha];
    }
    let mut rgba = image::imageops::fast_blur(&rgba, sigma);
    for pixel in rgba.pixels_mut() {
        let [red, green, blue, alpha] = pixel.0;
        if alpha > 0 {
            let scale = |value| (u16::from(value) * 255 / u16::from(alpha)).min(255) as u8;
            pixel.0 = [scale(red), scale(green), scale(blue), alpha];
        }
    }
    rgba
}

fn texture(ctx: &egui::Context, rgba: &image::RgbaImage) -> egui::TextureHandle {
    let size = [rgba.width() as usize, rgba.height() as usize];
    let image = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
    ctx.load_texture("Playlist background", image, egui::TextureOptions::LINEAR)
}

fn downscale(image: DynamicImage, max_width: u32, max_height: u32) -> image::RgbaImage {
    if image.width() > max_width || image.height() > max_height {
        image.thumbnail(max_width, max_height).into_rgba8()
    } else {
        image.into_rgba8()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Delay, Frame, Rgba, RgbaImage, codecs::gif::GifEncoder};

    #[test]
    fn sidecar_cover_is_written_once_and_never_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let music = directory.path().join("music");
        fs::create_dir(&music).unwrap();
        let source = music.join("track.flac");
        let pixels = RgbaImage::from_pixel(2, 2, Rgba([10, 20, 30, 255]));

        let path = save_sidecar_png(&pixels, &source).unwrap();
        assert_eq!(path, music.join("cover.png"));
        let first = fs::read(&path).unwrap();
        assert!(!first.is_empty());
        assert!(save_sidecar_png(&pixels, &source).is_err());
        assert_eq!(fs::read(&path).unwrap(), first);
    }

    #[test]
    fn cover_source_tracks_artwork_identity_and_clears_without_a_cover() {
        let ctx = egui::Context::default();
        let first = Cover::new(&ctx, RgbaImage::from_pixel(20, 20, Rgba([0, 100, 0, 255])));
        let second = Cover::new(&ctx, RgbaImage::from_pixel(20, 20, Rgba([100, 0, 0, 255])));
        let settings = Settings {
            enabled: true,
            source: SourceKind::CurrentCover,
            animate_blur: true,
            ..Settings::default()
        };
        let mut preview = Preview::default();
        preview.update(&settings, Some(&first), &ctx);
        let original = preview.source.clone();
        preview.update(&settings, Some(&first), &ctx);
        assert!(original == preview.source);
        preview.update(&settings, Some(&second), &ctx);
        assert!(original != preview.source);
        preview.update(&settings, None, &ctx);
        assert!(preview.source.is_none());
        assert!(preview.clip.is_none());
        assert_eq!(preview.status(), "No cover artwork for the current song.");
    }

    #[test]
    fn blur_cycles_back_and_keeps_opaque_wallpaper_at_the_selected_opacity() {
        assert_eq!(blur_weight(0.0), 0.0);
        assert_eq!(blur_weight(BLUR_CYCLE / 2.0), 1.0);
        assert!(blur_weight(BLUR_CYCLE).abs() < 1e-6);
        for opacity in [0.0, 0.2, 0.7, 1.0] {
            for amount in [0.0, 0.3, 0.8, 1.0] {
                let (base, overlay) = blend_opacity(opacity, amount);
                assert!((overlay + base * (1.0 - overlay) - opacity).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn loads_a_bounded_gif_with_its_original_frame_timing() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("test.gif");
        let mut writer = GifEncoder::new(File::create(&path).unwrap());
        for (millis, color) in [(50, [240, 0, 0, 255]), (140, [0, 0, 220, 255])] {
            writer
                .encode_frame(Frame::from_parts(
                    RgbaImage::from_pixel(12, 8, Rgba(color)),
                    0,
                    0,
                    Delay::from_numer_denom_ms(millis, 1),
                ))
                .unwrap();
        }
        drop(writer);
        let context = egui::Context::default();
        let clip = load(
            &path,
            Effect {
                blur: 3,
                animated: true,
            },
            &context,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(clip.frames.len(), 2);
        assert_eq!(clip.frames[0].base.size(), [12, 8]);
        assert_eq!(clip.frames[0].blurred.as_ref().unwrap().size(), [6, 4]);
        assert!((clip.ends[0] - 0.05).abs() < 1e-8);
        assert!((clip.seconds - 0.19).abs() < 1e-8);
        assert!(load(&path, Effect::default(), &context, &AtomicBool::new(true)).is_err());
        assert!(
            load(
                &temp.path().join("missing.gif"),
                Effect::default(),
                &context,
                &AtomicBool::new(false)
            )
            .is_err()
        );
    }

    #[test]
    fn blur_off_keeps_pixels_and_blur_on_smooths_transparent_edges() {
        let mut settings: Settings = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert_eq!(settings.blur, 0);
        settings.blur = 4;
        let saved: Settings =
            serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
        assert_eq!(saved.blur, 4);

        let mut image = RgbaImage::from_pixel(31, 31, Rgba([255, 0, 0, 0]));
        for pixel in image.rows_mut() {
            for value in pixel.take(15) {
                *value = Rgba([0, 0, 255, 255]);
            }
        }
        assert_eq!(blur_image(image.clone(), 0), image);
        let blurred = blur_image(image, saved.blur);
        assert_eq!(blurred.dimensions(), (31, 31));
        let [red, _, blue, alpha] = blurred.get_pixel(15, 15).0;
        assert_eq!(red, 0);
        assert!(blue > 200 && (0..255).contains(&alpha));
        assert!(alpha > 0);
    }

    #[test]
    fn liquid_glass_stays_continuous_and_inside_the_crop() {
        let mut settings: Settings = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert!(!settings.liquid);
        assert_eq!(settings.liquid_strength, 0.6);
        assert_eq!(settings.liquid_speed, 1.0);
        settings.liquid = true;
        settings.liquid_strength = 0.8;
        let saved: Settings =
            serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
        assert!(saved.liquid);
        assert_eq!(saved.liquid_strength, 0.8);

        let crop = Rect::from_min_max(Pos2::new(0.2, 0.1), Pos2::new(0.8, 0.9));
        let still = displaced_uv(crop, 0.5, 0.5, 0.25, 0.0);
        assert!((still.x - 0.5).abs() < 1e-6 && (still.y - 0.5).abs() < 1e-6);
        let first = displaced_uv(crop, 0.3, 0.6, 0.1, 1.0);
        let later = displaced_uv(crop, 0.3, 0.6, 0.4, 1.0);
        assert_ne!(first, later);
        for point in [first, later] {
            assert!(point.x >= crop.min.x && point.x <= crop.max.x);
            assert!(point.y >= crop.min.y && point.y <= crop.max.y);
        }

        let before = displaced_uv(crop, 0.3, 0.6, 0.999, 1.0);
        let after = displaced_uv(crop, 0.3, 0.6, 1.001, 1.0);
        assert!((before.x - after.x).abs() < 0.01);
        assert!((before.y - after.y).abs() < 0.01);

        assert_eq!(glass_tint(0.04, 0.04, 0.2, 0.0).a(), 0);
        assert!(glass_tint(0.04, 0.04, 0.2, 1.0).a() > 0);
        assert_ne!(
            glass_tint(0.04, 0.04, 0.2, 1.0),
            glass_tint(0.04, 0.04, 0.5, 1.0)
        );
    }

    #[test]
    fn profile_copy_uses_its_own_path_and_leaves_the_source_intact() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("image.png");
        let library = temp.path().join("profile/library.json");
        fs::write(&source, b"image bytes").unwrap();
        let copied = copy_to_profile(&source, &library).unwrap();
        assert_ne!(copied, source);
        assert_eq!(fs::read(&copied).unwrap(), fs::read(&source).unwrap());
        assert_eq!(
            copy_to_profile(&copied, &library).unwrap(),
            copied.canonicalize().unwrap()
        );
    }
}
