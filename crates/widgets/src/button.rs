use egui::{
    Color32, CursorIcon, FontId, Response, Sense, Stroke, StrokeKind, Ui, Vec2, Widget, WidgetInfo,
    WidgetType,
};

use crate::theme::{mix, paint_focus_ring, Theme};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonVariant {
    #[default]
    Default,
    Destructive,
    Outline,
    Secondary,
    Ghost,
    Link,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonSize {
    Sm,
    #[default]
    Default,
    Lg,
    /// Square button for a single icon glyph.
    Icon,
}

/// `<Button variant="..." size="...">`.
#[must_use = "add it with `ui.add(...)`"]
pub struct Button {
    text: String,
    variant: ButtonVariant,
    size: ButtonSize,
    full_width: bool,
}

impl Button {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            variant: ButtonVariant::Default,
            size: ButtonSize::Default,
            full_width: false,
        }
    }

    pub fn destructive(text: impl Into<String>) -> Self {
        Self::new(text).variant(ButtonVariant::Destructive)
    }
    pub fn outline(text: impl Into<String>) -> Self {
        Self::new(text).variant(ButtonVariant::Outline)
    }
    pub fn secondary(text: impl Into<String>) -> Self {
        Self::new(text).variant(ButtonVariant::Secondary)
    }
    pub fn ghost(text: impl Into<String>) -> Self {
        Self::new(text).variant(ButtonVariant::Ghost)
    }
    pub fn link(text: impl Into<String>) -> Self {
        Self::new(text).variant(ButtonVariant::Link)
    }

    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }
    pub fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }
    /// Stretch to the available width (`w-full`).
    pub fn full_width(mut self) -> Self {
        self.full_width = true;
        self
    }
}

impl Widget for Button {
    fn ui(self, ui: &mut Ui) -> Response {
        let t = Theme::get(ui.ctx());
        let (height, pad_x) = match self.size {
            ButtonSize::Sm => (32.0, 12.0),
            ButtonSize::Default => (36.0, 16.0),
            ButtonSize::Lg => (40.0, 24.0),
            ButtonSize::Icon => (36.0, 0.0),
        };
        let galley =
            ui.painter()
                .layout_no_wrap(self.text.clone(), FontId::proportional(14.0), Color32::PLACEHOLDER);
        let width = if self.size == ButtonSize::Icon {
            height
        } else if self.full_width {
            ui.available_width()
        } else {
            galley.size().x + 2.0 * pad_x
        };

        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), &self.text));
        let response = response.on_hover_cursor(CursorIcon::PointingHand);

        if ui.is_rect_visible(rect) {
            let hovered = response.hovered();
            let pressed = response.is_pointer_button_down_on();
            let k = if pressed { 0.2 } else if hovered { 0.1 } else { 0.0 };
            let none = Stroke::NONE;
            let (fill, fg, stroke) = match self.variant {
                ButtonVariant::Default => (mix(t.primary, t.background, k), t.primary_foreground, none),
                ButtonVariant::Destructive => {
                    (mix(t.destructive, t.background, k), t.destructive_foreground, none)
                }
                ButtonVariant::Secondary => {
                    let base = t.secondary;
                    (mix(base, t.foreground, k * 0.3), t.secondary_foreground, none)
                }
                ButtonVariant::Outline => (
                    if hovered { t.accent } else { t.background },
                    if hovered { t.accent_foreground } else { t.foreground },
                    Stroke::new(1.0, t.input),
                ),
                ButtonVariant::Ghost => (
                    if hovered { t.accent } else { Color32::TRANSPARENT },
                    if hovered { t.accent_foreground } else { t.foreground },
                    none,
                ),
                ButtonVariant::Link => (Color32::TRANSPARENT, t.primary, none),
            };

            let painter = ui.painter();
            let radius = t.radius_md();
            if self.variant != ButtonVariant::Link {
                if matches!(self.variant, ButtonVariant::Outline) {
                    painter.add(t.shadow_sm().as_shape(rect, radius));
                }
                painter.rect(rect, radius, fill, stroke, StrokeKind::Inside);
            }
            let text_pos = rect.center() - galley.size() / 2.0;
            let text_rect = egui::Rect::from_min_size(text_pos, galley.size());
            painter.galley(text_pos, galley, fg);
            if self.variant == ButtonVariant::Link && hovered {
                let y = text_rect.bottom() + 1.0;
                painter.line_segment(
                    [egui::pos2(text_rect.left(), y), egui::pos2(text_rect.right(), y)],
                    Stroke::new(1.0, fg),
                );
            }
            if response.has_focus() {
                paint_focus_ring(painter, rect, radius, &t);
            }
        }
        response
    }
}
