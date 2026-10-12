//! Точка входа: окно eframe и цикл кадра из раздела 3.
//!
//! 1. Забрать готовые `JobResult` из канала и применить к состоянию.
//! 2. Нарисовать UI; клики и клавиши превращаются в `Action`.
//! 3. Применить `Action`: изменить состояние или отправить `Job` в очередь репозитория.
//! 4. Worker выполняет `Job`, отправляет `JobResult` и вызывает `request_repaint()`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;

use brancher_core::{apply_result, job_started, reduce, Action, AppState, Effect, Session};
use brancher_git::Git;
use brancher_services::session::{self, SessionStore};
use brancher_services::Jobs;
use brancher_ui::BrancherUi;

fn main() -> eframe::Result {
    // Helper-режим (`brancher askpass`, `brancher editor`) для GIT_ASKPASS и GIT_SEQUENCE_EDITOR
    // появится вместе с IPC (NFR-09); пока просто отказываемся, чтобы git не завис.
    if let Some(mode) = std::env::args()
        .nth(1)
        .filter(|a| a == "askpass" || a == "editor")
    {
        eprintln!("helper-режим `{mode}` ещё не реализован");
        std::process::exit(1);
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Brancher")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native("Brancher", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

struct App {
    state: AppState,
    ui: BrancherUi,
    jobs: Option<Jobs>,
    /// `None` — не нашлась папка настроек, сессия не сохраняется.
    store: Option<SessionStore>,
    /// Последняя отправленная на запись сессия.
    saved: Session,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut state = AppState::new();
        let ctx = cc.egui_ctx.clone();
        let jobs = match Git::detect() {
            Ok((git, _version)) => Some(Jobs::new(git, Arc::new(move || ctx.request_repaint()))),
            Err(e) => {
                state.notifications.push(brancher_core::Notification {
                    severity: brancher_core::Severity::Error,
                    title: "Git не найден".into(),
                    details: format!("{e}. Установите Git 2.25.1+ и перезапустите приложение."),
                });
                None
            }
        };
        let path = session::default_path();
        let mut restored = path.as_deref().map(session::load).unwrap_or_default();
        // Папки, которые удалили или переместили, не открываем — останутся в «Недавних».
        let active = restored.active.and_then(|i| restored.tabs.get(i).cloned());
        restored.tabs.retain(|p| p.is_dir());
        restored.active = active.and_then(|a| restored.tabs.iter().position(|p| *p == a));
        let mut app = Self {
            state,
            ui: BrancherUi::default(),
            jobs,
            store: path.map(SessionStore::new),
            saved: Session::default(),
        };
        app.dispatch(Action::Restore(restored));
        app.saved = app.state.session();
        // Открыть репозиторий из аргумента командной строки: `brancher /path/to/repo`.
        if let Some(path) = std::env::args_os().nth(1) {
            app.dispatch(Action::OpenRepo(path.into()));
        }
        app
    }

    fn dispatch(&mut self, action: Action) {
        let effects = reduce(&mut self.state, action);
        self.run(effects);
    }

    fn run(&mut self, effects: Vec<Effect>) {
        let Some(jobs) = self.jobs.as_mut() else { return };
        for effect in effects {
            match effect {
                Effect::StartRepo { id, path } => jobs.start_repo(id, path),
                Effect::StopRepo(id) => jobs.stop_repo(id),
                Effect::Clone { id, url, dest } => jobs.clone_repo(id, url, dest),
                Effect::CancelClone => jobs.cancel_clone(),
                Effect::Spawn { repo, job } => {
                    if jobs.spawn(repo, job) {
                        job_started(&mut self.state, repo);
                    }
                }
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // 1. Результаты фоновых задач.
        let completed = self.jobs.as_ref().map(Jobs::drain).unwrap_or_default();
        for (repo, result) in completed {
            let effects = apply_result(&mut self.state, repo, result);
            self.run(effects);
        }
        // 2–3. Кадр и действия пользователя.
        for action in self.ui.show(ui, &self.state) {
            self.dispatch(action);
        }
        // 4. Сессия: вкладки, недавние и настройки пишутся в фоне при каждом изменении.
        let session = self.state.session();
        if session != self.saved {
            if let Some(store) = &self.store {
                store.save(session.clone());
            }
            self.saved = session;
        }
    }
}
