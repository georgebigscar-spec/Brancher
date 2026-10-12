//! Панель diff: заголовки файлов, ханки и строки с номерами. Виртуальная прокрутка
//! по плоскому списку [`brancher_core::DiffRow`], рисуются только видимые строки.

use brancher_core::{
    Action, Diff, DiffLine, DiffRow, DiffTarget, FileDiff, LineKind, RepoState, Settings, SplitRow, TAB_WIDTH,
};
use egui::text::{LayoutJob, TextFormat};
use egui::{pos2, vec2, Align2, Color32, FontId, Rect, Sense, Ui};
use widgets::{mix, Theme};

const ROW_HEIGHT: f32 = 20.0;
const FONT_SIZE: f32 = 13.0;
/// Колонка одного номера строки.
const NUMBER_WIDTH: f32 = 44.0;
const MARKER_WIDTH: f32 = 16.0;
const SELECT_HINT: &str = "Клик — выбрать строку, Shift+клик — диапазон, Ctrl+клик — добавить";

const GREEN: Color32 = Color32::from_rgb(0x22, 0xa5, 0x5e);
const RED: Color32 = Color32::from_rgb(0xef, 0x44, 0x44);
const BLUE: Color32 = Color32::from_rgb(0x3b, 0x82, 0xf6);

pub fn show(ui: &mut Ui, repo: &RepoState, settings: &Settings, actions: &mut Vec<Action>) {
    let Some(target) = &repo.diff_target else {
        return;
    };
    header(ui, repo, target, settings, actions);
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
    rows(ui, repo, diff, settings.side_by_side, actions);
}

fn header(
    ui: &mut Ui,
    repo: &RepoState,
    target: &DiffTarget,
    settings: &Settings,
    actions: &mut Vec<Action>,
) {
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
            let mut split = settings.side_by_side;
            if ui
                .add(widgets::Switch::new(&mut split).label("Две колонки"))
                .changed()
            {
                actions.push(Action::SetSideBySide(split));
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
    // Без выделения кнопок нет: подсказка про клик и Shift+клик — во всплывающей
    // подсказке строк, чтобы не теснить заголовок.
    if n == 0 {
        return;
    }
    let verb = if staged { "Unstage" } else { "Stage" };
    // Справа налево: сначала основная кнопка, левее — отмена строк.
    let button = ui
        .add(widgets::Button::new(format!("{verb} строки ({n})")).size(widgets::ButtonSize::Sm))
        .on_hover_text("S");
    let key = ui.memory(|m| m.focused().is_none()) && ui.input(|i| i.key_pressed(egui::Key::S));
    if button.clicked() || key {
        actions.push(Action::ApplyDiffSelection);
    }
    if !staged {
        let discard = ui
            .add(widgets::Button::outline("Отменить строки").size(widgets::ButtonSize::Sm))
            .on_hover_text("Удалить выбранные изменения из рабочей копии");
        if discard.clicked() {
            actions.push(Action::DiscardDiffSelection);
        }
    }
}

/// Общие для обоих режимов параметры отрисовки.
struct Paint<'a> {
    repo: &'a RepoState,
    diff: &'a Diff,
    theme: Theme,
    font: FontId,
    selectable: bool,
    staged: bool,
}

