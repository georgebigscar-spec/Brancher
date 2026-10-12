//! Вкладка репозитория: ветки слева, история в центре, изменения и коммит справа.
//! Граф коммитов рисует [`crate::graph`]; diff и blame — следующие шаги MVP.

use brancher_core::{Action, Change, Commit, DiffTarget, RefKind, RepoState};
use egui::{RichText, Ui};

const ROW_HEIGHT: f32 = 24.0;

pub fn show(ui: &mut Ui, repo: &RepoState, actions: &mut Vec<Action>) {
    if let Some(c) = &repo.confirm {
        let answer = widgets::confirm_dialog(
            ui.ctx(),
            "confirm",
            &c.title,
            &c.description,
            &c.action,
            "Отмена",
            true,
        );
        if let Some(yes) = answer {
            actions.push(Action::Confirm(yes));
        }
    }
    toolbar(ui, repo, actions);
    ui.separator();
    let height = ui.available_height();
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(220.0);
            ui.set_min_height(height);
            refs(ui, repo);
        });
        ui.separator();
        let side = 340.0;
        let center = (ui.available_width() - side - 16.0).max(200.0);
        ui.vertical(|ui| {
            ui.set_width(center);
            if repo.diff_target.is_some() {
                // История сверху, diff снизу.
                let history_height = (height * 0.4).max(120.0);
                ui.allocate_ui(egui::vec2(center, history_height), |ui| {
                    ui.set_min_height(history_height);
                    history(ui, repo, actions);
                });
                ui.separator();
                crate::diff_view::show(ui, repo, actions);
            } else {
                history(ui, repo, actions);
            }
        });
        ui.separator();
        ui.vertical(|ui| {
            ui.set_width(side);
            changes(ui, repo, actions);
        });
    });
}

fn toolbar(ui: &mut Ui, repo: &RepoState, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        widgets::h3(ui, &repo.name);
        if let Some(s) = &repo.status {
            let branch = s.branch.clone().unwrap_or_else(|| "detached HEAD".into());
            ui.add(widgets::Badge::new(branch).secondary());
            if s.ahead > 0 || s.behind > 0 {
                ui.add(widgets::Badge::new(format!("↑{} ↓{}", s.ahead, s.behind)).outline());
            }
        }
        if repo.busy > 0 {
            ui.spinner();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(widgets::Button::outline("Обновить"))
                .on_hover_text("F5")
                .clicked()
            {
                actions.push(Action::Refresh);
            }
        });
    });
}

fn refs(ui: &mut Ui, repo: &RepoState) {
    egui::ScrollArea::vertical().id_salt("refs").show(ui, |ui| {
        for (title, kind) in [
            ("Ветки", RefKind::LocalBranch),
            ("Remotes", RefKind::RemoteBranch),
            ("Теги", RefKind::Tag),
        ] {
            ui.label(RichText::new(title).strong());
            for r in repo.refs.iter().filter(|r| r.kind == kind) {
                let mut text = r.name.clone();
                if r.ahead > 0 || r.behind > 0 {
                    text.push_str(&format!("  ↑{} ↓{}", r.ahead, r.behind));
                }
                let label = if r.is_head {
                    RichText::new(text).strong()
                } else {
                    RichText::new(text)
                };
                ui.label(label);
            }
            ui.add_space(8.0);
        }
    });
}

/// Виртуальная прокрутка: рисуются только видимые строки (NFR-03).
fn history(ui: &mut Ui, repo: &RepoState, actions: &mut Vec<Action>) {
    if repo.history.is_empty() {
        widgets::muted(ui, "Коммитов пока нет.");
        return;
    }
    let graph_width = crate::graph::column_width(&repo.graph);
    let theme = widgets::Theme::get(ui.ctx());
    // Строки рисуются вручную ровно по ROW_HEIGHT: так show_rows знает точную высоту,
    // а линии графа соседних строк смыкаются без зазоров.
    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::vertical()
        .id_salt("history")
        .auto_shrink([false, false])
        .show_rows(ui, ROW_HEIGHT, repo.history.len(), |ui, range| {
            for i in range {
                let c = &repo.history[i];
                let selected = repo.selected_commit.as_deref() == Some(c.id.as_str());
                let size = egui::vec2(ui.available_width(), ROW_HEIGHT);
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
                if response.clicked() {
                    actions.push(Action::SelectCommit(Some(c.id.clone())));
                }
                if !ui.is_rect_visible(rect) {
                    continue;
                }
                let painter = ui.painter_at(rect);
                if selected || response.hovered() {
                    painter.rect_filled(rect, 0.0, theme.accent);
                }
                if let Some(row) = repo.graph.rows.get(i) {
                    let graph = egui::Rect::from_min_size(rect.min, egui::vec2(graph_width, ROW_HEIGHT));
                    crate::graph::paint_row(ui, graph, row, c.is_merge());
                }
                history_text(&painter, rect, graph_width, c, &theme);
            }
        });
}

