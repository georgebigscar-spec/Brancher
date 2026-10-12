//! Стартовая страница (REPO-01…03): открыть, клонировать, недавние.
//! Группы, drag-and-drop и диалог выбора папки — дальше по плану.

use std::path::PathBuf;

use brancher_core::{clone_dir_name, Action, AppState, Cloning};
use egui::Ui;

/// Поля ввода стартовой страницы.
#[derive(Default)]
pub struct Form {
    open_path: String,
    clone_url: String,
    /// Папка, в которой появится клон; по умолчанию — домашняя.
    clone_parent: Option<String>,
}

pub fn show(ui: &mut Ui, state: &AppState, form: &mut Form, actions: &mut Vec<Action>) {
    let open_path = &mut form.open_path;
    ui.add_space(24.0);
    widgets::h2(ui, "Репозитории");
    widgets::muted(
        ui,
        "Откройте папку с Git-репозиторием или клонируйте его. Диалог выбора папки появится в MVP.",
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

    ui.add_space(20.0);
    widgets::h3(ui, "Клонировать");
    match &state.cloning {
        Some(c) => clone_progress(ui, c, actions),
        None => clone_form(ui, form, actions),
    }

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

fn clone_form(ui: &mut Ui, form: &mut Form, actions: &mut Vec<Action>) {
    let parent = form.clone_parent.get_or_insert_with(home_dir);
    let mut submit = false;
    ui.horizontal(|ui| {
        let r = ui.add(
            widgets::Input::new(&mut form.clone_url)
                .placeholder("https://github.com/user/repo.git или git@host:user/repo.git")
                .width(420.0),
        );
        submit |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    });
    ui.horizontal(|ui| {
        ui.add(
            widgets::Input::new(parent)
                .placeholder("Папка, в которой создать клон")
                .width(420.0),
        );
    });
    let name = clone_dir_name(&form.clone_url);
    let dest = name
        .as_ref()
        .filter(|_| !parent.trim().is_empty())
        .map(|n| PathBuf::from(parent.trim()).join(n));
    ui.horizontal(|ui| {
        let clicked = ui
            .add_enabled(dest.is_some(), widgets::Button::new("Клонировать"))
            .clicked();
        if let Some(d) = &dest {
            widgets::muted(ui, format!("Будет создана папка {}", d.display()));
        }
        if let (true, Some(d)) = (clicked || submit, dest.clone()) {
            actions.push(Action::Clone {
                url: form.clone_url.trim().to_owned(),
                dest: d,
            });
        }
    });
}

fn clone_progress(ui: &mut Ui, c: &Cloning, actions: &mut Vec<Action>) {
    widgets::label(ui, &c.url);
    widgets::muted(ui, format!("в папку {}", c.dest.display()));
    let p = &c.progress;
    let phase = match (phase_name(&p.phase), p.percent) {
        (phase, Some(n)) => format!("{phase}: {n}%"),
        (phase, None) => format!("{phase}…"),
    };
    widgets::muted(ui, phase);
    ui.horizontal(|ui| {
        match p.percent {
            Some(n) => ui.add(widgets::Progress::new(f32::from(n) / 100.0).width(320.0)),
            // Процента ещё нет — пульсирующая полоса той же ширины.
            None => ui.add(widgets::Skeleton::new([320.0, 8.0])),
        };
        if ui.add(widgets::Button::outline("Отмена")).clicked() {
            actions.push(Action::CancelClone);
        }
    });
}

/// Этапы `git clone` по-русски; незнакомые (другая локаль git, сообщения сервера) — как есть.
fn phase_name(phase: &str) -> &str {
    match phase {
        "" => "Подключение",
        p if p.starts_with("Cloning into") => "Подключение",
        "Enumerating objects" | "Counting objects" | "Compressing objects" => "Подготовка на сервере",
        "Receiving objects" => "Загрузка объектов",
        "Resolving deltas" => "Разбор изменений",
        "Updating files" | "Checking out files" => "Распаковка файлов",
        p => p,
    }
}

/// Домашняя папка из окружения (без обращения к диску: UI не читает ФС).
fn home_dir() -> String {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default()
}
