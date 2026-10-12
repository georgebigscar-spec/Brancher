//! Floating UI: Select, DropdownMenu, Popover, Tooltip, Dialog, Toaster.

use egui::{
    Align2, Color32, Context, CursorIcon, FontId, Frame, Id, Margin, Modal, Popup,
    PopupCloseBehavior, Pos2, Rect, Response, RichText, Sense, Stroke, StrokeKind, Ui, Vec2,
    Widget, WidgetInfo, WidgetType,
};

use crate::basic::{paint_check, paint_chevron};
use crate::button::Button;
use crate::theme::{paint_focus_ring, Theme};

// ---------------------------------------------------------------------------

/// `<Select>`: a trigger that looks like an input and opens a list of options.
#[must_use = "add it with `ui.add(...)`"]
pub struct Select<'a> {
    selected: &'a mut Option<usize>,
    options: &'a [&'a str],
    placeholder: String,
    width: f32,
}

impl<'a> Select<'a> {
    pub fn new(selected: &'a mut Option<usize>, options: &'a [&'a str]) -> Self {
        Self { selected, options, placeholder: "Select…".into(), width: 180.0 }
    }
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }
}

impl Widget for Select<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let t = Theme::get(ui.ctx());
        let (rect, mut response) =
            ui.allocate_exact_size(Vec2::new(self.width, 36.0), Sense::click());
        let current = self.selected.and_then(|i| self.options.get(i).copied());
        response.widget_info(|| {
            WidgetInfo::labeled(WidgetType::ComboBox, ui.is_enabled(), current.unwrap_or(&self.placeholder))
        });
        response = response.on_hover_cursor(CursorIcon::PointingHand);

        let p = ui.painter();
        let fill = if t.dark { t.input.gamma_multiply(0.3) } else { Color32::TRANSPARENT };
        p.add(t.shadow_sm().as_shape(rect, t.radius_md()));
        p.rect(rect, t.radius_md(), fill.to_opaque_or(t.background), Stroke::new(1.0, t.input), StrokeKind::Inside);
        let (text, color) = match current {
            Some(s) => (s.to_owned(), t.foreground),
            None => (self.placeholder.clone(), t.muted_foreground),
        };
        let g = p.layout_no_wrap(text, FontId::proportional(14.0), color);
        p.galley(Pos2::new(rect.left() + 12.0, rect.center().y - g.size().y / 2.0), g, color);
        paint_chevron(p, Pos2::new(rect.right() - 16.0, rect.center().y), 0.0, t.muted_foreground);

        let mut changed = false;
        let popup = Popup::menu(&response)
            .width(self.width)
            .gap(4.0)
            .frame(t.popover_frame())
            .show(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (i, option) in self.options.iter().enumerate() {
                    let is_sel = *self.selected == Some(i);
                    if menu_row(ui, option, None, is_sel).clicked() {
                        *self.selected = Some(i);
                        changed = true;
                        ui.close();
                    }
                }
            });
        if response.has_focus() || popup.is_some() {
            paint_focus_ring(ui.painter(), rect, t.radius_md(), &t);
        }
        if changed {
            response.mark_changed();
        }
        response
    }
}

trait OpaqueOr {
    fn to_opaque_or(self, fallback: Color32) -> Color32;
}
impl OpaqueOr for Color32 {
    fn to_opaque_or(self, fallback: Color32) -> Color32 {
        if self.a() == 0 { fallback } else { self }
    }
}

/// One row inside a menu or select list: 32px, accent on hover, optional
/// shortcut on the right, check mark when `checked`.
fn menu_row(ui: &mut Ui, text: &str, shortcut: Option<&str>, checked: bool) -> Response {
    let t = Theme::get(ui.ctx());
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 32.0), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), text));
    let p = ui.painter();
    if response.hovered() || response.has_focus() {
        p.rect_filled(rect, t.radius_sm(), t.accent);
    }
    let fg = if response.hovered() { t.accent_foreground } else { t.popover_foreground };
    let g = p.layout_no_wrap(text.to_owned(), FontId::proportional(14.0), fg);
    p.galley(Pos2::new(rect.left() + 8.0, rect.center().y - g.size().y / 2.0), g, fg);
    if let Some(s) = shortcut {
        p.text(
            Pos2::new(rect.right() - 8.0, rect.center().y),
            Align2::RIGHT_CENTER,
            s,
            FontId::proportional(12.0),
            t.muted_foreground,
        );
    }
    if checked {
        let c = Rect::from_center_size(Pos2::new(rect.right() - 16.0, rect.center().y), Vec2::splat(14.0));
        paint_check(p, c, fg, 1.5);
    }
    response
}

// ---------------------------------------------------------------------------

