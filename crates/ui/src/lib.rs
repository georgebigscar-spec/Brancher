//! UI: функция от состояния. Рисует [`AppState`] и возвращает [`Action`]-ы — ничего не
//! запускает и не читает с диска (раздел 3, правило потоков).

mod diff_view;
mod graph;
mod repo_view;
mod welcome;

use brancher_core::{Action, AppState, Severity};
use egui::Ui;
use widgets::Theme;

/// Локальное состояние UI, которое не относится к модели (тексты полей ввода, тема).
#[derive(Default)]
pub struct BrancherUi {
    pub dark: bool,
    open_path: String,
}

impl BrancherUi {
    /// Рисует окно целиком и возвращает действия пользователя за этот кадр.
    pub fn show(&mut self, ui: &mut Ui, state: &AppState) -> Vec<Action> {
        let ctx = ui.ctx().clone();
        let theme = if self.dark { Theme::dark() } else { Theme::light() };
        if Theme::installed(&ctx).as_ref() != Some(&theme) {
            theme.install(&ctx);
        }

        let mut actions = Vec::new();
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(Theme::get(&ctx).background)
                    .inner_margin(12),
            )
            .show(ui, |ui| {
                self.tab_bar(ui, state, &mut actions);
                ui.separator();
                notifications(ui, state, &mut actions);
                match state.active_repo() {
                    None => welcome::show(ui, state, &mut self.open_path, &mut actions),
                    Some(repo) => repo_view::show(ui, repo, &state.settings, &mut actions),
                }
            });
        shortcuts(&ctx, state, &mut actions);
        actions
    }

    fn tab_bar(&mut self, ui: &mut Ui, state: &AppState, actions: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            if ui
                .selectable_label(state.active.is_none(), "⌂ Репозитории")
                .clicked()
            {
                actions.push(Action::ShowWelcome);
            }
            for (i, repo) in state.tabs.iter().enumerate() {
                let title = if repo.busy > 0 {
                    format!("{} …", repo.name)
                } else {
                    repo.name.clone()
                };
                if ui.selectable_label(state.active == Some(i), title).clicked() {
                    actions.push(Action::SelectTab(i));
                }
                if ui.small_button("×").on_hover_text("Закрыть (Ctrl+W)").clicked() {
                    actions.push(Action::CloseTab(i));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(widgets::Switch::new(&mut self.dark).label("Тёмная тема"));
            });
        });
    }
}

fn notifications(ui: &mut Ui, state: &AppState, actions: &mut Vec<Action>) {
    for (i, n) in state.notifications.iter().enumerate() {
        ui.horizontal(|ui| {
            let mut alert = widgets::Alert::new(&n.title).description(&n.details);
            if n.severity == Severity::Error {
                alert = alert.destructive();
            }
            alert.show(ui);
            if ui.small_button("Копировать").clicked() {
                ui.ctx().copy_text(format!("{}\n{}", n.title, n.details));
            }
            if ui.small_button("×").clicked() {
                actions.push(Action::DismissNotification(i));
            }
        });
    }
}

/// Горячие клавиши раздела 10: Ctrl+W — закрыть вкладку, Ctrl+Tab — следующая, F5 — обновить.
fn shortcuts(ctx: &egui::Context, state: &AppState, actions: &mut Vec<Action>) {
    use egui::{Key, KeyboardShortcut, Modifiers};
    let pressed = |m, k| ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(m, k)));
    if let Some(active) = state.active {
        if pressed(Modifiers::COMMAND, Key::W) {
            actions.push(Action::CloseTab(active));
        }
        if pressed(Modifiers::COMMAND, Key::Tab) && !state.tabs.is_empty() {
            actions.push(Action::SelectTab((active + 1) % state.tabs.len()));
        }
        if pressed(Modifiers::NONE, Key::F5) {
            actions.push(Action::Refresh);
        }
    }
}
