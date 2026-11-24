use eframe::egui;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{
    cell::{Cell, RefCell},
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{BOOL, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
        Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
            DeleteObject, HDC,
        },
        System::{
            Com::{
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoUninitialize,
            },
            LibraryLoader::GetModuleHandleW,
            SystemServices::{
                APPCOMMAND_MEDIA_NEXTTRACK, APPCOMMAND_MEDIA_PAUSE, APPCOMMAND_MEDIA_PLAY,
                APPCOMMAND_MEDIA_PLAY_PAUSE, APPCOMMAND_MEDIA_PREVIOUSTRACK,
            },
        },
        UI::{
            HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
            Shell::{
                DefSubclassProc, ITaskbarList3, RemoveWindowSubclass, SetWindowSubclass, THB_FLAGS,
                THB_ICON, THB_TOOLTIP, THBF_DISABLED, THBF_ENABLED, THBN_CLICKED, THUMBBUTTON,
                TaskbarList,
            },
            WindowsAndMessaging::{
                CreateIconIndirect, DestroyIcon, FAPPCOMMAND_MASK, HICON, ICON_BIG, ICON_SMALL,
                ICONINFO, IMAGE_FLAGS, IMAGE_ICON, LoadImageW, RegisterWindowMessageW, SM_CXICON,
                SM_CXSMICON, SM_CYICON, SM_CYSMICON, SendMessageW, WM_APPCOMMAND, WM_COMMAND,
                WM_DPICHANGED, WM_NCDESTROY, WM_SETICON,
            },
        },
    },
    core::{PCWSTR, w},
};

const SUBCLASS: usize = 1;
const FIRST_BUTTON: u32 = 4500;

#[derive(Clone, Copy)]
pub enum Command {
    Previous,
    Play,
    Pause,
    PlayPause,
    Next,
    Shuffle,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Buttons {
    pub previous: bool,
    pub can_play: bool,
    pub playing: bool,
    pub next: bool,
    pub shuffle: bool,
}

struct Signals {
    taskbar_created: u32,
    reset: Cell<bool>,
    dpi_changed: Cell<bool>,
    media_key_fallback: Cell<bool>,
    alive: Cell<bool>,
    commands: RefCell<Vec<Command>>,
    ctx: egui::Context,
}

pub struct Taskbar {
    hwnd: HWND,
    signals: Box<Signals>,
    icons: Vec<Icon>,
    window_icons: Option<[Icon; 2]>,
    list: Option<ITaskbarList3>,
    state: Option<Buttons>,
    retry: Instant,
    retry_delay: Duration,
    com_initialized: bool,
}

impl Taskbar {
    pub fn new(window: &impl HasWindowHandle, ctx: &egui::Context) -> Option<Self> {
        let RawWindowHandle::Win32(handle) = window.window_handle().ok()?.as_raw() else {
            return None;
        };
        let hwnd = HWND(handle.hwnd.get());
        let icons = load_icons(hwnd)?;
        let taskbar_created = unsafe { RegisterWindowMessageW(w!("TaskbarButtonCreated")) };
        let signals = Box::new(Signals {
            taskbar_created,
            reset: Cell::new(false),
            dpi_changed: Cell::new(false),
            media_key_fallback: Cell::new(false),
            alive: Cell::new(true),
            commands: RefCell::new(Vec::new()),
            ctx: ctx.clone(),
        });
        // The callback runs on this window's GUI thread. The Box stays at a stable
        // address until the subclass is removed or the window is destroyed.
        let installed = unsafe {
            SetWindowSubclass(
                hwnd,
                Some(subclass),
                SUBCLASS,
                &*signals as *const Signals as usize,
            )
        };
        if !installed.as_bool() {
            return None;
        }
        let com_initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
        Some(Self {
            hwnd,
            signals,
            icons,
            window_icons: set_window_icons(hwnd),
            list: None,
            state: None,
            retry: Instant::now(),
            retry_delay: Duration::from_secs(1),
            com_initialized,
        })
    }

    pub fn commands(&self) -> Vec<Command> {
        std::mem::take(&mut *self.signals.commands.borrow_mut())
    }

    pub fn use_focused_media_keys(&self, enabled: bool) {
        self.signals.media_key_fallback.set(enabled);
    }

