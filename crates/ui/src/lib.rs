//! UI: функция от состояния. Рисует [`AppState`] и возвращает [`Action`]-ы — ничего не
//! запускает и не читает с диска (раздел 3, правило потоков).

mod branches;
mod diff_view;
mod graph;
mod repo_view;
mod stash;
mod welcome;

use brancher_core::{Action, AppState, Severity};
use egui::Ui;
use widgets::Theme;

/// Локальное состояние UI, которое не относится к модели (тексты полей ввода).
#[derive(Default)]
pub struct BrancherUi {
    /// Ввод для вопроса git (логин, пароль); очищается после ответа.
    prompt_answer: String,
    welcome: welcome::Form,
}

impl BrancherUi {
    /// Рисует окно целиком и возвращает действия пользователя за этот кадр.
    pub fn show(&mut self, ui: &mut Ui, state: &AppState) -> Vec<Action> {
        let ctx = ui.ctx().clone();
        let theme = if state.settings.dark {
            Theme::dark()
        } else {
            Theme::light()
        };
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
                    None => welcome::show(ui, state, &mut self.welcome, &mut actions),
                    Some(repo) => repo_view::show(ui, repo, &state.settings, &mut actions),
                }
            });
        if let Some(p) = state.prompts.front() {
            prompt_dialog(&ctx, p, &mut self.prompt_answer, &mut actions);
        }
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
                let mut dark = state.settings.dark;
                if ui
                    .add(widgets::Switch::new(&mut dark).label("Тёмная тема"))
                    .changed()
                {
                    actions.push(Action::SetDarkTheme(dark));
                }
            });
        });
    }
}

/// Вопрос git или ssh: логин, пароль, passphrase, подтверждение ключа хоста.
fn prompt_dialog(
    ctx: &egui::Context,
    p: &brancher_core::Prompt,
    answer: &mut String,
    actions: &mut Vec<Action>,
) {
    let resp = widgets::Dialog::new(("git-prompt", p.id), "Git запрашивает данные")
        .description(p.text.trim())
        .show(ctx, |ui| {
            let mut input = widgets::Input::new(answer).width(ui.available_width());
            if p.secret {
                input = input.password();
            }
            let r = ui.add(input);
            if !r.has_focus() && !ui.ctx().memory(|m| m.focused().is_some()) {
                r.request_focus();
            }
            widgets::muted(
                ui,
                "Ответ уйдёт только в git. Сохранить его может ваш credential helper.",
            );
            widgets::footer(ui, |ui| {
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                let ok = ui.add(widgets::Button::new("OK")).clicked() || enter;
                let cancel = ui.add(widgets::Button::outline("Отмена")).clicked();
                (ok, cancel)
            })
        });
    let (ok, cancel) = resp.inner;
    if ok {
        actions.push(Action::AnswerPrompt(Some(std::mem::take(answer))));
    } else if cancel || resp.should_close {
        answer.clear();
        actions.push(Action::AnswerPrompt(None));
    }
}

fn notifications(ui: &mut Ui, state: &AppState, actions: &mut Vec<Action>) {
    for (i, n) in state.notifications.iter().enumerate() {
        ui.horizontal(|ui| {
            let mut alert = widgets::Alert::new(&n.title).description(&n.details);
            if n.severity == Severity::Error {
                alert = alert.destructive();
            }
            // Место под кнопки справа; длинный stderr переносится, а не раздвигает окно.
            let width = (ui.available_width() - 190.0).max(200.0);
            ui.allocate_ui(egui::vec2(width, 0.0), |ui| alert.show(ui));
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

/// Этапы `git … --progress` (clone, fetch, push) по-русски; незнакомые (другая локаль git, сообщения сервера) — как есть.
pub(crate) fn phase_name(phase: &str) -> &str {
    match phase {
        "" => "Подключение",
        p if p.starts_with("Cloning into") => "Подключение",
        "Enumerating objects" | "Counting objects" | "Compressing objects" => "Подготовка на сервере",
        "Receiving objects" => "Загрузка объектов",
        "Resolving deltas" => "Разбор изменений",
        "Updating files" | "Checking out files" => "Распаковка файлов",
        "Writing objects" => "Отправка объектов",
        p if p.starts_with("Delta compression") => "Сжатие",
        p if p.starts_with("Fetching ") => "Получение изменений",
        p if p.starts_with("To ") || p.starts_with("From ") => "Обмен с сервером",
        p => p,
    }
}
