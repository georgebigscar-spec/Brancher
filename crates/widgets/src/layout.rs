//! Structural components: Card, Alert, Tabs, Accordion, Table.

use egui::{
    collapsing_header::CollapsingState, Align2, Color32, CursorIcon, FontId, Frame, Id,
    InnerResponse, Margin, Pos2, Rect, Response, RichText, Sense, Stroke, StrokeKind, Ui, Vec2,
};

use crate::basic::paint_chevron;
use crate::theme::{paint_focus_ring, Theme};

// ---------------------------------------------------------------------------

/// `<Card>` with optional `<CardTitle>` and `<CardDescription>`.
pub struct Card {
    title: Option<String>,
    description: Option<String>,
    width: Option<f32>,
}

impl Card {
    pub fn new() -> Self {
        Self { title: None, description: None, width: None }
    }
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    pub fn show<R>(self, ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
        let t = Theme::get(ui.ctx());
        Frame::new()
            .fill(t.card)
            .stroke(Stroke::new(1.0, t.border))
            .corner_radius(t.radius_xl())
            .inner_margin(Margin::same(24))
            .shadow(t.shadow_sm())
            .show(ui, |ui| {
                if let Some(w) = self.width {
                    ui.set_width(w - 48.0);
                }
                ui.spacing_mut().item_spacing.y = 8.0;
                if let Some(title) = &self.title {
                    ui.label(RichText::new(title).size(16.0).strong().color(t.card_foreground));
                }
                if let Some(d) = &self.description {
                    ui.add_space(-4.0);
                    ui.label(RichText::new(d).size(14.0).color(t.muted_foreground));
                }
                if self.title.is_some() || self.description.is_some() {
                    ui.add_space(12.0);
                }
                add_contents(ui)
            })
    }
}

impl Default for Card {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------

/// `<Alert>` with an icon glyph, title and description.
pub struct Alert {
    title: String,
    description: Option<String>,
    icon: Option<String>,
    destructive: bool,
}

impl Alert {
    pub fn new(title: impl Into<String>) -> Self {
        Self { title: title.into(), description: None, icon: None, destructive: false }
    }
    pub fn description(mut self, text: impl Into<String>) -> Self {
        self.description = Some(text.into());
        self
    }
    /// Any glyph, for example "ℹ" or "⚠".
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }
    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let t = Theme::get(ui.ctx());
        let (title_c, desc_c) = if self.destructive {
            (t.destructive, t.destructive.gamma_multiply(0.9))
        } else {
            (t.card_foreground, t.muted_foreground)
        };
        Frame::new()
            .fill(t.card)
            .stroke(Stroke::new(1.0, t.border))
            .corner_radius(t.radius_lg())
            .inner_margin(Margin::symmetric(16, 12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_top(|ui| {
                    if let Some(icon) = &self.icon {
                        ui.label(RichText::new(icon).size(15.0).color(title_c));
                    }
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(RichText::new(&self.title).size(14.0).strong().color(title_c));
                        if let Some(d) = &self.description {
                            ui.label(RichText::new(d).size(14.0).color(desc_c));
                        }
                    });
                });
            })
            .response
    }
}

// ---------------------------------------------------------------------------

