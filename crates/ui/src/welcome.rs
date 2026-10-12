//! Стартовая страница (REPO-01, REPO-02). Группы, drag-and-drop и клонирование — дальше по плану.

use brancher_core::{Action, AppState};
use egui::Ui;

pub fn show(ui: &mut Ui, state: &AppState, open_path: &mut String, actions: &mut Vec<Action>) {
    ui.add_space(24.0);
    widgets::h2(ui, "Репозитории");
    widgets::muted(
        ui,
        "Откройте папку с Git-репозиторием. Диалог выбора папки и клонирование появятся в MVP.",
    );
    ui.add_space(12.0);

    ui.horizontal(|ui| {
        let input = ui.add(
            widgets::Input::new(open_path)
                .placeholder("/путь/к/репозиторию")
                .width(420.0),
        );
        let submit = input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.add(widgets::Button::new("Открыть")).clicked() || submit) && !open_path.trim().is_empty()
        {
            actions.push(Action::OpenRepo(open_path.trim().into()));
            open_path.clear();
        }
    });

    if !state.recent.is_empty() {
        ui.add_space(20.0);
        widgets::h3(ui, "Недавние");
        for entry in &state.recent {
            let r = ui
                .add(egui::Button::new(&entry.name).frame(false))
                .on_hover_text(entry.path.display().to_string());
            if r.clicked() {
                actions.push(Action::OpenRepo(entry.path.clone()));
            }
        }
    }
}