/// `<DropdownMenu>` attached to a trigger. Build the contents with
/// [`menu_item`], [`menu_label`] and [`menu_separator`].
pub fn dropdown_menu<R>(
    trigger: &Response,
    width: f32,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let t = Theme::get(&trigger.ctx);
    Popup::menu(trigger)
        .width(width)
        .gap(4.0)
        .frame(t.popover_frame())
        .show(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            add_contents(ui)
        })
        .map(|r| r.inner)
}

/// A clickable menu entry; closes the menu when clicked.
pub fn menu_item(ui: &mut Ui, text: &str, shortcut: Option<&str>) -> Response {
    let r = menu_row(ui, text, shortcut, false);
    if r.clicked() {
        ui.close();
    }
    r
}

/// A checkable menu entry (`DropdownMenuCheckboxItem`).
pub fn menu_checkbox(ui: &mut Ui, text: &str, checked: &mut bool) -> Response {
    let mut r = menu_row(ui, text, None, *checked);
    if r.clicked() {
        *checked = !*checked;
        r.mark_changed();
    }
    r
}

/// A non-interactive heading inside a menu.
pub fn menu_label(ui: &mut Ui, text: &str) {
    let t = Theme::get(ui.ctx());
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.label(RichText::new(text).size(14.0).strong().color(t.popover_foreground));
    });
    ui.add_space(6.0);
}

pub fn menu_separator(ui: &mut Ui) {
    let t = Theme::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 9.0), Sense::hover());
    ui.painter().hline(
        (rect.left() - 4.0)..=(rect.right() + 4.0),
        rect.center().y,
        Stroke::new(1.0, t.border),
    );
}

// ---------------------------------------------------------------------------

/// `<Popover>`: free-form content that opens when `trigger` is clicked and
/// closes when clicking outside.
pub fn popover<R>(
    trigger: &Response,
    width: f32,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let t = Theme::get(&trigger.ctx);
    Popup::from_toggle_button_response(trigger)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .width(width)
        .gap(4.0)
        .frame(t.popover_frame().inner_margin(Margin::same(16)))
        .show(add_contents)
        .map(|r| r.inner)
}

// ---------------------------------------------------------------------------

/// `<Tooltip>`: `ui.add(button).tooltip("text")`.
pub trait TooltipExt {
    fn tooltip(self, text: impl Into<String>) -> Self;
}

impl TooltipExt for Response {
    fn tooltip(self, text: impl Into<String>) -> Self {
        let t = Theme::get(&self.ctx);
        let mut tip = egui::Tooltip::for_enabled(&self);
        tip.popup = tip.popup.gap(4.0).frame(
            Frame::new()
                .fill(t.primary)
                .corner_radius(t.radius_md())
                .inner_margin(Margin::symmetric(12, 6)),
        );
        let text = text.into();
        tip.show(|ui| {
            ui.label(RichText::new(text).size(12.0).color(t.primary_foreground));
        });
        self
    }
}

// ---------------------------------------------------------------------------

/// `<Dialog>`: a centered modal with title, description and a close button.
pub struct Dialog {
    id: Id,
    title: String,
    description: Option<String>,
    width: f32,
}

/// What happened in a [`Dialog`] this frame.
pub struct DialogResponse<R> {
    pub inner: R,
    /// Escape, a click on the backdrop, or the ✕ button.
    pub should_close: bool,
}

impl Dialog {
    pub fn new(id_salt: impl egui::AsId, title: impl Into<String>) -> Self {
        Self { id: Id::new(id_salt), title: title.into(), description: None, width: 425.0 }
    }
    pub fn description(mut self, text: impl Into<String>) -> Self {
        self.description = Some(text.into());
        self
    }
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    pub fn show<R>(self, ctx: &Context, add_contents: impl FnOnce(&mut Ui) -> R) -> DialogResponse<R> {
        let t = Theme::get(ctx);
        let mut close_clicked = false;
        let modal = Modal::new(self.id)
            .backdrop_color(Color32::from_black_alpha(128))
            .frame(
                Frame::new()
                    .fill(t.background)
                    .stroke(Stroke::new(1.0, t.border))
                    .corner_radius(t.radius_lg())
                    .inner_margin(Margin::same(24))
                    .shadow(t.shadow_lg()),
            )
            .show(ctx, |ui| {
                ui.set_width(self.width - 48.0);
                let top = ui.max_rect().right_top();
                let close = Rect::from_center_size(top + Vec2::new(-4.0, 4.0), Vec2::splat(20.0));
                let r = ui.interact(close, self.id.with("close"), Sense::click());
                let c = if r.hovered() { t.foreground } else { t.muted_foreground };
                let p = ui.painter();
                let s = 4.5;
                let m = close.center();
                p.line_segment([m + Vec2::new(-s, -s), m + Vec2::new(s, s)], Stroke::new(1.5, c));
                p.line_segment([m + Vec2::new(-s, s), m + Vec2::new(s, -s)], Stroke::new(1.5, c));
                close_clicked = r.clicked();

                ui.label(RichText::new(&self.title).size(18.0).strong().color(t.foreground));
                if let Some(d) = &self.description {
                    ui.add_space(-2.0);
                    ui.label(RichText::new(d).size(14.0).color(t.muted_foreground));
                }
                ui.add_space(8.0);
                add_contents(ui)
            });
        DialogResponse { should_close: modal.should_close() || close_clicked, inner: modal.inner }
    }
}

