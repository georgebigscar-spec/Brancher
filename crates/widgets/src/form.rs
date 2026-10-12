//! Form controls: Input, Textarea, Checkbox, Switch, RadioGroup, Slider, Toggle, ToggleGroup.

use std::ops::RangeInclusive;

use egui::{
    Color32, CornerRadius, CursorIcon, FontId, Frame, Key, Margin, Pos2, Rect, Response, RichText,
    Sense, Stroke, StrokeKind, TextEdit, Ui, Vec2, Widget, WidgetInfo, WidgetType,
};

use crate::basic::paint_check;
use crate::theme::{paint_focus_ring, Theme};

// ---------------------------------------------------------------------------

/// `<Input>`: a single-line text field.
#[must_use = "add it with `ui.add(...)`"]
pub struct Input<'t> {
    text: &'t mut String,
    placeholder: String,
    password: bool,
    width: Option<f32>,
    invalid: bool,
}

impl<'t> Input<'t> {
    pub fn new(text: &'t mut String) -> Self {
        Self { text, placeholder: String::new(), password: false, width: None, invalid: false }
    }
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }
    pub fn password(mut self) -> Self {
        self.password = true;
        self
    }
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }
    /// `aria-invalid`: red border and ring.
    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }
}

fn input_frame(t: &Theme, invalid: bool) -> Frame {
    Frame::new()
        .fill(if t.dark { t.input.gamma_multiply(0.3) } else { Color32::TRANSPARENT })
        .stroke(Stroke::new(1.0, if invalid { t.destructive } else { t.input }))
        .corner_radius(t.radius_md())
        .inner_margin(Margin::symmetric(12, 8))
}

fn paint_input_focus(ui: &Ui, rect: Rect, t: &Theme, invalid: bool) {
    let mut ring_theme = t.clone();
    if invalid {
        ring_theme.ring = t.destructive;
    }
    paint_focus_ring(ui.painter(), rect, t.radius_md(), &ring_theme);
}

impl Widget for Input<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let t = Theme::get(ui.ctx());
        let width = self.width.unwrap_or_else(|| ui.available_width());
        let response = ui.add(
            TextEdit::singleline(self.text)
                .frame(input_frame(&t, self.invalid))
                .font(FontId::proportional(14.0))
                .text_color(t.foreground)
                .hint_text(RichText::new(self.placeholder).color(t.muted_foreground))
                .password(self.password)
                .vertical_align(egui::Align::Center)
                .min_size(Vec2::new(width, 36.0))
                .desired_width(width - 24.0),
        );
        if response.has_focus() {
            paint_input_focus(ui, response.rect, &t, self.invalid);
        }
        response
    }
}

/// `<Textarea>`: a multi-line text field.
#[must_use = "add it with `ui.add(...)`"]
pub struct Textarea<'t> {
    text: &'t mut String,
    placeholder: String,
    rows: usize,
    width: Option<f32>,
}

impl<'t> Textarea<'t> {
    pub fn new(text: &'t mut String) -> Self {
        Self { text, placeholder: String::new(), rows: 3, width: None }
    }
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }
    pub fn rows(mut self, rows: usize) -> Self {
        self.rows = rows;
        self
    }
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }
}

impl Widget for Textarea<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let t = Theme::get(ui.ctx());
        let width = self.width.unwrap_or_else(|| ui.available_width());
        let response = ui.add(
            TextEdit::multiline(self.text)
                .frame(input_frame(&t, false))
                .font(FontId::proportional(14.0))
                .text_color(t.foreground)
                .hint_text(RichText::new(self.placeholder).color(t.muted_foreground))
                .desired_rows(self.rows)
                .min_size(Vec2::new(width, 64.0))
                .desired_width(width - 24.0),
        );
        if response.has_focus() {
            paint_input_focus(ui, response.rect, &t, false);
        }
        response
    }
}

// ---------------------------------------------------------------------------