/// Короткий SHA, заголовок и автор в одной строке истории.
fn history_text(painter: &egui::Painter, rect: egui::Rect, graph_width: f32, c: &Commit, t: &widgets::Theme) {
    let y = rect.center().y;
    let mono = egui::FontId::monospace(13.0);
    let body = egui::FontId::proportional(14.0);
    let author = painter.layout_no_wrap(c.author.name.clone(), body.clone(), t.muted_foreground);
    let author_x = rect.right() - 8.0 - author.size().x;
    painter.galley(
        egui::pos2(author_x, y - author.size().y / 2.0),
        author,
        t.muted_foreground,
    );

    let id_x = rect.left() + graph_width + 4.0;
    let id = painter.text(
        egui::pos2(id_x, y),
        egui::Align2::LEFT_CENTER,
        c.short_id(),
        mono,
        t.muted_foreground,
    );
    let subject_x = id.right() + 12.0;
    let subject_clip = egui::Rect::from_x_y_ranges(subject_x..=author_x - 12.0, rect.y_range());
    painter.with_clip_rect(subject_clip).text(
        egui::pos2(subject_x, y),
        egui::Align2::LEFT_CENTER,
        &c.subject,
        body,
        t.foreground,
    );
}

fn changes(ui: &mut Ui, repo: &RepoState, actions: &mut Vec<Action>) {
    let Some(status) = &repo.status else {
        widgets::muted(ui, "Загрузка…");
        return;
    };
    let unstaged: Vec<&Change> = status.unstaged().collect();
    let staged: Vec<&Change> = status.staged().collect();

    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("Неиндексированные ({})", unstaged.len())).strong());
        if !unstaged.is_empty() && ui.small_button("Stage all").clicked() {
            actions.push(Action::StageAll);
        }
    });
    file_list(ui, repo, "unstaged", &unstaged, false, actions);

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("Индексированные ({})", staged.len())).strong());
        if !staged.is_empty() && ui.small_button("Unstage all").clicked() {
            actions.push(Action::UnstageAll);
        }
    });
    file_list(ui, repo, "staged", &staged, true, actions);

    ui.add_space(8.0);
    let mut message = repo.commit_message.clone();
    if ui
        .add(
            widgets::Textarea::new(&mut message)
                .placeholder("Сообщение коммита")
                .rows(4),
        )
        .changed()
    {
        actions.push(Action::SetCommitMessage(message));
    }
    ui.horizontal(|ui| {
        let mut amend = repo.amend;
        if ui
            .add(widgets::Checkbox::new(&mut amend).label("Amend"))
            .changed()
        {
            actions.push(Action::SetAmend(amend));
        }
        let can_commit = !repo.commit_message.trim().is_empty() && (!staged.is_empty() || repo.amend);
        let commit = ui.add_enabled(can_commit, widgets::Button::new("Commit"));
        let ctrl_enter = ui.input_mut(|i| {
            i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::Enter,
            ))
        });
        if can_commit && (commit.clicked() || ctrl_enter) {
            actions.push(Action::Commit);
        }
    });
}

fn file_list(
    ui: &mut Ui,
    repo: &RepoState,
    id: &str,
    files: &[&Change],
    staged: bool,
    actions: &mut Vec<Action>,
) {
    let theme = widgets::Theme::get(ui.ctx());
    egui::ScrollArea::vertical()
        .id_salt(id)
        .max_height(180.0)
        .show(ui, |ui| {
            for c in files {
                let kind = if staged { c.staged } else { c.unstaged };
                let letter = kind.map(|k| k.letter()).unwrap_or(' ');
                let selected = matches!(
                    &repo.diff_target,
                    Some(DiffTarget::Change { path, staged: s, .. }) if path == &c.path && *s == staged
                );
                // Фон рисуется под строкой, поэтому место под него резервируется заранее.
                let background = ui.painter().add(egui::Shape::Noop);
                let row = ui.horizontal(|ui| {
                    ui.monospace(letter.to_string());
                    let r = ui.add(egui::Label::new(&c.path).truncate().sense(egui::Sense::click()));
                    if r.double_clicked() {
                        let paths = vec![c.path.clone()];
                        actions.push(if staged {
                            Action::Unstage(paths)
                        } else {
                            Action::Stage(paths)
                        });
                    }
                    if r.clicked() {
                        actions.push(Action::SelectChange {
                            path: c.path.clone(),
                            staged,
                        });
                    }
                    r.on_hover_text(if staged {
                        "Клик — diff, двойной клик — unstage"
                    } else {
                        "Клик — diff, двойной клик — stage"
                    });
                });
                if selected {
                    let rect = row.response.rect.expand2(egui::vec2(4.0, 0.0));
                    ui.painter().set(
                        background,
                        egui::Shape::rect_filled(rect, theme.radius_sm(), theme.accent),
                    );
                }
            }
        });
}
