//! Вкладка репозитория: ветки слева, история в центре, изменения и коммит справа.
//! Граф коммитов рисует [`crate::graph`]; diff и blame — следующие шаги MVP.

use brancher_core::{
    Action, Change, ChangeKind, Commit, DiffTarget, Operation, RepoState, RepoStatus, Settings,
};
use egui::{RichText, Ui};

const ROW_HEIGHT: f32 = 24.0;

pub fn show(ui: &mut Ui, repo: &RepoState, settings: &Settings, actions: &mut Vec<Action>) {
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
    crate::branches::new_branch_dialog(ui.ctx(), actions);
    crate::tags::new_tag_dialog(ui.ctx(), actions);
    crate::stash::dialog(ui.ctx(), actions);
    crate::remotes::dialog(ui.ctx(), repo, actions);
    crate::oplog::dialog(ui.ctx(), repo, actions);
    if let Some(plan) = &repo.rebase_plan {
        crate::rebase_plan::dialog(ui.ctx(), plan, actions);
    }
    toolbar(ui, repo, actions);
    ui.separator();
    let height = ui.available_height();
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(220.0);
            ui.set_min_height(height);
            crate::branches::refs(ui, repo, actions);
        });
        ui.separator();
        let side = 340.0;
        let center = (ui.available_width() - side - 16.0).max(200.0);
        ui.vertical(|ui| {
            ui.set_width(center);
            if repo.diff_target.is_some() || repo.file_view.is_some() {
                // История сверху, diff снизу.
                let history_height = (height * 0.4).max(120.0);
                ui.allocate_ui(egui::vec2(center, history_height), |ui| {
                    ui.set_min_height(history_height);
                    history(ui, repo, actions);
                });
                ui.separator();
                match &repo.file_view {
                    Some(view) => crate::file_view::show(ui, repo, view, actions),
                    None => crate::diff_view::show(ui, repo, settings, actions),
                }
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
        if repo.busy > 0 && repo.progress.is_none() {
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
            if ui
                .add(widgets::Button::outline("История"))
                .on_hover_text("История операций: отменить commit, rebase, удаление ветки…")
                .clicked()
            {
                actions.push(Action::ShowOplog);
            }
            remote_buttons(ui, repo, actions);
        });
    });
}

/// Fetch, pull и push; во время сетевой задачи — её этап и «Отмена».
/// Макет справа налево: кнопки идут в обратном порядке.
fn remote_buttons(ui: &mut Ui, repo: &RepoState, actions: &mut Vec<Action>) {
    if let Some(p) = &repo.progress {
        if ui.add(widgets::Button::outline("Отмена")).clicked() {
            actions.push(Action::CancelJob);
        }
        if let Some(n) = p.percent {
            ui.add(widgets::Progress::new(f32::from(n) / 100.0).width(120.0));
        }
        let phase = crate::phase_name(&p.phase);
        let text = match p.percent {
            Some(n) => format!("{phase}: {n}%"),
            None => format!("{phase}…"),
        };
        ui.spinner();
        widgets::muted(ui, text);
        return;
    }
    let (ahead, behind) = repo.status.as_ref().map_or((0, 0), |s| (s.ahead, s.behind));
    let label = |name: &str, n: u32, arrow: &str| match n {
        0 => name.to_owned(),
        n => format!("{name} {arrow}{n}"),
    };
    if ui
        .add(widgets::Button::outline(label("Push", ahead, "↑")))
        .on_hover_text("Отправить текущую ветку; без upstream — на origin с -u")
        .clicked()
    {
        actions.push(Action::Push);
    }
    if ui
        .add(widgets::Button::outline(label("Pull", behind, "↓")))
        .on_hover_text("Забрать изменения текущей ветки (только перемотка вперёд)")
        .clicked()
    {
        actions.push(Action::Pull);
    }
    if ui
        .add(widgets::Button::outline("Fetch"))
        .on_hover_text("Получить изменения всех remotes")
        .clicked()
    {
        actions.push(Action::Fetch);
    }
    let dirty = repo.status.as_ref().is_some_and(|s| !s.changes.is_empty());
    if ui
        .add_enabled(dirty, widgets::Button::outline("Stash"))
        .on_hover_text("Спрятать изменения рабочей копии")
        .clicked()
    {
        crate::stash::open_dialog(ui.ctx());
    }
}

/// Виртуальная прокрутка: рисуются только видимые строки (NFR-03).
fn history(ui: &mut Ui, repo: &RepoState, actions: &mut Vec<Action>) {
    if repo.history.is_empty() {
        widgets::muted(ui, "Коммитов пока нет.");
        return;
    }
    search_bar(ui, repo, actions);
    // С фильтром соседние строки — не соседи в графе, поэтому граф не рисуется.
    let graph_width = match repo.history_filter {
        Some(_) => 0.0,
        None => crate::graph::column_width(&repo.graph),
    };
    let count = repo.history_filter.as_ref().map_or(repo.history.len(), Vec::len);
    let current = repo.status.as_ref().and_then(|s| s.branch.as_deref());
    if count == 0 {
        widgets::muted(ui, "Ничего не найдено.");
        return;
    }
    let theme = widgets::Theme::get(ui.ctx());
    // Строки рисуются вручную ровно по ROW_HEIGHT: так show_rows знает точную высоту,
    // а линии графа соседних строк смыкаются без зазоров.
    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::vertical()
        .id_salt("history")
        .auto_shrink([false, false])
        .show_rows(ui, ROW_HEIGHT, count, |ui, range| {
            for row in range {
                let i = repo.history_filter.as_ref().map_or(row, |f| f[row]);
                let c = &repo.history[i];
                let selected = repo.selected_commit.as_deref() == Some(c.id.as_str());
                let size = egui::vec2(ui.available_width(), ROW_HEIGHT);
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
                if response.clicked() {
                    actions.push(Action::SelectCommit(Some(c.id.clone())));
                }
                crate::branches::commit_menu(&response, c, current, actions);
                if !ui.is_rect_visible(rect) {
                    continue;
                }
                let painter = ui.painter_at(rect);
                if selected || response.hovered() {
                    painter.rect_filled(rect, 0.0, theme.accent);
                }
                if let (Some(row), None) = (repo.graph.rows.get(i), &repo.history_filter) {
                    let graph = egui::Rect::from_min_size(rect.min, egui::vec2(graph_width, ROW_HEIGHT));
                    crate::graph::paint_row(ui, graph, row, c.is_merge());
                }
                history_text(&painter, rect, graph_width, c, &theme);
            }
        });
}

