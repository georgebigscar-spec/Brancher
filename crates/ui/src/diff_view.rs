//! Панель diff: заголовки файлов, ханки и строки с номерами. Виртуальная прокрутка
//! по плоскому списку [`brancher_core::DiffRow`], рисуются только видимые строки.

use brancher_core::{Action, Diff, DiffRow, DiffTarget, FileDiff, LineKind, RepoState, TAB_WIDTH};
use egui::{pos2, vec2, Align2, Color32, FontId, Rect, Sense, Ui};
use widgets::{mix, Theme};

const ROW_HEIGHT: f32 = 20.0;
const FONT_SIZE: f32 = 13.0;
/// Колонка одного номера строки.
const NUMBER_WIDTH: f32 = 44.0;
const MARKER_WIDTH: f32 = 16.0;

const GREEN: Color32 = Color32::from_rgb(0x22, 0xa5, 0x5e);
const RED: Color32 = Color32::from_rgb(0xef, 0x44, 0x44);
const BLUE: Color32 = Color32::from_rgb(0x3b, 0x82, 0xf6);

pub fn show(ui: &mut Ui, repo: &RepoState, actions: &mut Vec<Action>) {
    let Some(target) = &repo.diff_target else {
        return;
    };
    header(ui, repo, target, actions);
    let Some(diff) = &repo.diff else {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.spinner();
            widgets::muted(ui, "Загрузка diff…");
        });
        return;
    };
    if diff.files.is_empty() {
        widgets::muted(ui, "Изменений нет.");
        return;
    }
    rows(ui, repo, diff, actions);
}

fn header(ui: &mut Ui, repo: &RepoState, target: &DiffTarget, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        let title = match target {
            DiffTarget::Commit { id, .. } => {
                let subject = repo
                    .history
                    .iter()
                    .find(|c| &c.id == id)
                    .map(|c| c.subject.as_str())
                    .unwrap_or_default();
                format!("{} {subject}", &id[..id.len().min(10)])
            }
            DiffTarget::Change { path, .. } => path.clone(),
        };
        ui.add(egui::Label::new(egui::RichText::new(title).strong()).truncate());
        match target {
            DiffTarget::Change { staged: true, .. } => {
                ui.add(widgets::Badge::new("индекс").secondary());
            }
            DiffTarget::Change { untracked: true, .. } => {
                ui.add(widgets::Badge::new("новый файл").outline());
            }
            DiffTarget::Change { .. } => {
                ui.add(widgets::Badge::new("рабочая копия").outline());
            }
            DiffTarget::Commit { .. } => {}
        }
        if let Some(diff) = &repo.diff {
            let (add, del) = diff
                .files
                .iter()
                .map(FileDiff::stats)
                .fold((0, 0), |a, s| (a.0 + s.0, a.1 + s.1));
            ui.label(egui::RichText::new(format!("+{add}")).color(GREEN));
            ui.label(egui::RichText::new(format!("−{del}")).color(RED));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let close = ui
                .add(widgets::Button::ghost("×").size(widgets::ButtonSize::Icon))
                .on_hover_text("Закрыть diff (Esc)");
            let esc = ui.memory(|m| m.focused().is_none()) && ui.input(|i| i.key_pressed(egui::Key::Escape));
            if close.clicked() || esc {
                actions.push(Action::CloseDiff);
            }
            selection_controls(ui, repo, target, actions);
        });
    });
}

/// Кнопка «Stage строки (N)» или подсказка, как выделять строки.
fn selection_controls(ui: &mut Ui, repo: &RepoState, target: &DiffTarget, actions: &mut Vec<Action>) {
    if !repo.can_select_lines() {
        return;
    }
    let staged = matches!(target, DiffTarget::Change { staged: true, .. });
    let n = repo.diff_selection.len();
    if n == 0 {
        widgets::muted(ui, "Клик или Shift+клик по строкам, чтобы выбрать");
        return;
    }
    let verb = if staged { "Unstage" } else { "Stage" };
    let button = ui
        .add(widgets::Button::new(format!("{verb} строки ({n})")).size(widgets::ButtonSize::Sm))
        .on_hover_text("S");
    let key = ui.memory(|m| m.focused().is_none()) && ui.input(|i| i.key_pressed(egui::Key::S));
    if button.clicked() || key {
        actions.push(Action::ApplyDiffSelection);
    }
}