    pub fn refresh(&mut self, state: Buttons) {
        if !self.signals.alive.get() {
            return;
        }
        if self.signals.reset.replace(false) {
            self.list = None;
            self.state = None;
            self.retry = Instant::now();
            self.retry_delay = Duration::from_secs(1);
        }
        if self.signals.dpi_changed.replace(false) {
            if let Some(icons) = set_window_icons(self.hwnd) {
                self.window_icons = Some(icons);
            }
            if let Some(icons) = load_icons(self.hwnd) {
                self.icons = icons;
                self.state = None;
            }
        }
        if self.state == Some(state) || Instant::now() < self.retry {
            return;
        }
        let buttons = [
            make_button(0, self.icons[0].0, "Previous track", state.previous),
            make_button(
                1,
                self.icons[if state.playing { 2 } else { 1 }].0,
                if state.playing { "Pause" } else { "Play" },
                state.can_play,
            ),
            make_button(2, self.icons[3].0, "Next track", state.next),
            make_button(
                3,
                self.icons[if state.shuffle { 5 } else { 4 }].0,
                if state.shuffle {
                    "Turn shuffle off"
                } else {
                    "Turn shuffle on"
                },
                true,
            ),
        ];
        let result = if let Some(list) = &self.list {
            unsafe { list.ThumbBarUpdateButtons(self.hwnd, &buttons) }
        } else {
            // Explorer can create its taskbar button before eframe supplies the HWND.
            // In that case a successful initial registration also establishes readiness.
            unsafe {
                CoCreateInstance::<_, ITaskbarList3>(&TaskbarList, None, CLSCTX_INPROC_SERVER)
                    .and_then(|list| {
                        list.HrInit()?;
                        // Explorer only accepts one Add per window. A late readiness
                        // message can arrive after a successful registration.
                        if list.ThumbBarAddButtons(self.hwnd, &buttons).is_err() {
                            list.ThumbBarUpdateButtons(self.hwnd, &buttons)?;
                        }
                        Ok(list)
                    })
            }
            .map(|list| {
                self.list = Some(list);
            })
        };
        if result.is_ok() {
            self.state = Some(state);
            self.retry_delay = Duration::from_secs(1);
        } else {
            self.list = None;
            self.state = None;
            let delay = self.retry_delay;
            self.retry = Instant::now() + delay;
            self.retry_delay = (delay * 2).min(Duration::from_secs(30));
            self.signals
                .ctx
                .request_repaint_after_for(delay, egui::ViewportId::ROOT);
        }
    }
}

impl Drop for Taskbar {
    fn drop(&mut self) {
        if self.signals.alive.get() {
            unsafe {
                let _ = RemoveWindowSubclass(self.hwnd, Some(subclass), SUBCLASS);
                if self.window_icons.is_some() {
                    for kind in [ICON_SMALL, ICON_BIG] {
                        SendMessageW(self.hwnd, WM_SETICON, WPARAM(kind as usize), LPARAM(0));
                    }
                }
            }
        }
        drop(self.list.take());
        if self.com_initialized {
            unsafe {
                CoUninitialize();
            }
        }
    }
}

unsafe extern "system" fn subclass(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    reference: usize,
) -> LRESULT {
    let signals = unsafe { &*(reference as *const Signals) };
    if message == WM_NCDESTROY {
        signals.alive.set(false);
        unsafe {
            let _ = RemoveWindowSubclass(hwnd, Some(subclass), id);
        }
    } else if message == signals.taskbar_created && message != 0 {
        signals.reset.set(true);
        signals.ctx.request_repaint_of(egui::ViewportId::ROOT);
    } else if message == WM_DPICHANGED {
        signals.dpi_changed.set(true);
        signals.ctx.request_repaint();
    } else if message == WM_APPCOMMAND
        && signals.media_key_fallback.get()
        && let Some(command) = app_command(lparam)
    {
        enqueue(signals, command);
        return LRESULT(1);
    } else if message == WM_COMMAND && ((wparam.0 >> 16) & 0xffff) as u32 == THBN_CLICKED {
        let button = (wparam.0 as u32 & 0xffff).checked_sub(FIRST_BUTTON);
        if let Some(command) = match button {
            Some(0) => Some(Command::Previous),
            Some(1) => Some(Command::PlayPause),
            Some(2) => Some(Command::Next),
            Some(3) => Some(Command::Shuffle),
            _ => None,
        } {
            enqueue(signals, command);
            return LRESULT(0);
        }
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

fn enqueue(signals: &Signals, command: Command) {
    if let Ok(mut queue) = signals.commands.try_borrow_mut() {
        if queue.len() < 16 {
            queue.push(command);
        }
    }
    signals.ctx.request_repaint_of(egui::ViewportId::ROOT);
}

fn app_command(lparam: LPARAM) -> Option<Command> {
    let id = ((lparam.0 as usize >> 16) as u32 & 0xffff) & !FAPPCOMMAND_MASK;
    match id {
        id if id == APPCOMMAND_MEDIA_PREVIOUSTRACK.0 => Some(Command::Previous),
        id if id == APPCOMMAND_MEDIA_NEXTTRACK.0 => Some(Command::Next),
        id if id == APPCOMMAND_MEDIA_PLAY_PAUSE.0 => Some(Command::PlayPause),
        id if id == APPCOMMAND_MEDIA_PLAY.0 => Some(Command::Play),
        id if id == APPCOMMAND_MEDIA_PAUSE.0 => Some(Command::Pause),
        _ => None,
    }
}

fn make_button(index: u32, icon: HICON, tip: &str, enabled: bool) -> THUMBBUTTON {
    let mut label = [0; 260];
    for (out, input) in label.iter_mut().zip(tip.encode_utf16()) {
        *out = input;
    }
    THUMBBUTTON {
        dwMask: THB_ICON | THB_TOOLTIP | THB_FLAGS,
        iId: FIRST_BUTTON + index,
        iBitmap: 0,
        hIcon: icon,
        szTip: label,
        dwFlags: if enabled { THBF_ENABLED } else { THBF_DISABLED },
    }
}

#[derive(Clone, Copy)]
enum Glyph {
    Previous,
    Play,
    Pause,
    Next,
    Shuffle,
    ShuffleOn,
}

fn load_icons(hwnd: HWND) -> Option<Vec<Icon>> {
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    let width = unsafe { GetSystemMetricsForDpi(SM_CXICON, dpi) }.clamp(16, 256);
    let height = unsafe { GetSystemMetricsForDpi(SM_CYICON, dpi) }.clamp(16, 256);
    [
        Glyph::Previous,
        Glyph::Play,
        Glyph::Pause,
        Glyph::Next,
        Glyph::Shuffle,
        Glyph::ShuffleOn,
    ]
    .into_iter()
    .map(|glyph| Icon::new(glyph, width, height))
    .collect()
}

struct Icon(HICON);

fn set_window_icons(hwnd: HWND) -> Option<[Icon; 2]> {
    // Resource group 1 is written by build.rs. Load the nearest size for this monitor's DPI.
    unsafe {
        let module = GetModuleHandleW(PCWSTR::null()).ok()?;
        let dpi = GetDpiForWindow(hwnd).max(96);
        let load = |cx, cy| {
            LoadImageW(
                HINSTANCE(module.0),
                PCWSTR(std::ptr::without_provenance(1)),
                IMAGE_ICON,
                GetSystemMetricsForDpi(cx, dpi),
                GetSystemMetricsForDpi(cy, dpi),
                IMAGE_FLAGS(0),
            )
            .ok()
            .map(|handle| Icon(HICON(handle.0)))
        };
        let small = load(SM_CXSMICON, SM_CYSMICON)?;
        let large = load(SM_CXICON, SM_CYICON)?;
        SendMessageW(
            hwnd,
            WM_SETICON,
            WPARAM(ICON_SMALL as usize),
            LPARAM(small.0.0),
        );
        SendMessageW(
            hwnd,
            WM_SETICON,
            WPARAM(ICON_BIG as usize),
            LPARAM(large.0.0),
        );
        Some([small, large])
    }
}

impl Icon {
    fn new(glyph: Glyph, width: i32, height: i32) -> Option<Self> {
        let pixels = glyph_pixels(glyph, width, height);
        let bitmap = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut data = std::ptr::null_mut();
        let mask_bits = vec![0_u16; (width as usize).div_ceil(16) * height as usize];
        // The DIB section has width × height writable pixels. CreateIconIndirect copies both bitmaps.
        unsafe {
            let color = CreateDIBSection(
                HDC::default(),
                &bitmap,
                DIB_RGB_COLORS,
                &mut data,
                HANDLE::default(),
                0,
            )
            .ok()?;
            let mask = CreateBitmap(width, height, 1, 1, Some(mask_bits.as_ptr().cast()));
            data.cast::<u32>()
                .copy_from_nonoverlapping(pixels.as_ptr(), pixels.len());
            let icon = CreateIconIndirect(&ICONINFO {
                fIcon: BOOL(1),
                xHotspot: 0,
                yHotspot: 0,
                hbmMask: mask,
                hbmColor: color,
            });
            let _ = DeleteObject(mask);
            let _ = DeleteObject(color);
            icon.ok().map(Self)
        }
    }
}

fn glyph_pixels(glyph: Glyph, width: i32, height: i32) -> Vec<u32> {
    // Render in a 16-unit grid and supersample at the requested Windows icon size.
    let mut coverage = vec![0_u8; (width * height) as usize];
    for y in 0..height {
        for x in 0..width {
            let mut samples = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let xx = (x as f32 + (sx as f32 + 0.5) * 0.25) * 16.0 / width as f32;
                    let yy = (y as f32 + (sy as f32 + 0.5) * 0.25) * 16.0 / height as f32;
                    samples += u8::from(glyph_contains(glyph, xx, yy));
                }
            }
            coverage[(y * width + x) as usize] = samples;
        }
    }
    let color = match glyph {
        Glyph::ShuffleOn => [103_u32, 208, 248],
        _ => [243, 246, 251],
    };
    coverage
        .iter()
        .enumerate()
        .map(|(index, &samples)| {
            let x = (index % width as usize) as i32;
            let y = (index / width as usize) as i32;
            let edges = [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)];
            let edge = edges
                .into_iter()
                .filter(|(x, y)| (0..width).contains(x) && (0..height).contains(y))
                .map(|(x, y)| coverage[(y * width + x) as usize])
                .max()
                .unwrap_or(0);
            let foreground = u32::from(samples);
            let shadow = u32::from(edge.saturating_sub(samples)) / 2;
            let channel = |i: usize| (color[i] * foreground + 28 * shadow) / 16;
            ((foreground + shadow) * 255 / 16) << 24
                | channel(0) << 16
                | channel(1) << 8
                | channel(2)
        })
        .collect()
}

