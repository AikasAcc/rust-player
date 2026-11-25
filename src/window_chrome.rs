use eframe::egui::{
    self, Color32, Context, PointerButton, Rect, Sense, Stroke, StrokeKind, Ui, ViewportCommand,
    pos2, vec2,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::cell::{Cell, RefCell};
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::ScreenToClient,
    UI::{
        HiDpi::GetDpiForWindow,
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            GetClientRect, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTCLIENT, HTLEFT,
            HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, IsZoomed, WM_NCDESTROY, WM_NCHITTEST,
        },
    },
};

const SUBCLASS: usize = 2;

struct HitRegions {
    captions: RefCell<Vec<Rect>>,
    alive: Cell<bool>,
}

pub(super) struct NativeChrome {
    hwnd: HWND,
    regions: Box<HitRegions>,
}

impl NativeChrome {
    pub fn new(window: &impl HasWindowHandle) -> Option<Self> {
        let RawWindowHandle::Win32(handle) = window.window_handle().ok()?.as_raw() else {
            return None;
        };
        let hwnd = HWND(handle.hwnd.get());
        let regions = Box::new(HitRegions {
            captions: RefCell::new(Vec::new()),
            alive: Cell::new(true),
        });
        // Windows invokes the subclass on the UI thread. Its data stays boxed until removal.
        if !unsafe {
            SetWindowSubclass(
                hwnd,
                Some(hit_test),
                SUBCLASS,
                &*regions as *const HitRegions as usize,
            )
        }
        .as_bool()
        {
            return None;
        }
        Some(Self { hwnd, regions })
    }

    pub fn set_captions(&self, rects: impl IntoIterator<Item = Rect>, pixels_per_point: f32) {
        let mut captions = self.regions.captions.borrow_mut();
        captions.clear();
        captions.extend(rects.into_iter().filter_map(|rect| {
            let scaled = Rect::from_min_max(
                (rect.min.to_vec2() * pixels_per_point).to_pos2(),
                (rect.max.to_vec2() * pixels_per_point).to_pos2(),
            );
            (scaled.width() > 0.0 && scaled.height() > 0.0).then_some(scaled)
        }));
    }
}

impl Drop for NativeChrome {
    fn drop(&mut self) {
        if self.regions.alive.get() {
            unsafe {
                let _ = RemoveWindowSubclass(self.hwnd, Some(hit_test), SUBCLASS);
            }
        }
    }
}

unsafe extern "system" fn hit_test(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    data: usize,
) -> LRESULT {
    if message == WM_NCDESTROY {
        unsafe {
            (*(data as *const HitRegions)).alive.set(false);
            let _ = RemoveWindowSubclass(hwnd, Some(hit_test), SUBCLASS);
        }
    }
    let original = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
    if message != WM_NCHITTEST || original.0 != HTCLIENT as isize {
        return original;
    }
    let mut point = POINT {
        x: i32::from(lparam.0 as i16),
        y: i32::from((lparam.0 >> 16) as i16),
    };
    if !unsafe { ScreenToClient(hwnd, &mut point) }.as_bool() {
        return original;
    }
    if !unsafe { IsZoomed(hwnd) }.as_bool() {
        let mut client = RECT::default();
        if unsafe { GetClientRect(hwnd, &mut client) }.is_ok() {
            let border = (unsafe { GetDpiForWindow(hwnd) }.max(96) * 6 / 96) as i32;
            let x = if point.x < border {
                -1
            } else if point.x >= client.right - border {
                1
            } else {
                0
            };
            let y = if point.y < border {
                -1
            } else if point.y >= client.bottom - border {
                1
            } else {
                0
            };
            let edge = match (x, y) {
                (-1, -1) => HTTOPLEFT,
                (0, -1) => HTTOP,
                (1, -1) => HTTOPRIGHT,
                (-1, 0) => HTLEFT,
                (1, 0) => HTRIGHT,
                (-1, 1) => HTBOTTOMLEFT,
                (0, 1) => HTBOTTOM,
                (1, 1) => HTBOTTOMRIGHT,
                _ => HTCLIENT,
            };
            if edge != HTCLIENT {
                return LRESULT(edge as isize);
            }
        }
    }
    let regions = unsafe { &*(data as *const HitRegions) };
    if regions
        .captions
        .borrow()
        .iter()
        .any(|rect| rect.contains(pos2(point.x as f32, point.y as f32)))
    {
        LRESULT(HTCAPTION as isize)
    } else {
        original
    }
}

fn maximized(ctx: &Context) -> bool {
    ctx.input(|input| input.viewport().maximized.unwrap_or(false))
}

pub(super) fn title_drag(ui: &Ui, rect: Rect) {
    let title = ui.interact(rect, ui.id().with("window drag"), Sense::click_and_drag());
    if title.double_clicked_by(PointerButton::Primary) {
        ui.ctx()
            .send_viewport_cmd(ViewportCommand::Maximized(!maximized(ui.ctx())));
    } else if title.drag_started_by(PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
}

pub(super) fn buttons(ui: &mut Ui) {
    let maximized = maximized(ui.ctx());
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 1.0;
        // The header lays out actions right-to-left.
        for (index, label) in [
            "Close window",
            if maximized {
                "Restore window"
            } else {
                "Maximize window"
            },
            "Minimize window",
        ]
        .into_iter()
        .enumerate()
        {
            let response = ui
                .add(
                    egui::Button::new("")
                        .frame(false)
                        .min_size(vec2(29.0, 28.0)),
                )
                .on_hover_text(label);
            response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
            let painter = ui.painter();
            let mut color = ui.visuals().weak_text_color();
            if response.hovered() || response.has_focus() {
                let fill = if index == 0 {
                    color = Color32::WHITE;
                    Color32::from_rgb(168, 42, 53)
                } else {
                    color = ui.visuals().text_color();
                    ui.visuals().widgets.hovered.weak_bg_fill
                };
                painter.rect_filled(response.rect, 4, fill);
            }
            let center = response.rect.center();
            let stroke = Stroke::new(1.0_f32, color);
            match index {
                0 => {
                    painter
                        .line_segment([center + vec2(-4.0, -4.0), center + vec2(4.0, 4.0)], stroke);
                    painter
                        .line_segment([center + vec2(-4.0, 4.0), center + vec2(4.0, -4.0)], stroke);
                }
                1 if maximized => {
                    let front = Rect::from_center_size(center + vec2(-1.0, 1.0), vec2(7.0, 7.0));
                    let back = front.translate(vec2(2.0, -2.0));
                    painter.hline(back.x_range(), back.top(), stroke);
                    painter.vline(back.right(), back.y_range(), stroke);
                    painter.rect_stroke(front, 0, stroke, StrokeKind::Inside);
                }
                1 => {
                    painter.rect_stroke(
                        Rect::from_center_size(center, vec2(9.0, 9.0)),
                        0,
                        stroke,
                        StrokeKind::Inside,
                    );
                }
                _ => {
                    painter.hline(center.x - 4.5..=center.x + 4.5, center.y + 3.5, stroke);
                }
            }
            if response.clicked() {
                ui.ctx().send_viewport_cmd(match index {
                    0 => ViewportCommand::Close,
                    1 => ViewportCommand::Maximized(!maximized),
                    _ => ViewportCommand::Minimized(true),
                });
            }
        }
    });
}
