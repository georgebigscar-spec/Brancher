//! Tree view in the style of the shadcn sidebar file tree.

use egui::{
    collapsing_header::CollapsingState, epaint::PathShape, Align2, CursorIcon, EventFilter, FontId,
    Id, InnerResponse, Key, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2,
    WidgetInfo, WidgetType,
};

use crate::basic::paint_chevron;
use crate::theme::{paint_focus_ring, Theme};

const ROW_HEIGHT: f32 = 32.0;
const INDENT: f32 = 20.0;

/// A tree of folders and leaves.
///
/// ```ignore
/// Tree::new("files").default_open_depth(1).show(ui, &mut selected, |tree| {
///     tree.folder("src", "src", |tree| {
///         tree.leaf("src/main.rs", "main.rs");
///     });
///     tree.leaf("Cargo.toml", "Cargo.toml");
/// });
/// ```
///
/// `selected` is an `Option<T>` for single selection, or a `HashSet<T>` /
/// `Vec<T>` for multiple selection (see [`TreeSelection`]).
///
/// Click a row (or press Space/Enter) to select it; clicking a folder also
/// opens or closes it. Up/Down move between rows, Right opens a folder and
/// Left closes it. With multiple selection, Ctrl/Cmd+click toggles a row,
/// Shift+click selects the range from the last clicked row, and Ctrl/Cmd+A
/// selects every visible row.
pub struct Tree {
    id: Id,
    open_depth: usize,
    icons: bool,
    max_height: Option<f32>,
}

impl Tree {
    pub fn new(id_salt: impl egui::AsId) -> Self {
        Self { id: Id::new(id_salt), open_depth: 0, icons: true, max_height: None }
    }

    /// Folders less than `depth` levels deep start open. Default 0: all closed.
    pub fn default_open_depth(mut self, depth: usize) -> Self {
        self.open_depth = depth;
        self
    }

    /// Show folder and file icons (on by default).
    pub fn icons(mut self, icons: bool) -> Self {
        self.icons = icons;
        self
    }

    /// Limits the height; taller trees scroll, and arrow keys keep the focused row in view.
    pub fn max_height(mut self, height: f32) -> Self {
        self.max_height = Some(height);
        self
    }

    /// Draws the tree. The response is marked changed when the selection changes.
    pub fn show<T: PartialEq + Clone, R>(
        self,
        ui: &mut Ui,
        selected: &mut impl TreeSelection<T>,
        add_nodes: impl FnOnce(&mut TreeUi<'_, T>) -> R,
    ) -> InnerResponse<R> {
        let multiple = selected.multiple();
        let mut shared = Shared { changed: false, multiple, order: Vec::new(), range_to: None, focused: false };
        let max_height = self.max_height;
        let id = self.id;
        let draw = |ui: &mut Ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let mut tree = TreeUi {
                    ui,
                    selected: &mut *selected,
                    shared: &mut shared,
                    id: self.id,
                    root: self.id,
                    depth: 0,
                    open_depth: self.open_depth,
                    icons: self.icons,
                };
                add_nodes(&mut tree)
            })
        };
        let mut inner = match max_height {
            Some(h) => {
                let out = egui::ScrollArea::vertical()
                    .id_salt(id.with("scroll"))
                    .max_height(h)
                    .auto_shrink([false, true])
                    .show(ui, draw);
                InnerResponse::new(out.inner.inner, out.inner.response)
            }
            None => draw(ui),
        };

        // Shift+click and Ctrl+A need every visible row, so they are applied
        // after all rows have been laid out.
        let anchor_id = id.with("anchor");
        if let Some(target) = shared.range_to {
            let anchor = ui.data(|d| d.get_temp::<Id>(anchor_id));
            let pos = |row: Id| shared.order.iter().position(|(r, _)| *r == row);
            let to = pos(target);
            let from = anchor.and_then(pos).or(to);
            if let (Some(a), Some(b)) = (from, to) {
                selected.clear();
                for (_, v) in &shared.order[a.min(b)..=a.max(b)] {
                    selected.add(v.clone());
                }
                shared.changed = true;
            }
        }
        if multiple && shared.focused && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::A)) {
            selected.clear();
            for (_, v) in &shared.order {
                selected.add(v.clone());
            }
            shared.changed = true;
        }
        if shared.changed {
            inner.response.mark_changed();
            ui.request_repaint(); // rows drawn earlier this frame still show the old selection
        }
        inner
    }
}

