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
    rows(ui, diff);
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
        });
    });
}

fn rows(ui: &mut Ui, diff: &Diff) {
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
                let (rect, _) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::hover());
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
                        painter.text(
                            pos2(rect.left() + code_x, y),
                            Align2::LEFT_CENTER,
                            header,
                            font.clone(),
                            theme.muted_foreground,
                        );
                    }
                    DiffRow::Line(f, h, l) => {
                        let line = &diff.files[f].hunks[h].lines[l];
                        let (bg, marker) = match line.kind {
                            LineKind::Added => (Some(GREEN), "+"),
                            LineKind::Removed => (Some(RED), "-"),
                            LineKind::Context | LineKind::NoNewline => (None, ""),
                        };
                        if let Some(c) = bg {
                            let t = if theme.dark { 0.22 } else { 0.14 };
                            painter.rect_filled(rect, 0.0, mix(theme.background, c, t));
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