fn rows(ui: &mut Ui, repo: &RepoState, diff: &Diff, actions: &mut Vec<Action>) {
    let selectable = repo.can_select_lines();
    let staged = matches!(repo.diff_target, Some(DiffTarget::Change { staged: true, .. }));
    let theme = Theme::get(ui.ctx());
    let font = FontId::monospace(FONT_SIZE);
    let char_width = ui.fonts_mut(|f| f.glyph_width(&font, 'M'));
    let code_x = NUMBER_WIDTH * 2.0 + MARKER_WIDTH;
    let content_width = code_x + char_width * diff.max_line_len as f32 + 16.0;

    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::both()
        .id_salt("diff")
        .auto_shrink([false, false])
        .show_rows(ui, ROW_HEIGHT, diff.rows.len(), |ui, range| {
            let width = ui.available_width().max(content_width);
            for i in range {
                let change_line = match diff.rows[i] {
                    DiffRow::Line(f, h, l) => {
                        let kind = diff.files[f].hunks[h].lines[l].kind;
                        matches!(kind, LineKind::Added | LineKind::Removed)
                    }
                    _ => false,
                };
                let sense = if selectable && change_line {
                    Sense::click()
                } else {
                    Sense::hover()
                };
                let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), sense);
                if let (true, DiffRow::Line(file, hunk, line)) = (response.clicked(), diff.rows[i]) {
                    let m = ui.input(|i| i.modifiers);
                    actions.push(Action::SelectDiffLine {
                        file,
                        hunk,
                        line,
                        extend: m.shift,
                        toggle: m.command,
                    });
                }
                if !ui.is_rect_visible(rect) {
                    continue;
                }
                let painter = ui.painter_at(rect);
                let y = rect.center().y;
                match diff.rows[i] {
                    DiffRow::File(f) => file_row(&painter, rect, &diff.files[f], &theme),
                    DiffRow::Hunk(f, h) => {
                        painter.rect_filled(rect, 0.0, mix(theme.background, BLUE, 0.08));
                        let header = &diff.files[f].hunks[h].header;
                        let text = painter.text(
                            pos2(rect.left() + code_x, y),
                            Align2::LEFT_CENTER,
                            header,
                            font.clone(),
                            theme.muted_foreground,
                        );
                        if selectable {
                            let label = if staged {
                                "Unstage ханк"
                            } else {
                                "Stage ханк"
                            };
                            if link(ui, &painter, pos2(text.right() + 16.0, y), label, (f, h), &theme) {
                                actions.push(Action::ApplyHunk { file: f, hunk: h });
                            }
                        }
                    }
                    DiffRow::Line(f, h, l) => {
                        let line = &diff.files[f].hunks[h].lines[l];
                        let (bg, marker) = match line.kind {
                            LineKind::Added => (Some(GREEN), "+"),
                            LineKind::Removed => (Some(RED), "-"),
                            LineKind::Context | LineKind::NoNewline => (None, ""),
                        };
                        let selected = repo.diff_selection.contains(&(f, h, l));
                        if let Some(c) = bg {
                            let t = match (selected, theme.dark) {
                                (true, true) => 0.45,
                                (true, false) => 0.32,
                                (false, true) => 0.22,
                                (false, false) => 0.14,
                            };
                            painter.rect_filled(rect, 0.0, mix(theme.background, c, t));
                        }
                        if selected {
                            let bar = Rect::from_min_size(rect.min, vec2(3.0, ROW_HEIGHT));
                            painter.rect_filled(bar, 0.0, bg.unwrap_or(theme.foreground));
                        } else if response.hovered() && selectable && change_line {
                            painter.rect_filled(rect, 0.0, mix(Color32::TRANSPARENT, theme.foreground, 0.04));
                        }
                        let number = |n: Option<u32>, right: f32| {
                            if let Some(n) = n {
                                painter.text(
                                    pos2(rect.left() + right - 6.0, y),
                                    Align2::RIGHT_CENTER,
                                    n.to_string(),
                                    font.clone(),
                                    theme.muted_foreground,
                                );
                            }
                        };
                        number(line.old_no, NUMBER_WIDTH);
                        number(line.new_no, NUMBER_WIDTH * 2.0);
                        painter.text(
                            pos2(rect.left() + NUMBER_WIDTH * 2.0 + 4.0, y),
                            Align2::LEFT_CENTER,
                            marker,
                            font.clone(),
                            theme.muted_foreground,
                        );
                        let (text, color) = if line.kind == LineKind::NoNewline {
                            (line.text.clone(), theme.muted_foreground)
                        } else {
                            (line.text.replace('\t', &" ".repeat(TAB_WIDTH)), theme.foreground)
                        };
                        painter.text(
                            pos2(rect.left() + code_x, y),
                            Align2::LEFT_CENTER,
                            text,
                            font.clone(),
                            color,
                        );
                    }
                }
            }
        });
}

/// Текстовая кнопка внутри строки, нарисованной вручную. Возвращает `true` при клике.
fn link(
    ui: &Ui,
    painter: &egui::Painter,
    at: egui::Pos2,
    text: &str,
    id: (usize, usize),
    theme: &Theme,
) -> bool {
    let font = FontId::proportional(12.0);
    let galley = painter.layout_no_wrap(text.to_owned(), font, theme.foreground);
    let rect =
        Rect::from_min_size(pos2(at.x, at.y - galley.size().y / 2.0), galley.size()).expand2(vec2(6.0, 1.0));
    let response = ui
        .interact(rect, ui.id().with(("hunk-action", id)), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let fill = if response.hovered() {
        theme.accent
    } else {
        theme.background
    };
    painter.rect(
        rect,
        theme.radius_sm(),
        fill,
        egui::Stroke::new(1.0, theme.border),
        egui::StrokeKind::Inside,
    );
    painter.galley(
        pos2(rect.left() + 6.0, rect.top() + 1.0),
        galley,
        theme.foreground,
    );
    response.clicked()
}

fn file_row(painter: &egui::Painter, rect: Rect, file: &FileDiff, theme: &Theme) {
    painter.rect_filled(rect, 0.0, theme.muted);
    let y = rect.center().y;
    let path = match (&file.old_path, &file.new_path) {
        (Some(a), Some(b)) if a != b => format!("{a} → {b}"),
        _ => file.path().to_owned(),
    };
    let bold = FontId::proportional(14.0);
    let r = painter.text(
        pos2(rect.left() + 8.0, y),
        Align2::LEFT_CENTER,
        path,
        bold,
        theme.foreground,
    );
    let note = match (&file.old_path, &file.new_path, file.binary) {
        (_, _, true) => "бинарный файл".to_owned(),
        (None, _, _) => "добавлен".to_owned(),
        (_, None, _) => "удалён".to_owned(),
        _ => {
            let (a, d) = file.stats();
            format!("+{a} −{d}")
        }
    };
    painter.text(
        pos2(r.right() + 12.0, y),
        Align2::LEFT_CENTER,
        note,
        FontId::proportional(13.0),
        theme.muted_foreground,
    );
}