impl Paint<'_> {
    fn is_change(&self, f: usize, h: usize, l: usize) -> bool {
        let kind = self.diff.files[f].hunks[h].lines[l].kind;
        matches!(kind, LineKind::Added | LineKind::Removed)
    }

    /// Строка кода в ячейке `rect`: номера строк, маркер и текст.
    /// `numbers` — `(старый, новый)`; в side-by-side у каждой ячейки один номер.
    fn line_cell(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        at: (usize, usize, usize),
        numbers: &[Option<u32>],
        hovered: bool,
    ) {
        let (f, h, l) = at;
        let line = &self.diff.files[f].hunks[h].lines[l];
        let t = &self.theme;
        let y = rect.center().y;
        let (bg, marker) = match line.kind {
            LineKind::Added => (Some(GREEN), "+"),
            LineKind::Removed => (Some(RED), "-"),
            LineKind::Context | LineKind::NoNewline => (None, ""),
        };
        let selected = self.repo.diff_selection.contains(&at);
        if let Some(c) = bg {
            let k = match (selected, t.dark) {
                (true, true) => 0.45,
                (true, false) => 0.32,
                (false, true) => 0.22,
                (false, false) => 0.14,
            };
            painter.rect_filled(rect, 0.0, mix(t.background, c, k));
        }
        if selected {
            let bar = Rect::from_min_size(rect.min, vec2(3.0, ROW_HEIGHT));
            painter.rect_filled(bar, 0.0, bg.unwrap_or(t.foreground));
        } else if hovered && self.selectable && bg.is_some() {
            painter.rect_filled(rect, 0.0, mix(Color32::TRANSPARENT, t.foreground, 0.04));
        }
        let mut x = rect.left();
        for n in numbers {
            x += NUMBER_WIDTH;
            if let Some(n) = n {
                painter.text(
                    pos2(x - 6.0, y),
                    Align2::RIGHT_CENTER,
                    n.to_string(),
                    self.font.clone(),
                    t.muted_foreground,
                );
            }
        }
        painter.text(
            pos2(x + 4.0, y),
            Align2::LEFT_CENTER,
            marker,
            self.font.clone(),
            t.muted_foreground,
        );
        let job = self.code_job(line);
        let galley = painter.layout_job(job);
        let pos = pos2(x + MARKER_WIDTH, y - galley.size().y / 2.0);
        painter.galley(pos, galley, t.foreground);
    }

    /// Текст строки кода с подсветкой синтаксиса; табуляции раскрываются в каждом участке.
    fn code_job(&self, line: &DiffLine) -> LayoutJob {
        let t = &self.theme;
        let mut job = LayoutJob::default();
        let mut push = |text: &str, color: Color32| {
            let text = text.replace('\t', &" ".repeat(TAB_WIDTH));
            job.append(&text, 0.0, TextFormat::simple(self.font.clone(), color));
        };
        if line.kind == LineKind::NoNewline {
            push(&line.text, t.muted_foreground);
            return job;
        }
        let text = line.text.as_str();
        let mut at = 0;
        for span in &line.spans {
            let (start, end) = (span.start as usize, span.end as usize);
            let (Some(_), Some(part)) = (text.get(at..start), text.get(start..end)) else {
                continue;
            };
            push(&text[at..start], t.foreground);
            let [r, g, b] = if t.dark { span.dark } else { span.light };
            push(part, Color32::from_rgb(r, g, b));
            at = end;
        }
        push(&text[at..], t.foreground);
        job
    }

    fn hunk_row(
        &self,
        ui: &Ui,
        painter: &egui::Painter,
        rect: Rect,
        (f, h): (usize, usize),
        x: f32,
        actions: &mut Vec<Action>,
    ) {
        let t = &self.theme;
        let y = rect.center().y;
        painter.rect_filled(rect, 0.0, mix(t.background, BLUE, 0.08));
        let header = &self.diff.files[f].hunks[h].header;
        let text = painter.text(
            pos2(x, y),
            Align2::LEFT_CENTER,
            header,
            self.font.clone(),
            t.muted_foreground,
        );
        if !self.selectable {
            return;
        }
        let label = if self.staged {
            "Unstage ханк"
        } else {
            "Stage ханк"
        };
        let (clicked, right) = link(
            ui,
            painter,
            pos2(text.right() + 16.0, y),
            label,
            (f, h, "apply"),
            t,
        );
        if clicked {
            actions.push(Action::ApplyHunk { file: f, hunk: h });
        }
        if !self.staged {
            let (clicked, _) = link(
                ui,
                painter,
                pos2(right + 8.0, y),
                "Отменить ханк",
                (f, h, "discard"),
                t,
            );
            if clicked {
                actions.push(Action::DiscardHunk { file: f, hunk: h });
            }
        }
    }
}

fn select(ui: &Ui, (file, hunk, line): (usize, usize, usize), actions: &mut Vec<Action>) {
    let m = ui.input(|i| i.modifiers);
    actions.push(Action::SelectDiffLine {
        file,
        hunk,
        line,
        extend: m.shift,
        toggle: m.command,
    });
}

