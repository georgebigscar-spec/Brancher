//! Вкладка репозитория: ветки слева, история в центре, изменения и коммит справа.
//! Граф коммитов, diff и blame — отдельные виджеты этого крейта, следующие шаги MVP.

use egui::{RichText, Ui};
use brancher_core::{Action, Change, RefKind, RepoState};

const ROW_HEIGHT: f32 = 24.0;

pub fn show(ui: &mut Ui, repo: &RepoState, actions: &mut Vec<Action>) {
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
            history(ui, repo, actions);
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
    egui::ScrollArea::vertical()
        .id_salt("history")
        .auto_shrink([false, false])
        .show_rows(ui, ROW_HEIGHT, repo.history.len(), |ui, range| {
            for c in &repo.history[range] {
                let selected = repo.selected_commit.as_deref() == Some(c.id.as_str());
                ui.horizontal(|ui| {
                    ui.set_height(ROW_HEIGHT);
                    ui.monospace(c.short_id());
                    if ui.selectable_label(selected, &c.subject).clicked() {
                        actions.push(Action::SelectCommit(Some(c.id.clone())));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        widgets::muted(ui, &c.author.name);
                    });
                });
            }
        });
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
    file_list(ui, "unstaged", &unstaged, false, actions);

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("Индексированные ({})", staged.len())).strong());
        if !staged.is_empty() && ui.small_button("Unstage all").clicked() {
            actions.push(Action::UnstageAll);
        }
    });
    file_list(ui, "staged", &staged, true, actions);

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

fn file_list(ui: &mut Ui, id: &str, files: &[&Change], staged: bool, actions: &mut Vec<Action>) {
    egui::ScrollArea::vertical()
        .id_salt(id)
        .max_height(180.0)
        .show(ui, |ui| {
            for c in files {
                let kind = if staged { c.staged } else { c.unstaged };
                let letter = kind.map(|k| k.letter()).unwrap_or(' ');
                ui.horizontal(|ui| {
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
                    r.on_hover_text(if staged {
                        "Двойной клик — unstage"
                    } else {
                        "Двойной клик — stage"
                    });
                });
            }
        });
}
