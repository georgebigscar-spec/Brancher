//! Ветки, remotes, теги и stash слева (remotes — в [`crate::remotes`]), их контекстные меню и диалог новой ветки.
//! Двойной клик по ветке — переключиться на неё.

use brancher_core::{branch_name_error, Action, Commit, GitRef, RefKind, RepoState};
use egui::{Context, Id, RichText, Ui};

/// Открытый диалог «Новая ветка»; живёт в памяти egui, а не в модели: это ввод, а не данные.
#[derive(Clone, Default)]
struct NewBranch {
    /// Коммит или ссылка, от которой создаём.
    start: String,
    /// Что показать в описании: имя ветки или коммит.
    from: String,
    name: String,
    checkout: bool,
    /// Фокус в поле имени ставится один раз, при открытии.
    focused: bool,
}

fn dialog_id() -> Id {
    Id::new("new-branch")
}

fn open_new_branch(ctx: &Context, start: String, from: String) {
    let d = NewBranch {
        start,
        from,
        name: String::new(),
        checkout: true,
        focused: false,
    };
    ctx.data_mut(|m| m.insert_temp(dialog_id(), Some(d)));
}

pub fn refs(ui: &mut Ui, repo: &RepoState, actions: &mut Vec<Action>) {
    egui::ScrollArea::vertical().id_salt("refs").show(ui, |ui| {
        let current = repo.status.as_ref().and_then(|s| s.branch.as_deref());
        for (title, kind) in [
            ("Ветки", RefKind::LocalBranch),
            ("Remotes", RefKind::RemoteBranch),
            ("Теги", RefKind::Tag),
        ] {
            if kind == RefKind::RemoteBranch {
                let mut row_actions = vec![];
                let mut row = |ui: &mut Ui, r: &GitRef| ref_row(ui, r, current, &mut row_actions);
                crate::remotes::section(ui, repo, &mut row, actions);
                actions.extend(row_actions);
            } else {
                ui.label(RichText::new(title).strong());
                for r in repo.refs.iter().filter(|r| r.kind == kind) {
                    ref_row(ui, r, current, actions);
                }
            }
            ui.add_space(8.0);
        }
        crate::stash::section(ui, repo, actions);
    });
}

fn ref_row(ui: &mut Ui, r: &GitRef, current: Option<&str>, actions: &mut Vec<Action>) {
    let mut text = r.name.clone();
    if r.ahead > 0 || r.behind > 0 {
        text.push_str(&format!("  ↑{} ↓{}", r.ahead, r.behind));
    }
    let label = if r.is_head {
        RichText::new(text).strong()
    } else {
        RichText::new(text)
    };
    let response = ui.add(egui::Label::new(label).sense(egui::Sense::click()).truncate());
    let switch = match r.kind {
        RefKind::LocalBranch if !r.is_head => Some(Action::SwitchBranch(r.name.clone())),
        RefKind::RemoteBranch => Some(Action::CheckoutRemote(r.name.clone())),
        _ => None,
    };
    let response = match r.kind {
        RefKind::LocalBranch if r.is_head => response.on_hover_text("Текущая ветка"),
        RefKind::Tag => response,
        _ => response.on_hover_text("Двойной клик — переключиться"),
    };
    if response.double_clicked() {
        actions.extend(switch.clone());
    }
    widgets::context_menu(&response, 220.0, |ui| {
        if let Some(a) = &switch {
            if widgets::menu_item(ui, "Переключиться", None).clicked() {
                actions.push(a.clone());
            }
        }
        if widgets::menu_item(ui, "Создать ветку отсюда…", None).clicked() {
            open_new_branch(ui.ctx(), r.full_name.clone(), r.name.clone());
        }
        if let (Some(current), false) = (current, r.is_head) {
            widgets::menu_separator(ui);
            if widgets::menu_item(ui, &format!("Слить в {current}"), None).clicked() {
                actions.push(Action::Merge(r.name.clone()));
            }
            if widgets::menu_item(ui, &format!("Перебазировать {current} на {}…", r.name), None).clicked()
            {
                actions.push(Action::Rebase(r.name.clone()));
            }
        }
        if r.kind == RefKind::LocalBranch && !r.is_head {
            widgets::menu_separator(ui);
            if widgets::menu_item(ui, "Удалить…", None).clicked() {
                actions.push(Action::DeleteBranch(r.name.clone()));
            }
        }
        if r.kind == RefKind::Tag {
            widgets::menu_separator(ui);
            if widgets::menu_item(ui, "Отправить в remote", None).clicked() {
                actions.push(Action::PushTag(r.name.clone()));
            }
            if widgets::menu_item(ui, "Удалить тег…", None).clicked() {
                actions.push(Action::DeleteTag(r.name.clone()));
            }
        }
    });
}