fn rows(ui: &mut Ui, repo: &RepoState, diff: &Diff, side_by_side: bool, actions: &mut Vec<Action>) {
    let p = Paint {
        repo,
        diff,
        theme: Theme::get(ui.ctx()),
        font: FontId::monospace(FONT_SIZE),
        selectable: repo.can_select_lines(),
        staged: matches!(repo.diff_target, Some(DiffTarget::Change { staged: true, .. })),
    };
    let char_width = ui.fonts_mut(|f| f.glyph_width(&p.font, 'M'));
    let code = char_width * diff.max_line_len as f32 + 16.0;
    let count = if side_by_side {
        diff.split_rows.len()
    } else {
        diff.rows.len()
    };
    // В side-by-side у каждой половины свой номер строки.
    let content_width = if side_by_side {
        2.0 * (NUMBER_WIDTH + MARKER_WIDTH + code)
    } else {
        NUMBER_WIDTH * 2.0 + MARKER_WIDTH + code
    };

    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::both()
        .id_salt("diff")
        .auto_shrink([false, false])
        .show_rows(ui, ROW_HEIGHT, count, |ui, range| {
            let width = ui.available_width().max(content_width);
            for i in range {
                if side_by_side {
                    split_row(ui, &p, diff.split_rows[i], width, actions);
                } else {
                    unified_row(ui, &p, diff.rows[i], width, actions);
                }
            }
        });
}

fn unified_row(ui: &mut Ui, p: &Paint, row: DiffRow, width: f32, actions: &mut Vec<Action>) {
    let clickable = matches!(row, DiffRow::Line(f, h, l) if p.selectable && p.is_change(f, h, l));
    let sense = if clickable { Sense::click() } else { Sense::hover() };
    let (rect, mut response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), sense);
    if clickable {
        response = response.on_hover_text_at_pointer(SELECT_HINT);
    }
    if let (true, DiffRow::Line(f, h, l)) = (response.clicked(), row) {
        select(ui, (f, h, l), actions);
    }
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter_at(rect);
    match row {
        DiffRow::File(f) => file_row(&painter, rect, &p.diff.files[f], &p.theme),
        DiffRow::Hunk(f, h) => {
            let x = rect.left() + NUMBER_WIDTH * 2.0 + MARKER_WIDTH;
            p.hunk_row(ui, &painter, rect, (f, h), x, actions);
        }
        DiffRow::Line(f, h, l) => {
            let line = &p.diff.files[f].hunks[h].lines[l];
            p.line_cell(
                &painter,
                rect,
                (f, h, l),
                &[line.old_no, line.new_no],
                response.hovered(),
            );
        }
    }
}

fn split_row(ui: &mut Ui, p: &Paint, row: SplitRow, width: f32, actions: &mut Vec<Action>) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter_at(rect);
    match row {
        SplitRow::File(f) => file_row(&painter, rect, &p.diff.files[f], &p.theme),
        SplitRow::Hunk(f, h) => {
            let x = rect.left() + NUMBER_WIDTH + MARKER_WIDTH;
            p.hunk_row(ui, &painter, rect, (f, h), x, actions);
        }
        SplitRow::Pair(f, h, left, right) => {
            let half = rect.width() / 2.0;
            let cells = [
                (Rect::from_min_size(rect.min, vec2(half, ROW_HEIGHT)), left, true),
                (
                    Rect::from_min_size(pos2(rect.left() + half, rect.top()), vec2(half, ROW_HEIGHT)),
                    right,
                    false,
                ),
            ];
            for (cell, line, old_side) in cells {
                let painter = painter.with_clip_rect(cell);
                let Some(l) = line else {
                    // Пустая половина: строки на этой стороне нет.
                    painter.rect_filled(cell, 0.0, p.theme.muted);
                    continue;
                };
                let id = ui.id().with(("split", f, h, l, old_side));
                let clickable = p.selectable && p.is_change(f, h, l);
                let sense = if clickable { Sense::click() } else { Sense::hover() };
                let mut response = ui.interact(cell, id, sense);
                if clickable {
                    response = response.on_hover_text_at_pointer(SELECT_HINT);
                }
                if response.clicked() {
                    select(ui, (f, h, l), actions);
                }
                let dl = &p.diff.files[f].hunks[h].lines[l];
                let number = if old_side { dl.old_no } else { dl.new_no };
                p.line_cell(&painter, cell, (f, h, l), &[number], response.hovered());
            }
            let x = rect.left() + half;
            painter.line_segment(
                [pos2(x, rect.top()), pos2(x, rect.bottom())],
                egui::Stroke::new(1.0, p.theme.border),
            );
        }
    }
}

/// Текстовая кнопка внутри строки, нарисованной вручную. Возвращает `true` при клике.
fn link(
    ui: &Ui,
    painter: &egui::Painter,
    at: egui::Pos2,
    text: &str,
    id: (usize, usize, &str),
    theme: &Theme,
) -> (bool, f32) {
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
    (response.clicked(), rect.right())
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