/// Поиск по истории (Ctrl+F) и фильтр «только текущая ветка».
fn search_bar(ui: &mut Ui, repo: &RepoState, actions: &mut Vec<Action>) {
    let focus = ui.input_mut(|i| {
        i.consume_shortcut(&egui::KeyboardShortcut::new(
            egui::Modifiers::COMMAND,
            egui::Key::F,
        ))
    });
    ui.horizontal(|ui| {
        let mut head_only = repo.head_only;
        let mut query = repo.history_query.clone();
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(widgets::Checkbox::new(&mut head_only).label("Только текущая ветка"))
                .changed()
            {
                actions.push(Action::SetHeadOnly(head_only));
            }
            // Подпись есть всегда (иногда пустая): иначе у поля сменится авто-id и уйдёт фокус.
            let found = match &repo.history_filter {
                Some(f) => format!("{} из {}", f.len(), repo.history.len()),
                None => String::new(),
            };
            widgets::muted(ui, found);
            let input = ui.add(
                widgets::Input::new(&mut query)
                    .placeholder("Поиск: текст, автор или SHA (Ctrl+F)")
                    .width(ui.available_width()),
            );
            if focus {
                input.request_focus();
            }
            if input.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                query.clear();
                input.surrender_focus();
            }
            if query != repo.history_query {
                actions.push(Action::SetHistoryQuery(query));
            }
        });
    });
    ui.add_space(4.0);
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
    if let Some(op) = status.operation {
        operation_banner(ui, status, op, actions);
        ui.add_space(8.0);
    }

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

/// Идущий merge или rebase: сколько конфликтов осталось, «Продолжить» и «Прервать».
fn operation_banner(ui: &mut Ui, status: &RepoStatus, op: Operation, actions: &mut Vec<Action>) {
    let conflicts = status.conflicts().count();
    let mut title = format!("Идёт {}", op.name());
    if let Some((step, total)) = status.rebase_step {
        title.push_str(&format!(", шаг {step} из {total}"));
    }
    let details = match conflicts {
        0 => "Конфликты разрешены, можно продолжать.".to_owned(),
        n => format!("Конфликтов: {n}. Правый клик по файлу: взять одну из версий или отметить решённым."),
    };
    let mut alert = widgets::Alert::new(title).description(details);
    if conflicts > 0 {
        alert = alert.destructive();
    }
    alert.show(ui);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(conflicts == 0, widgets::Button::new("Продолжить"))
            .clicked()
        {
            actions.push(Action::ContinueOperation);
        }
        if ui.add(widgets::Button::outline("Прервать…")).clicked() {
            actions.push(Action::AbortOperation);
        }
    });
}

/// Меню файла: blame (в коммите `rev` или в рабочей копии) и история.
pub(crate) fn file_menu(response: &egui::Response, path: &str, rev: Option<&str>, actions: &mut Vec<Action>) {
    widgets::context_menu(response, 200.0, |ui| {
        if widgets::menu_item(ui, "Blame", None).clicked() {
            actions.push(Action::ShowBlame {
                path: path.to_owned(),
                rev: rev.map(str::to_owned),
            });
        }
        if widgets::menu_item(ui, "История файла", None).clicked() {
            actions.push(Action::ShowFileHistory(path.to_owned()));
        }
    });
}

/// Подписи сторон конфликта: в rebase «наша» — ветка, на которую перебазируем.
fn side_labels(op: Option<Operation>) -> (&'static str, &'static str) {
    match op {
        Some(Operation::Rebase) => ("Взять версию базы", "Взять версию своего коммита"),
        Some(Operation::Merge) => ("Взять версию текущей ветки", "Взять версию сливаемой ветки"),
        _ => ("Взять нашу версию", "Взять их версию"),
    }
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
                    let r = r.on_hover_text(if c.is_conflicted() {
                        "Конфликт: правый клик — выбрать версию, двойной клик — отметить решённым"
                    } else if staged {
                        "Клик — diff, двойной клик — unstage"
                    } else {
                        "Клик — diff, двойной клик — stage"
                    });
                    if !c.is_conflicted() && c.unstaged != Some(ChangeKind::Untracked) {
                        file_menu(&r, &c.path, None, actions);
                    }
                    if c.is_conflicted() {
                        let (ours, theirs) = side_labels(repo.operation());
                        widgets::context_menu(&r, 260.0, |ui| {
                            for (label, ours) in [(ours, true), (theirs, false)] {
                                if widgets::menu_item(ui, label, None).clicked() {
                                    actions.push(Action::ResolveConflict {
                                        path: c.path.clone(),
                                        ours,
                                    });
                                }
                            }
                            widgets::menu_separator(ui);
                            if widgets::menu_item(ui, "Отметить решённым", None).clicked() {
                                actions.push(Action::Stage(vec![c.path.clone()]));
                            }
                        });
                    }
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