/// Allocates `[control][gap][label]` as one clickable row.
fn control_with_label(
    ui: &mut Ui,
    control_size: Vec2,
    label: Option<&str>,
    widget_type: WidgetType,
) -> (Response, Rect, Option<(std::sync::Arc<egui::Galley>, Pos2)>) {
    let t = Theme::get(ui.ctx());
    let galley = label.map(|l| {
        ui.painter()
            .layout_no_wrap(l.to_owned(), FontId::proportional(14.0), t.foreground)
    });
    let gap = 8.0;
    let mut size = control_size;
    if let Some(g) = &galley {
        size.x += gap + g.size().x;
        size.y = size.y.max(g.size().y);
    }
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| WidgetInfo::labeled(widget_type, ui.is_enabled(), label.unwrap_or("")));
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    let control = Rect::from_min_size(
        Pos2::new(rect.left(), rect.center().y - control_size.y / 2.0),
        control_size,
    );
    let text = galley.map(|g| {
        let pos = Pos2::new(control.right() + gap, rect.center().y - g.size().y / 2.0);
        (g, pos)
    });
    (response, control, text)
}

/// `<Checkbox>` with an optional label.
#[must_use = "add it with `ui.add(...)`"]
pub struct Checkbox<'a> {
    checked: &'a mut bool,
    label: Option<String>,
}

impl<'a> Checkbox<'a> {
    pub fn new(checked: &'a mut bool) -> Self {
        Self { checked, label: None }
    }
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
}

impl Widget for Checkbox<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let t = Theme::get(ui.ctx());
        let (mut response, rect, text) =
            control_with_label(ui, Vec2::splat(16.0), self.label.as_deref(), WidgetType::Checkbox);
        if response.clicked() {
            *self.checked = !*self.checked;
            response.mark_changed();
        }
        let p = ui.painter();
        let radius = CornerRadius::same(4);
        if *self.checked {
            p.rect(rect, radius, t.primary, Stroke::new(1.0, t.primary), StrokeKind::Inside);
            paint_check(p, rect.shrink(2.0), t.primary_foreground, 1.6);
        } else {
            p.rect_stroke(rect, radius, Stroke::new(1.0, t.input), StrokeKind::Inside);
        }
        if response.has_focus() {
            paint_focus_ring(p, rect, radius, &t);
        }
        if let Some((g, pos)) = text {
            p.galley(pos, g, t.foreground);
        }
        response
    }
}

/// `<Switch>` with an optional label.
#[must_use = "add it with `ui.add(...)`"]
pub struct Switch<'a> {
    on: &'a mut bool,
    label: Option<String>,
}

impl<'a> Switch<'a> {
    pub fn new(on: &'a mut bool) -> Self {
        Self { on, label: None }
    }
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
}

impl Widget for Switch<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let t = Theme::get(ui.ctx());
        let (mut response, rect, text) = control_with_label(
            ui,
            Vec2::new(32.0, 18.4),
            self.label.as_deref(),
            WidgetType::Checkbox,
        );
        if response.clicked() {
            *self.on = !*self.on;
            response.mark_changed();
        }
        let k = ui.ctx().animate_bool_responsive(response.id, *self.on);
        let p = ui.painter();
        let off_track = if t.dark { t.input.gamma_multiply(0.8) } else { t.input };
        let track = crate::theme::mix(off_track, t.primary, k);
        let radius = CornerRadius::same((rect.height() / 2.0) as u8);
        p.rect_filled(rect, radius, track);
        let thumb_r = 8.0;
        let x = egui::lerp(
            (rect.left() + 1.0 + thumb_r)..=(rect.right() - 1.0 - thumb_r),
            k,
        );
        let thumb = if t.dark {
            crate::theme::mix(t.foreground, t.primary_foreground, k)
        } else {
            t.background
        };
        p.circle_filled(Pos2::new(x, rect.center().y), thumb_r, thumb);
        if response.has_focus() {
            paint_focus_ring(p, rect, radius, &t);
        }
        if let Some((g, pos)) = text {
            p.galley(pos, g, t.foreground);
        }
        response
    }
}

/// One option of a `<RadioGroup>`; selected when `*current == value`.
#[must_use = "add it with `ui.add(...)`"]
pub struct RadioItem<'a, T: PartialEq> {
    current: &'a mut T,
    value: T,
    label: String,
}

impl<'a, T: PartialEq> RadioItem<'a, T> {
    pub fn new(current: &'a mut T, value: T, label: impl Into<String>) -> Self {
        Self { current, value, label: label.into() }
    }
}

