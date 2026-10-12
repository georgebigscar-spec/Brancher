//! Лента отмены: снимки перед изменяющими операциями, к любому можно вернуться.

use brancher_core::{Action, RepoState};
use egui::{Context, RichText};

pub fn dialog(ctx: &Context, repo: &RepoState, actions: &mut Vec<Action>) {
    let Some(list) = &repo.oplog else {
        return;
    };
    let theme = widgets::Theme::get(ctx);
    let resp = widgets::Dialog::new("oplog", "История операций")
        .description(
            "Перед каждой операцией Brancher запоминает ветки, теги, stash, HEAD и незакоммиченные \
             изменения. «Вернуть» откатывает репозиторий к состоянию перед операцией.",
        )
        .width(620.0)
        .show(ctx, |ui| {
            match list {
                None => {
                    ui.spinner();
                }
                Some(list) if list.is_empty() => {
                    widgets::muted(ui, "Пока пусто: снимки появятся после первой операции.");
                }
                Some(list) => {
                    egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                        for (i, s) in list.iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(crate::format_date_time(s.time))
                                        .monospace()
                                        .color(theme.muted_foreground),
                                );
                                ui.add(egui::Label::new(&s.title).truncate());
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let text = if i == 0 {
                                        "Отменить"
                                    } else {
                                        "Вернуть"
                                    };
                                    let hover = if i == 0 {
                                        "Отменить последнюю операцию".to_owned()
                                    } else {
                                        format!("Вернуться к состоянию перед «{}»", s.title)
                                    };
                                    let busy = repo.busy > 0 || repo.operation().is_some();
                                    if ui
                                        .add_enabled(!busy, widgets::Button::outline(text))
                                        .on_hover_text(hover)
                                        .on_disabled_hover_text(
                                            "Недоступно, пока идёт операция git или merge/rebase",
                                        )
                                        .clicked()
                                    {
                                        actions.push(Action::RestoreSnapshot(s.id.clone()));
                                    }
                                });
                            });
                        }
                    });
                }
            }
            widgets::footer(ui, |ui| ui.add(widgets::Button::outline("Закрыть")).clicked())
        });
    // Пока поверх открыт диалог подтверждения, лента не закрывается.
    if (resp.inner || resp.should_close) && repo.confirm.is_none() {
        actions.push(Action::CloseOplog);
    }
}