/// Right-aligned footer row for a dialog or card (`<DialogFooter>`).
pub fn footer<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    ui.add_space(8.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add_contents).inner
}

/// Convenience: an `<AlertDialog>` with Cancel / Continue. Returns
/// `Some(true)` on continue, `Some(false)` on cancel or dismiss.
pub fn alert_dialog(
    ctx: &Context,
    id_salt: impl egui::AsId,
    title: &str,
    description: &str,
    action: &str,
) -> Option<bool> {
    let resp = Dialog::new(id_salt, title).description(description).show(ctx, |ui| {
        footer(ui, |ui| {
            if ui.add(Button::new(action)).clicked() {
                return Some(true);
            }
            if ui.add(Button::outline("Cancel")).clicked() {
                return Some(false);
            }
            None
        })
    });
    resp.inner.or(resp.should_close.then_some(false))
}

// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Toast {
    id: u64,
    title: String,
    description: Option<String>,
    created: f64,
    destructive: bool,
}

#[derive(Clone, Default)]
struct ToastState {
    next_id: u64,
    toasts: Vec<Toast>,
}

/// Sonner-style toasts. Call [`toast`] anywhere, and [`Toaster::show`] once per frame.
pub struct Toaster;

fn toast_state_id() -> Id {
    Id::new("widgets::toasts")
}

/// Queues a toast with an optional description.
pub fn toast(ctx: &Context, title: impl Into<String>, description: Option<&str>) {
    push_toast(ctx, title.into(), description.map(str::to_owned), false);
}

/// Queues a red error toast.
pub fn toast_error(ctx: &Context, title: impl Into<String>, description: Option<&str>) {
    push_toast(ctx, title.into(), description.map(str::to_owned), true);
}

fn push_toast(ctx: &Context, title: String, description: Option<String>, destructive: bool) {
    let now = ctx.input(|i| i.time);
    ctx.data_mut(|d| {
        let s = d.get_temp_mut_or_default::<ToastState>(toast_state_id());
        s.next_id += 1;
        let id = s.next_id;
        s.toasts.push(Toast { id, title, description, created: now, destructive });
    });
}

impl Toaster {
    const LIFETIME: f64 = 4.0;

    pub fn show(ctx: &Context) {
        let t = Theme::get(ctx);
        let now = ctx.input(|i| i.time);
        let mut state = ctx.data(|d| d.get_temp::<ToastState>(toast_state_id())).unwrap_or_default();
        state.toasts.retain(|toast| now - toast.created < Self::LIFETIME);
        if state.toasts.is_empty() {
            ctx.data_mut(|d| d.insert_temp(toast_state_id(), state));
            return;
        }
        let mut dismissed = None;
        let screen = ctx.content_rect();
        let mut bottom = screen.bottom() - 16.0;
        for toast in state.toasts.iter().rev().take(3) {
            let age = (now - toast.created) as f32;
            let fade = (age / 0.15).min(1.0).min(((Self::LIFETIME as f32) - age) / 0.3).clamp(0.0, 1.0);
            let area = egui::Area::new(Id::new(("shadcn_toast", toast.id)))
                .order(egui::Order::Foreground)
                .pivot(Align2::RIGHT_BOTTOM)
                .fixed_pos(Pos2::new(screen.right() - 16.0, bottom + (1.0 - fade) * 12.0))
                .show(ctx, |ui| {
                    ui.multiply_opacity(fade);
                    Frame::new()
                        .fill(t.popover)
                        .stroke(Stroke::new(1.0, t.border))
                        .corner_radius(t.radius_lg())
                        .inner_margin(Margin::same(16))
                        .shadow(t.shadow_lg())
                        .show(ui, |ui| {
                            ui.set_width(324.0);
                            let title_color = if toast.destructive { t.destructive } else { t.popover_foreground };
                            ui.label(RichText::new(&toast.title).size(14.0).strong().color(title_color));
                            if let Some(d) = &toast.description {
                                ui.add_space(-4.0);
                                ui.label(RichText::new(d).size(13.0).color(t.muted_foreground));
                            }
                        });
                });
            if area.response.interact(Sense::click()).clicked() {
                dismissed = Some(toast.id);
            }
            bottom -= area.response.rect.height() + 12.0;
        }
        if let Some(id) = dismissed {
            state.toasts.retain(|x| x.id != id);
        }
        ctx.data_mut(|d| d.insert_temp(toast_state_id(), state));
        ctx.request_repaint();
    }
}