/// What [`Tree::show`] stores the selection in.
///
/// Implemented for `Option<T>` (one row), and for `HashSet<T>` and `Vec<T>`
/// (several rows, with Ctrl/Shift+click and Ctrl+A).
pub trait TreeSelection<T> {
    fn is_selected(&self, value: &T) -> bool;
    /// Plain click: select only `value`.
    fn select_only(&mut self, value: T);
    /// Ctrl/Cmd+click.
    fn toggle(&mut self, value: T);
    /// Adds `value` (used for Shift+click ranges and Ctrl+A).
    fn add(&mut self, value: T);
    fn clear(&mut self);
    /// Whether Ctrl/Shift+click and Ctrl+A select several rows.
    fn multiple(&self) -> bool;
}

impl<T: PartialEq> TreeSelection<T> for Option<T> {
    fn is_selected(&self, value: &T) -> bool {
        self.as_ref() == Some(value)
    }
    fn select_only(&mut self, value: T) {
        *self = Some(value);
    }
    fn toggle(&mut self, value: T) {
        *self = Some(value);
    }
    fn add(&mut self, value: T) {
        *self = Some(value);
    }
    fn clear(&mut self) {
        *self = None;
    }
    fn multiple(&self) -> bool {
        false
    }
}

impl<T: Eq + std::hash::Hash> TreeSelection<T> for std::collections::HashSet<T> {
    fn is_selected(&self, value: &T) -> bool {
        self.contains(value)
    }
    fn select_only(&mut self, value: T) {
        self.clear();
        self.insert(value);
    }
    fn toggle(&mut self, value: T) {
        if !self.remove(&value) {
            self.insert(value);
        }
    }
    fn add(&mut self, value: T) {
        self.insert(value);
    }
    fn clear(&mut self) {
        std::collections::HashSet::clear(self);
    }
    fn multiple(&self) -> bool {
        true
    }
}

/// Keeps rows in the order they were selected.
impl<T: PartialEq> TreeSelection<T> for Vec<T> {
    fn is_selected(&self, value: &T) -> bool {
        self.contains(value)
    }
    fn select_only(&mut self, value: T) {
        self.clear();
        self.push(value);
    }
    fn toggle(&mut self, value: T) {
        match self.iter().position(|v| *v == value) {
            Some(i) => {
                self.remove(i);
            }
            None => self.push(value),
        }
    }
    fn add(&mut self, value: T) {
        if !self.contains(&value) {
            self.push(value);
        }
    }
    fn clear(&mut self) {
        Vec::clear(self);
    }
    fn multiple(&self) -> bool {
        true
    }
}

/// Per-frame state shared by all rows of one tree.
struct Shared<T> {
    changed: bool,
    multiple: bool,
    /// Visible rows in display order, for Shift+click ranges and Ctrl+A.
    order: Vec<(Id, T)>,
    /// The row a Shift+click ended on.
    range_to: Option<Id>,
    /// Whether any row of the tree has keyboard focus.
    focused: bool,
}

/// Draws a row's content; see [`TreeUi::leaf_ui`].
type RowContent<'c> = Option<Box<dyn FnOnce(&mut Ui) + 'c>>;

/// Adds nodes to a [`Tree`]; passed to the closures of [`Tree::show`] and [`TreeUi::folder`].
pub struct TreeUi<'a, T> {
    ui: &'a mut Ui,
    selected: &'a mut dyn TreeSelection<T>,
    shared: &'a mut Shared<T>,
    id: Id,
    root: Id,
    depth: usize,
    open_depth: usize,
    icons: bool,
}

