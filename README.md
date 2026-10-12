# Brancher

Бесплатный кроссплатформенный Git-клиент с открытым кодом: граф истории, работа с рабочей
копией до уровня строк и все повседневные операции Git без терминала. Референс поведения —
[SourceGit](https://github.com/sourcegit-scm/sourcegit), код пишется с нуля на Rust.

Спецификация: «Спецификация: десктопный Git-клиент (аналог SourceGit)» (Claude Docs).

## Стек

Rust · eframe/egui 0.36 · компоненты из shadcn_egui (перенесены в `crates/widgets`) ·
установленный `git` (2.25.1+) как единственный источник истины.

## Структура (раздел 3 спецификации)

| Крейт | Пакет | Содержимое |
|---|---|---|
| `crates/app` | `brancher` (bin) | `main`, окно eframe, цикл кадра, helper-режим (заготовка) |
| `crates/ui` | `brancher-ui` | Стартовая страница, вкладки, ветки, история, изменения и коммит |
| `crates/widgets` | `widgets` | Код shadcn_egui @ `7425c3f` (MIT), доработки — здесь, см. `UPSTREAM.md` |
| `crates/core` | `brancher-core` | Модели, `AppState`/`RepoState`, `Action` → `reduce` → `Effect`; без I/O |
| `crates/git` | `brancher-git` | `Runner` (без shell, с отменой), аргументы и парсеры status/log/refs |
| `crates/services` | `brancher-services` | Очередь задач на репозиторий, результаты в UI через канал + repaint |

Пакеты называются `brancher-*`: имя `core` занято встроенным крейтом Rust.

## Запуск

```sh
cargo run                       # стартовая страница
cargo run -- /path/to/repo      # сразу открыть репозиторий
cargo run -p widgets --example gallery   # галерея компонентов
```

## Тесты

```sh
cargo test -p brancher-core -p brancher-git -p brancher-services   # без окна, нужен только git
cargo test --workspace                                             # + snapshot-тесты widgets (wgpu)
```

Слои `core` и `git` зависят только от std. Сторонние крейты из раздела 3 добавляются вместе
с функциями, которым они нужны: syntect и notify уже в `services`, serde и keyring — дальше.

## Что уже работает

- Открытие репозитория по пути, вкладки (Ctrl+W, Ctrl+Tab), F5 — обновить.
- Статус (porcelain v2, переименования, конфликты, не-ASCII пути), ветки/remotes/теги с ↑↓.
- История с виртуальной прокруткой (до 20 000 коммитов, `--all --date-order`).
- Stage/unstage файлов (двойной клик, «Stage all»), commit и amend (Ctrl+Enter).
- Граф коммитов (раскладка в фоне) и diff коммита, индекса и рабочей копии.
- Частичный stage/unstage: ханк целиком или выбранные строки (клик, Shift/Ctrl+клик, S).
- Discard строк и ханков в рабочей копии — с подтверждением.
- Diff в одну или две колонки (side-by-side).
- Подсветка синтаксиса в diff (syntect, считается в фоне, цвета под светлую и тёмную тему).
- Автообновление при изменениях снаружи (редактор, терминал): notify, пачки событий,
  игнорируемые git файлы (сборка, `node_modules`) не вызывают обновления.
- Ошибки git показываются со stderr и кнопкой «Копировать».

## Следующие шаги (прототип → MVP)

1. Прототип рисков egui: NFR-01…03, IME и шрифты на трёх ОС.
2. Доработки `widgets` для MVP: интерактивные строки DataTable, контекстные меню, вкладки с закрытием,
   combobox с поиском, постоянные уведомления, Alert с кнопками, drag в Tree.
3. Клонирование (REPO-03), сохранение вкладок и настроек.

## Лицензия

MIT. Код `crates/widgets` — MIT, © georgebigscar-spec (shadcn_egui).