/// Контекстное меню строки истории; `current` — текущая ветка (для cherry-pick).
pub fn commit_menu(response: &egui::Response, c: &Commit, current: Option<&str>, actions: &mut Vec<Action>) {
    widgets::context_menu(response, 240.0, |ui| {
        let short: String = c.id.chars().take(10).collect();
        if widgets::menu_item(ui, "Создать ветку…", None).clicked() {
            open_new_branch(ui.ctx(), c.id.clone(), format!("{short} {}", c.subject));
        }
        if widgets::menu_item(ui, "Создать тег…", None).clicked() {
            crate::tags::open_new_tag(ui.ctx(), c.id.clone(), format!("{short} {}", c.subject));
        }
        widgets::menu_separator(ui);
        let pick = match current {
            Some(b) => format!("Cherry-pick в {b}"),
            None => "Cherry-pick".to_owned(),
        };
        if widgets::menu_item(ui, &pick, None).clicked() {
            actions.push(Action::CherryPick(c.id.clone()));
        }
        if widgets::menu_item(ui, "Revert", None).clicked() {
            actions.push(Action::Revert(c.id.clone()));
        }
        if current.is_some() && widgets::menu_item(ui, "Interactive rebase отсюда…", None).clicked() {
            actions.push(Action::PlanRebase(c.id.clone()));
        }
        widgets::menu_separator(ui);
        if widgets::menu_item(ui, "Копировать SHA", None).clicked() {
            ui.ctx().copy_text(c.id.clone());
        }
    });
}

pub fn new_branch_dialog(ctx: &Context, actions: &mut Vec<Action>) {
    let Some(mut d) = ctx
        .data(|m| m.get_temp::<Option<NewBranch>>(dialog_id()))
        .flatten()
    else {
        return;
    };
    let error = branch_name_error(d.name.trim());
    let resp = widgets::Dialog::new("new-branch-dialog", "Новая ветка")
        .description(format!("От {}", d.from))
        .show(ctx, |ui| {
            let input = ui.add(
                widgets::Input::new(&mut d.name)
                    .placeholder("feature/имя")
                    .width(ui.available_width()),
            );
            if !d.focused {
                input.request_focus();
                d.focused = true;
            }
            if let (Some(e), false) = (error, d.name.is_empty()) {
                ui.label(RichText::new(e).color(widgets::Theme::get(ui.ctx()).destructive));
            }
            ui.add_space(6.0);
            ui.add(widgets::Checkbox::new(&mut d.checkout).label("Переключиться на неё"));
            widgets::footer(ui, |ui| {
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                let create = ui
                    .add_enabled(error.is_none(), widgets::Button::new("Создать"))
                    .clicked();
                let cancel = ui.add(widgets::Button::outline("Отмена")).clicked();
                (create || (enter && error.is_none()), cancel)
            })
        });
    let (create, cancel) = resp.inner;
    if create {
        actions.push(Action::CreateBranch {
            name: d.name.trim().to_owned(),
            start: d.start.clone(),
            checkout: d.checkout,
        });
    }
    let open = !(create || cancel || resp.should_close);
    ctx.data_mut(|m| m.insert_temp(dialog_id(), open.then_some(d)));
}