impl<T: PartialEq + Clone> TreeUi<'_, T> {
    /// The `Ui` the rows are drawn into, for adding your own widgets between them.
    pub fn ui(&mut self) -> &mut Ui {
        self.ui
    }

    /// A row without children.
    pub fn leaf(&mut self, value: T, label: &str) -> Response {
        let id = self.id.with(label);
        self.row(id, value, label, None, None)
    }

    /// A row without children whose content you draw: `add_contents` gets a
    /// left-to-right `Ui` covering the row after the icon, in place of the label
    /// (which is still used for the open state and accessibility).
    ///
    /// ```ignore
    /// tree.leaf_ui(path, "main.rs", |ui| {
    ///     ui.label("main.rs");
    ///     ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
    ///         ui.label(RichText::new("M").color(modified_color));
    ///     });
    /// });
    /// ```
    pub fn leaf_ui(&mut self, value: T, label: &str, add_contents: impl FnOnce(&mut Ui)) -> Response {
        let id = self.id.with(label);
        self.row(id, value, label, None, Some(Box::new(add_contents)))
    }

    /// A folder whose children are added by `add_children`, which only runs while it is open.
    /// The open state is kept per label, so siblings need distinct labels.
    pub fn folder(&mut self, value: T, label: &str, add_children: impl FnOnce(&mut TreeUi<'_, T>)) -> Response {
        self.folder_impl(value, label, None, add_children)
    }

    /// A folder whose row content you draw, like [`TreeUi::leaf_ui`].
    pub fn folder_ui(
        &mut self,
        value: T,
        label: &str,
        add_contents: impl FnOnce(&mut Ui),
        add_children: impl FnOnce(&mut TreeUi<'_, T>),
    ) -> Response {
        self.folder_impl(value, label, Some(Box::new(add_contents)), add_children)
    }

    fn folder_impl(
        &mut self,
        value: T,
        label: &str,
        content: RowContent<'_>,
        add_children: impl FnOnce(&mut TreeUi<'_, T>),
    ) -> Response {
        let id = self.id.with(label);
        let mut state = CollapsingState::load_with_default_open(self.ui.ctx(), id, self.depth < self.open_depth);
        let response = self.row(id, value, label, Some(&mut state), content);

        let line_x = self.ui.max_rect().left() + self.depth as f32 * INDENT + 16.0;
        let body = state.show_body_unindented(self.ui, |ui| {
            let mut child = TreeUi {
                ui,
                selected: &mut *self.selected,
                shared: &mut *self.shared,
                id,
                root: self.root,
                depth: self.depth + 1,
                open_depth: self.open_depth,
                icons: self.icons,
            };
            add_children(&mut child);
        });
        if let Some(body) = body {
            let t = Theme::get(self.ui.ctx());
            let r = body.response.rect;
            self.ui.painter().vline(line_x, r.top()..=r.bottom(), Stroke::new(1.0, t.border));
        }
        response
    }

    fn row(
        &mut self,
        id: Id,
        value: T,
        label: &str,
        mut folder: Option<&mut CollapsingState>,
        content: RowContent<'_>,
    ) -> Response {
        let ui = &mut *self.ui;
        let t = Theme::get(ui.ctx());
        let left = ui.max_rect().left() + self.depth as f32 * INDENT;
        let width = (ui.max_rect().right() - left).max(0.0);
        let (full, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_HEIGHT), Sense::hover());
        let rect = Rect::from_min_size(Pos2::new(left, full.top()), Vec2::new(width, ROW_HEIGHT));
        let response = ui
            .interact(rect, id.with("row"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);

        let is_selected = self.selected.is_selected(&value);
        if self.shared.multiple {
            self.shared.order.push((response.id, value.clone()));
        }
        response.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, ui.is_enabled(), is_selected, label));

        // Clicking focuses the row so the arrow keys work afterwards, but the focus
        // ring only shows once the keyboard is used (like `:focus-visible`).
        let pointer_focus_id = self.root.with("pointer_focus");
        if response.clicked() {
            response.request_focus();
            ui.data_mut(|d| d.insert_temp(pointer_focus_id, true));
        }
        let anchor_id = self.root.with("anchor");
        let modifiers = ui.input(|i| i.modifiers);
        let multi_click = self.shared.multiple && response.clicked() && (modifiers.command || modifiers.shift);
        let mut select = response.clicked() && !multi_click;
        if multi_click {
            if modifiers.shift {
                self.shared.range_to = Some(response.id);
            } else {
                self.selected.toggle(value.clone());
                ui.data_mut(|d| d.insert_temp(anchor_id, response.id));
                self.shared.changed = true;
            }
        }
        if response.has_focus() {
            self.shared.focused = true;
            if ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. }))) {
                ui.data_mut(|d| d.insert_temp(pointer_focus_id, false));
            }
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(response.id, EventFilter { horizontal_arrows: true, ..Default::default() })
            });
            if let Some(state) = folder.as_deref_mut() {
                let (open, close) = ui.input(|i| (i.key_pressed(Key::ArrowRight), i.key_pressed(Key::ArrowLeft)));
                if open != close && state.is_open() != open {
                    state.set_open(open);
                    ui.request_repaint();
                }
            }
            // egui moves the focus to the next row at the end of the frame with the
            // key press, so remember the press and select the row that gets the focus.
            let arrow_nav_id = self.root.with("arrow_nav");
            let focused_row_id = self.root.with("focused_row");
            let newly_focused = ui.data(|d| d.get_temp::<Id>(focused_row_id)) != Some(response.id);
            if newly_focused {
                ui.data_mut(|d| d.insert_temp(focused_row_id, response.id));
                if !response.clicked() {
                    response.scroll_to_me(None); // keep keyboard focus visible inside a ScrollArea
                }
                let pressed_at = ui.data_mut(|d| d.remove_temp::<u64>(arrow_nav_id));
                select |= pressed_at.is_some_and(|p| ui.ctx().cumulative_pass_nr() <= p + 2);
            }
            if ui.input(|i| i.key_pressed(Key::ArrowUp) || i.key_pressed(Key::ArrowDown)) {
                let pass = ui.ctx().cumulative_pass_nr();
                ui.data_mut(|d| d.insert_temp(arrow_nav_id, pass));
            }
        }
        if response.clicked() && !multi_click {
            if let Some(state) = folder.as_deref_mut() {
                state.toggle(ui);
            }
        }
        if select {
            ui.data_mut(|d| d.insert_temp(anchor_id, response.id));
            self.shared.changed |= !is_selected || self.shared.multiple;
            self.selected.select_only(value.clone());
        }
        let is_selected = self.selected.is_selected(&value);

        let p = ui.painter();
        if is_selected || response.hovered() {
            p.rect_filled(rect, t.radius_md(), t.accent);
        }
        let fg = if is_selected { t.accent_foreground } else { t.foreground };
        let mut x = rect.left() + 8.0;
        let cy = rect.center().y;
        if let Some(state) = folder.as_deref() {
            let openness = state.openness(ui.ctx());
            paint_chevron(p, Pos2::new(x + 8.0, cy), (openness - 1.0) * std::f32::consts::FRAC_PI_2, t.muted_foreground);
        }
        x += 20.0;
        if self.icons {
            let center = Pos2::new(x + 8.0, cy);
            if folder.is_some() {
                paint_folder_icon(p, center, t.muted_foreground);
            } else {
                paint_file_icon(p, center, t.muted_foreground);
            }
            x += 24.0;
        }
        let text_rect = Rect::from_min_max(Pos2::new(x, rect.top()), Pos2::new(rect.right() - 8.0, rect.bottom()));
        let clip = text_rect.intersect(p.clip_rect());
        match content {
            Some(add_contents) => {
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .id_salt(id.with("content"))
                        .max_rect(text_rect)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                child.set_clip_rect(clip);
                child.spacing_mut().item_spacing.x = 6.0;
                child.style_mut().interaction.selectable_labels = false;
                child.style_mut().visuals.override_text_color = Some(fg);
                add_contents(&mut child);
            }
            None => {
                p.with_clip_rect(clip).text(Pos2::new(x, cy), Align2::LEFT_CENTER, label, FontId::proportional(14.0), fg);
            }
        }
        let p = ui.painter();
        if response.has_focus() && !ui.data(|d| d.get_temp(pointer_focus_id).unwrap_or(false)) {
            paint_focus_ring(p, rect, t.radius_md(), &t);
        }
        response
    }
}

/// Lucide-like icons drawn on a 24-unit grid scaled to 16 px.
fn icon_path(p: &Painter, center: Pos2, points: &[(f32, f32)], closed: bool, color: egui::Color32) {
    let s = 16.0 / 24.0;
    let pts = points.iter().map(|&(x, y)| center + Vec2::new(x - 12.0, y - 12.0) * s).collect();
    let stroke = Stroke::new(1.3, color);
    p.add(Shape::Path(if closed { PathShape::closed_line(pts, stroke) } else { PathShape::line(pts, stroke) }));
}

fn paint_folder_icon(p: &Painter, center: Pos2, color: egui::Color32) {
    icon_path(p, center, &[(2.5, 4.0), (9.0, 4.0), (11.0, 7.0), (21.5, 7.0), (21.5, 20.0), (2.5, 20.0)], true, color);
}

fn paint_file_icon(p: &Painter, center: Pos2, color: egui::Color32) {
    icon_path(p, center, &[(5.0, 2.0), (14.0, 2.0), (19.5, 7.5), (19.5, 22.0), (5.0, 22.0)], true, color);
    icon_path(p, center, &[(14.0, 2.0), (14.0, 7.5), (19.5, 7.5)], false, color);
}