impl<T: PartialEq> Widget for RadioItem<'_, T> {
    fn ui(self, ui: &mut Ui) -> Response {
        let t = Theme::get(ui.ctx());
        let (mut response, rect, text) =
            control_with_label(ui, Vec2::splat(16.0), Some(&self.label), WidgetType::RadioButton);
        let selected = *self.current == self.value;
        if response.clicked() && !selected {
            *self.current = self.value;
            response.mark_changed();
        }
        let selected = response.changed() || selected;
        let p = ui.painter();
        p.circle_stroke(rect.center(), 7.5, Stroke::new(1.0, t.input));
        if selected {
            p.circle_filled(rect.center(), 4.0, t.primary);
        }
        if response.has_focus() {
            paint_focus_ring(p, rect, CornerRadius::same(8), &t);
        }
        if let Some((g, pos)) = text {
            p.galley(pos, g, t.foreground);
        }
        response
    }
}

/// A vertical `<RadioGroup>` over `(value, label)` pairs.
pub fn radio_group<T: PartialEq + Clone>(
    ui: &mut Ui,
    current: &mut T,
    options: &[(T, &str)],
) -> Response {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 12.0;
        let mut response: Option<Response> = None;
        for (value, label) in options {
            let r = ui.add(RadioItem::new(current, value.clone(), *label));
            response = Some(match response {
                Some(acc) => acc | r,
                None => r,
            });
        }
        response.expect("radio_group needs at least one option")
    })
    .inner
}

// ---------------------------------------------------------------------------

/// `<Slider>` over an `f32`.
#[must_use = "add it with `ui.add(...)`"]
pub struct Slider<'a> {
    value: &'a mut f32,
    range: RangeInclusive<f32>,
    step: Option<f32>,
    width: Option<f32>,
}

impl<'a> Slider<'a> {
    pub fn new(value: &'a mut f32, range: RangeInclusive<f32>) -> Self {
        Self { value, range, step: None, width: None }
    }
    pub fn step(mut self, step: f32) -> Self {
        self.step = Some(step);
        self
    }
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }
}

impl Widget for Slider<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let t = Theme::get(ui.ctx());
        let width = self.width.unwrap_or_else(|| ui.available_width());
        let (rect, mut response) =
            ui.allocate_exact_size(Vec2::new(width, 20.0), Sense::click_and_drag());
        response.widget_info(|| WidgetInfo::slider(ui.is_enabled(), *self.value as f64, ""));
        let thumb_r = 8.0;
        let track = Rect::from_center_size(rect.center(), Vec2::new(width - 2.0 * thumb_r, 6.0));
        let (min, max) = (*self.range.start(), *self.range.end());
        let snap = |v: f32| {
            let v = match self.step {
                Some(s) if s > 0.0 => min + ((v - min) / s).round() * s,
                _ => v,
            };
            v.clamp(min, max)
        };

        let old = *self.value;
        if let Some(pos) = response.interact_pointer_pos() {
            let k = ((pos.x - track.left()) / track.width()).clamp(0.0, 1.0);
            *self.value = snap(egui::lerp(min..=max, k));
        }
        if response.has_focus() {
            let step = self.step.unwrap_or((max - min) / 100.0);
            let delta = ui.input(|i| {
                i.num_presses(Key::ArrowRight) as f32 - i.num_presses(Key::ArrowLeft) as f32
            });
            if delta != 0.0 {
                *self.value = snap(*self.value + delta * step);
            }
        }
        if *self.value != old {
            response.mark_changed();
        }

        let p = ui.painter();
        let full = CornerRadius::same(3);
        p.rect_filled(track, full, t.muted);
        let k = if max > min { (*self.value - min) / (max - min) } else { 0.0 };
        let x = track.left() + track.width() * k;
        p.rect_filled(Rect::from_min_max(track.min, Pos2::new(x, track.max.y)), full, t.primary);
        let center = Pos2::new(x, rect.center().y);
        if response.hovered() || response.has_focus() || response.dragged() {
            p.circle_filled(center, thumb_r + 4.0, t.ring_soft());
        }
        p.add(t.shadow_sm().as_shape(
            Rect::from_center_size(center, Vec2::splat(2.0 * thumb_r)),
            CornerRadius::same(thumb_r as u8),
        ));
        p.circle(center, thumb_r, t.background, Stroke::new(1.0, t.primary));
        response.on_hover_cursor(CursorIcon::PointingHand)
    }
}

