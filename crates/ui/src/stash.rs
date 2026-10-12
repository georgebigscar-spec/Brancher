//! Stash: список записей под тегами, их контекстное меню и диалог «Спрятать изменения».

use brancher_core::{Action, DiffTarget, RepoState, Stash};
use egui::{Context, Id, RichText, Ui};

/// Открытый диалог; живёт в памяти egui, как и диалог новой ветки.
#[derive(Clone, Default)]
struct StashDialog {
    message: String,
    untracked: bool,
    focused: bool,
}

fn dialog_id() -> Id {
    Id::new("stash-dialog")
}

pub fn open_dialog(ctx: &Context) {
    let d = StashDialog {
        untracked: true,
        ..Default::default()
    };
    ctx.data_mut(|m| m.insert_temp(dialog_id(), Some(d)));
}

/// `On main: текст` → `текст`; `WIP on main: abc тема` остаётся как есть.
fn title(st: &Stash) -> &str {
    match st.message.strip_prefix("On ") {
        Some(rest) => rest.split_once(": ").map_or(st.message.as_str(), |(_, m)| m),
        None => &st.message,
    }
}

pub fn section(ui: &mut Ui, repo: &RepoState, actions: &mut Vec<Action>) {
    ui.label(RichText::new("Stash").strong());
    if repo.stashes.is_empty() {
        widgets::muted(ui, "Нет записей");
        return;
    }
    for st in &repo.stashes {
        let shown = matches!(&repo.diff_target, Some(DiffTarget::Commit { id, .. }) if *id == st.id);
        let text = RichText::new(title(st));
        let text = if shown { text.strong() } else { text };
        let response = ui
            .add(egui::Label::new(text).sense(egui::Sense::click()).truncate())
            .on_hover_text(format!("stash@{{{}}}: {}", st.index, st.message));
        if response.clicked() {
            actions.push(Action::SelectStash(st.index));
        }
        widgets::context_menu(&response, 220.0, |ui| {
            if widgets::menu_item(ui, "Применить", None).clicked() {
                actions.push(Action::ApplyStash(st.index));
            }
            if widgets::menu_item(ui, "Применить и удалить", None).clicked() {
                actions.push(Action::PopStash(st.index));
            }
            widgets::menu_separator(ui);
            if widgets::menu_item(ui, "Удалить…", None).clicked() {
                actions.push(Action::DropStash(st.index));
            }
        });
    }
}

pub fn dialog(ctx: &Context, actions: &mut Vec<Action>) {
    let Some(mut d) = ctx
        .data(|m| m.get_temp::<Option<StashDialog>>(dialog_id()))
        .flatten()
    else {
        return;
    };
    let resp = widgets::Dialog::new("stash-dialog-window", "Спрятать изменения")
        .description("Рабочая копия вернётся к последнему коммиту, изменения сохранятся в stash.")
        .show(ctx, |ui| {
            let input = ui.add(
                widgets::Input::new(&mut d.message)
                    .placeholder("Сообщение (необязательно)")
                    .width(ui.available_width()),
            );
            if !d.focused {
                input.request_focus();
                d.focused = true;
            }
            ui.add_space(6.0);
            ui.add(widgets::Checkbox::new(&mut d.untracked).label("Включая неотслеживаемые файлы"));
            widgets::footer(ui, |ui| {
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                let ok = ui.add(widgets::Button::new("Спрятать")).clicked() || enter;
                let cancel = ui.add(widgets::Button::outline("Отмена")).clicked();
                (ok, cancel)
            })
        });
    let (ok, cancel) = resp.inner;
    if ok {
        actions.push(Action::Stash {
            message: d.message.trim().to_owned(),
            untracked: d.untracked,
        });
    }
    let open = !(ok || cancel || resp.should_close);
    ctx.data_mut(|m| m.insert_temp(dialog_id(), open.then_some(d)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stash_titles() {
        let st = |m: &str| Stash {
            message: m.into(),
            ..Default::default()
        };
        assert_eq!(title(&st("On main: правка: вторая")), "правка: вторая");
        assert_eq!(title(&st("WIP on main: abc тема")), "WIP on main: abc тема");
        assert_eq!(title(&st("On main")), "On main");
    }
}