/// `<Tabs>`: the segmented tab list. Render the matching content yourself
/// based on `selected`.
pub fn tabs(ui: &mut Ui, selected: &mut usize, labels: &[&str]) -> Response {
    let t = Theme::get(ui.ctx());
    let font = FontId::proportional(14.0);
    let widths: Vec<f32> = labels
        .iter()
        .map(|l| ui.painter().layout_no_wrap(l.to_string(), font.clone(), Color32::PLACEHOLDER).size().x + 24.0)
        .collect();
    let pad = 3.0;
    let size = Vec2::new(widths.iter().sum::<f32>() + 2.0 * pad, 36.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, t.radius_lg(), t.muted);
    let mut x = rect.left() + pad;
    for (i, (label, w)) in labels.iter().zip(&widths).enumerate() {
        let item = Rect::from_min_size(Pos2::new(x, rect.top() + pad), Vec2::new(*w, 36.0 - 2.0 * pad));
        x += w;
        let r = ui
            .interact(item, response.id.with(i), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        if r.clicked() && *selected != i {
            *selected = i;
            response.mark_changed();
        }
        let active = *selected == i;
        let p = ui.painter();
        if active {
            let fill = if t.dark { t.input.gamma_multiply(0.3) } else { t.background };
            p.add(t.shadow_sm().as_shape(item, t.radius_md()));
            let stroke = if t.dark { Stroke::new(1.0, t.input) } else { Stroke::NONE };
            p.rect(item, t.radius_md(), if t.dark { t.background } else { fill }, stroke, StrokeKind::Inside);
        }
        let fg = if active || r.hovered() { t.foreground } else { t.muted_foreground };
        p.text(item.center(), Align2::CENTER_CENTER, label, font.clone(), fg);
        if r.has_focus() {
            paint_focus_ring(p, item, t.radius_md(), &t);
        }
    }
    response
}

// ---------------------------------------------------------------------------

/// `<Accordion type="single" collapsible>`: only one item open at a time.
pub struct Accordion {
    id: Id,
}

impl Accordion {
    pub fn new(id_salt: impl egui::AsId) -> Self {
        Self { id: Id::new(id_salt) }
    }

    /// Adds one `<AccordionItem>`.
    pub fn item(&self, ui: &mut Ui, title: &str, add_body: impl FnOnce(&mut Ui)) {
        let t = Theme::get(ui.ctx());
        let item_id = self.id.with(title);
        let open_id = self.id.with("open");
        let open_item: Option<Id> = ui.data(|d| d.get_temp(open_id)).flatten();
        let mut state = CollapsingState::load_with_default_open(ui.ctx(), item_id, false);
        state.set_open(open_item == Some(item_id));

        let width = ui.available_width();
        let (rect, header) = ui.allocate_exact_size(Vec2::new(width, 48.0), Sense::click());
        let header = header.on_hover_cursor(CursorIcon::PointingHand);
        if header.clicked() {
            let next = if open_item == Some(item_id) { None } else { Some(item_id) };
            ui.data_mut(|d| d.insert_temp(open_id, next));
            state.set_open(next.is_some());
        }
        let openness = state.openness(ui.ctx());
        let p = ui.painter();
        let galley = p.layout_no_wrap(title.to_owned(), FontId::proportional(14.0), t.foreground);
        let text_pos = Pos2::new(rect.left(), rect.center().y - galley.size().y / 2.0);
        if header.hovered() {
            let y = text_pos.y + galley.size().y + 1.0;
            p.hline(rect.left()..=(rect.left() + galley.size().x), y, Stroke::new(1.0, t.foreground));
        }
        p.galley(text_pos, galley, t.foreground);
        paint_chevron(
            p,
            Pos2::new(rect.right() - 8.0, rect.center().y),
            openness * std::f32::consts::PI,
            t.muted_foreground,
        );
        if header.has_focus() {
            paint_focus_ring(p, rect, t.radius_md(), &t);
        }
        state.show_body_unindented(ui, |ui| {
            ui.add_space(-4.0);
            ui.scope(|ui| {
                ui.style_mut().visuals.override_text_color = Some(t.foreground);
                add_body(ui);
            });
            ui.add_space(8.0);
        });

        let (line, _) = ui.allocate_exact_size(Vec2::new(width, 1.0), Sense::hover());
        ui.painter().rect_filled(line, 0.0, t.border);
        ui.add_space(-ui.spacing().item_spacing.y);
    }
}

// ---------------------------------------------------------------------------

/// `<Table>`: header row in muted text, rows divided by borders, hover highlight.
pub struct Table<'a> {
    headers: &'a [&'a str],
    widths: Option<&'a [f32]>,
    right_align: &'a [usize],
}

impl<'a> Table<'a> {
    pub fn new(headers: &'a [&'a str]) -> Self {
        Self { headers, widths: None, right_align: &[] }
    }
    /// Relative column widths, for example `&[1.0, 1.0, 2.0]`.
    pub fn widths(mut self, widths: &'a [f32]) -> Self {
        self.widths = Some(widths);
        self
    }
    /// Columns whose text is right aligned (amounts, numbers).
    pub fn right_align(mut self, columns: &'a [usize]) -> Self {
        self.right_align = columns;
        self
    }

    /// Draws the table; returns the index of a clicked row.
    pub fn show<S: AsRef<str>>(self, ui: &mut Ui, rows: &[Vec<S>]) -> Option<usize> {
        let t = Theme::get(ui.ctx());
        let total = ui.available_width();
        let n = self.headers.len();
        let weights: Vec<f32> = match self.widths {
            Some(w) => w.to_vec(),
            None => vec![1.0; n],
        };
        let sum: f32 = weights.iter().sum();
        let cols: Vec<f32> = weights.iter().map(|w| w / sum * total).collect();
        let font = FontId::proportional(14.0);
        let saved = ui.spacing().item_spacing.y;
        ui.spacing_mut().item_spacing.y = 0.0;

        let mut clicked = None;
        let mut row_ui = |ui: &mut Ui, cells: Vec<&str>, header: bool, idx: usize| {
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(total, if header { 40.0 } else { 44.0 }), Sense::click());
            let p = ui.painter();
            if !header && resp.hovered() {
                p.rect_filled(rect, 0.0, t.muted.gamma_multiply(0.5));
            }
            let color = if header { t.muted_foreground } else { t.foreground };
            let mut x = rect.left();
            for (c, (cell, w)) in cells.iter().zip(&cols).enumerate() {
                let right = self.right_align.contains(&c);
                let (pos, align) = if right {
                    (Pos2::new(x + w - 8.0, rect.center().y), Align2::RIGHT_CENTER)
                } else {
                    (Pos2::new(x + 8.0, rect.center().y), Align2::LEFT_CENTER)
                };
                p.text(pos, align, cell, font.clone(), color);
                x += w;
            }
            p.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, t.border));
            if !header && resp.clicked() {
                clicked = Some(idx);
            }
        };
        row_ui(ui, self.headers.to_vec(), true, 0);
        for (i, row) in rows.iter().enumerate() {
            row_ui(ui, row.iter().map(|s| s.as_ref()).collect(), false, i);
        }
        ui.spacing_mut().item_spacing.y = saved;
        clicked
    }
}