// ---------------------------------------------------------------------------

/// `<Toggle>`: a two-state button.
#[must_use = "add it with `ui.add(...)`"]
pub struct Toggle<'a> {
    on: &'a mut bool,
    text: String,
    outline: bool,
}

impl<'a> Toggle<'a> {
    pub fn new(on: &'a mut bool, text: impl Into<String>) -> Self {
        Self { on, text: text.into(), outline: false }
    }
    pub fn outline(mut self) -> Self {
        self.outline = true;
        self
    }
}

fn paint_toggle_face(ui: &Ui, rect: Rect, text: &str, on: bool, hovered: bool, outline: bool) {
    let t = Theme::get(ui.ctx());
    let p = ui.painter();
    let fill = if on {
        t.accent
    } else if hovered {
        t.muted
    } else {
        Color32::TRANSPARENT
    };
    let stroke = if outline { Stroke::new(1.0, t.input) } else { Stroke::NONE };
    p.rect(rect, t.radius_md(), fill, stroke, StrokeKind::Inside);
    let fg = if on { t.accent_foreground } else if hovered { t.muted_foreground } else { t.foreground };
    let g = p.layout_no_wrap(text.to_owned(), FontId::proportional(14.0), fg);
    p.galley(rect.center() - g.size() / 2.0, g, fg);
}

impl Widget for Toggle<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let g = ui.painter().layout_no_wrap(
            self.text.clone(),
            FontId::proportional(14.0),
            Color32::PLACEHOLDER,
        );
        let size = Vec2::new((g.size().x + 16.0).max(36.0), 36.0);
        let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
        response.widget_info(|| {
            WidgetInfo::selected(WidgetType::Button, ui.is_enabled(), *self.on, &self.text)
        });
        if response.clicked() {
            *self.on = !*self.on;
            response.mark_changed();
        }
        paint_toggle_face(ui, rect, &self.text, *self.on, response.hovered(), self.outline);
        if response.has_focus() {
            let t = Theme::get(ui.ctx());
            paint_focus_ring(ui.painter(), rect, t.radius_md(), &t);
        }
        response
    }
}

/// `<ToggleGroup type="single" variant="outline">`: joined buttons, one selected.
pub fn toggle_group(ui: &mut Ui, selected: &mut usize, items: &[&str]) -> Response {
    let t = Theme::get(ui.ctx());
    let font = FontId::proportional(14.0);
    let widths: Vec<f32> = items
        .iter()
        .map(|s| {
            let g = ui.painter().layout_no_wrap(s.to_string(), font.clone(), Color32::PLACEHOLDER);
            (g.size().x + 20.0).max(36.0)
        })
        .collect();
    let total = Vec2::new(widths.iter().sum(), 36.0);
    let (rect, mut response) = ui.allocate_exact_size(total, Sense::hover());
    let r = t.radius_md();
    let mut x = rect.left();
    for (i, (label, w)) in items.iter().zip(&widths).enumerate() {
        let item = Rect::from_min_size(Pos2::new(x, rect.top()), Vec2::new(*w, 36.0));
        x += w;
        let id = response.id.with(i);
        let item_resp = ui.interact(item, id, Sense::click());
        if item_resp.clicked() && *selected != i {
            *selected = i;
            response.mark_changed();
        }
        let on = *selected == i;
        let fill = if on {
            t.accent
        } else if item_resp.hovered() {
            t.muted
        } else {
            Color32::TRANSPARENT
        };
        let corner = CornerRadius {
            nw: if i == 0 { r.nw } else { 0 },
            sw: if i == 0 { r.sw } else { 0 },
            ne: if i + 1 == items.len() { r.ne } else { 0 },
            se: if i + 1 == items.len() { r.se } else { 0 },
        };
        let p = ui.painter();
        p.rect_filled(item, corner, fill);
        let fg = if on { t.accent_foreground } else { t.foreground };
        let g = p.layout_no_wrap(label.to_string(), font.clone(), fg);
        p.galley(item.center() - g.size() / 2.0, g, fg);
        if i > 0 {
            p.vline(item.left(), item.y_range(), Stroke::new(1.0, t.input));
        }
    }
    ui.painter().rect_stroke(rect, r, Stroke::new(1.0, t.input), StrokeKind::Inside);
    response
}