fn glyph_contains(glyph: Glyph, x: f32, y: f32) -> bool {
    let triangle = |x: f32, start: f32, end: f32| {
        (start..=end).contains(&x) && (y - 8.0).abs() <= (end - x) / (end - start) * 4.0
    };
    let segment = |ax: f32, ay: f32, bx: f32, by: f32, radius: f32| {
        let dx = bx - ax;
        let dy = by - ay;
        let t = (((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        (x - ax - t * dx).powi(2) + (y - ay - t * dy).powi(2) <= radius * radius
    };
    match glyph {
        Glyph::Previous => triangle(16.0 - x, 4.0, 10.5) || segment(3.5, 4.5, 3.5, 11.5, 0.85),
        Glyph::Play => triangle(x, 5.0, 12.5),
        Glyph::Pause => segment(5.4, 4.8, 5.4, 11.2, 1.0) || segment(10.6, 4.8, 10.6, 11.2, 1.0),
        Glyph::Next => triangle(x, 4.0, 10.5) || segment(12.5, 4.5, 12.5, 11.5, 0.85),
        Glyph::Shuffle | Glyph::ShuffleOn => {
            let r = 0.72;
            segment(2.4, 4.5, 4.5, 4.5, r)
                || segment(2.4, 11.5, 4.5, 11.5, r)
                || segment(4.5, 4.5, 10.2, 11.5, r)
                || segment(4.5, 11.5, 10.2, 4.5, r)
                || segment(10.2, 4.5, 13.5, 4.5, r)
                || segment(10.2, 11.5, 13.5, 11.5, r)
                || segment(11.5, 2.4, 13.8, 4.5, r)
                || segment(11.5, 6.6, 13.8, 4.5, r)
                || segment(11.5, 9.4, 13.8, 11.5, r)
                || segment(11.5, 13.6, 13.8, 11.5, r)
        }
    }
}

impl Drop for Icon {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyIcon(self.0);
        }
    }
}
