//! Диалог «Новый тег»: имя и необязательное сообщение (с ним тег аннотированный).

use brancher_core::{tag_name_error, Action};
use egui::{Context, Id, RichText};

#[derive(Clone, Default)]
struct NewTag {
    target: String,
    /// Что показать в описании: коммит или ветка.
    from: String,
    name: String,
    message: String,
    focused: bool,
}

fn dialog_id() -> Id {
    Id::new("new-tag")
}

pub fn open_new_tag(ctx: &Context, target: String, from: String) {
    let d = NewTag {
        target,
        from,
        ..Default::default()
    };
    ctx.data_mut(|m| m.insert_temp(dialog_id(), Some(d)));
}

pub fn new_tag_dialog(ctx: &Context, actions: &mut Vec<Action>) {
    let Some(mut d) = ctx.data(|m| m.get_temp::<Option<NewTag>>(dialog_id())).flatten() else {
        return;
    };
    let error = tag_name_error(d.name.trim());
    let resp = widgets::Dialog::new("new-tag-dialog", "Новый тег")
        .description(format!("На {}", d.from))
        .show(ctx, |ui| {
            let input = ui.add(
                widgets::Input::new(&mut d.name)
                    .placeholder("v1.0.0")
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
            ui.add(
                widgets::Textarea::new(&mut d.message)
                    .placeholder("Сообщение (необязательно: с ним тег будет аннотированным)")
                    .rows(3),
            );
            widgets::footer(ui, |ui| {
                // Enter в многострочном поле — перевод строки, поэтому создаёт только кнопка
                // или Enter в поле имени.
                let enter = input.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let create = ui
                    .add_enabled(error.is_none(), widgets::Button::new("Создать"))
                    .clicked();
                let cancel = ui.add(widgets::Button::outline("Отмена")).clicked();
                (create || (enter && error.is_none()), cancel)
            })
        });
    let (create, cancel) = resp.inner;
    if create {
        actions.push(Action::CreateTag {
            name: d.name.trim().to_owned(),
            target: d.target.clone(),
            message: d.message.trim().to_owned(),
        });
    }
    let open = !(create || cancel || resp.should_close);
    ctx.data_mut(|m| m.insert_temp(dialog_id(), open.then_some(d)));
}
