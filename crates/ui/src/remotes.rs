//! Remotes слева: remote со своими ветками, его меню и диалог «Добавить / изменить remote».

use brancher_core::{remote_name_error, Action, GitRef, RefKind, Remote, RepoState};
use egui::{Context, Id, RichText, Ui};

/// Открытый диалог; `editing` — имя изменяемого remote, `None` — новый.
#[derive(Clone, Default)]
struct RemoteForm {
    editing: Option<String>,
    name: String,
    url: String,
    focused: bool,
}

fn dialog_id() -> Id {
    Id::new("remote-form")
}

fn open_form(ctx: &Context, form: RemoteForm) {
    ctx.data_mut(|m| m.insert_temp(dialog_id(), Some(form)));
}

/// Секция «Remotes»: заголовок с кнопкой добавления, под каждым remote — его ветки.
pub fn section(
    ui: &mut Ui,
    repo: &RepoState,
    row: &mut dyn FnMut(&mut Ui, &GitRef),
    actions: &mut Vec<Action>,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Remotes").strong());
        if ui.small_button("+").on_hover_text("Добавить remote").clicked() {
            open_form(ui.ctx(), RemoteForm::default());
        }
    });
    let branches = repo.refs.iter().filter(|r| r.kind == RefKind::RemoteBranch);
    if repo.remotes.is_empty() {
        // Список ещё не загружен или remotes нет: ветки (если есть) — плоским списком.
        for r in branches {
            row(ui, r);
        }
        return;
    }
    for remote in &repo.remotes {
        remote_row(ui, remote, actions);
        let own: Vec<&GitRef> = branches
            .clone()
            .filter(|r| owner(&repo.remotes, &r.name) == Some(remote))
            .collect();
        if !own.is_empty() {
            ui.indent(("remote", &remote.name), |ui| {
                for r in own {
                    row(ui, r);
                }
            });
        }
    }
}

/// Remote ветки `origin/x`: самый длинный совпавший префикс (имена remotes могут содержать `/`).
fn owner<'a>(remotes: &'a [Remote], branch: &str) -> Option<&'a Remote> {
    remotes
        .iter()
        .filter(|r| {
            branch
                .strip_prefix(r.name.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
        })
        .max_by_key(|r| r.name.len())
}

fn remote_row(ui: &mut Ui, remote: &Remote, actions: &mut Vec<Action>) {
    let label =
        RichText::new(format!("☁ {}", remote.name)).color(widgets::Theme::get(ui.ctx()).muted_foreground);
    let response = ui.add(egui::Label::new(label).sense(egui::Sense::click()).truncate());
    let hover = if remote.push_url != remote.url && !remote.push_url.is_empty() {
        format!("{}\npush: {}", remote.url, remote.push_url)
    } else {
        remote.url.clone()
    };
    let response = response.on_hover_text(hover);
    widgets::context_menu(&response, 220.0, |ui| {
        if widgets::menu_item(ui, "Fetch", None).clicked() {
            actions.push(Action::FetchRemote(remote.name.clone()));
        }
        if widgets::menu_item(ui, "Изменить…", None).clicked() {
            open_form(
                ui.ctx(),
                RemoteForm {
                    editing: Some(remote.name.clone()),
                    name: remote.name.clone(),
                    url: remote.url.clone(),
                    focused: false,
                },
            );
        }
        if widgets::menu_item(ui, "Копировать URL", None).clicked() {
            ui.ctx().copy_text(remote.url.clone());
        }
        widgets::menu_separator(ui);
        if widgets::menu_item(ui, "Удалить…", None).clicked() {
            actions.push(Action::RemoveRemote(remote.name.clone()));
        }
    });
}

pub fn dialog(ctx: &Context, repo: &RepoState, actions: &mut Vec<Action>) {
    let Some(mut d) = ctx
        .data(|m| m.get_temp::<Option<RemoteForm>>(dialog_id()))
        .flatten()
    else {
        return;
    };
    let name = d.name.trim();
    let taken = d.editing.as_deref() != Some(name) && repo.remotes.iter().any(|r| r.name == name);
    let name_error = remote_name_error(name).or(taken.then_some("Remote с таким именем уже есть"));
    let error = name_error.or(d.url.trim().is_empty().then_some("Введите URL"));
    let (title, button) = match &d.editing {
        Some(_) => ("Изменить remote", "Сохранить"),
        None => ("Новый remote", "Добавить"),
    };
    let resp = widgets::Dialog::new("remote-form-dialog", title)
        .description("Имя и адрес репозитория: https://…, git@host:путь или путь к папке.")
        .show(ctx, |ui| {
            let width = ui.available_width();
            let input = ui.add(
                widgets::Input::new(&mut d.name)
                    .placeholder("origin")
                    .width(width),
            );
            if !d.focused {
                input.request_focus();
                d.focused = true;
            }
            if let (Some(e), false) = (name_error, d.name.is_empty()) {
                ui.label(RichText::new(e).color(widgets::Theme::get(ui.ctx()).destructive));
            }
            ui.add_space(6.0);
            ui.add(
                widgets::Input::new(&mut d.url)
                    .placeholder("https://github.com/user/repo.git")
                    .width(width),
            );
            widgets::footer(ui, |ui| {
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                let ok = ui
                    .add_enabled(error.is_none(), widgets::Button::new(button))
                    .clicked();
                let cancel = ui.add(widgets::Button::outline("Отмена")).clicked();
                (ok || (enter && error.is_none()), cancel)
            })
        });
    let (ok, cancel) = resp.inner;
    if ok {
        let (name, url) = (d.name.trim().to_owned(), d.url.trim().to_owned());
        actions.push(match &d.editing {
            Some(old) => Action::EditRemote {
                name: old.clone(),
                new_name: name,
                url,
            },
            None => Action::AddRemote { name, url },
        });
    }
    let open = !(ok || cancel || resp.should_close);
    ctx.data_mut(|m| m.insert_temp(dialog_id(), open.then_some(d)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_belongs_to_longest_matching_remote() {
        let r = |n: &str| Remote {
            name: n.into(),
            ..Default::default()
        };
        let remotes = [r("origin"), r("team"), r("team/qa")];
        assert_eq!(owner(&remotes, "origin/main").unwrap().name, "origin");
        assert_eq!(owner(&remotes, "team/qa/x").unwrap().name, "team/qa");
        assert_eq!(owner(&remotes, "team/dev").unwrap().name, "team");
        assert!(owner(&remotes, "originx/main").is_none());
    }
}
