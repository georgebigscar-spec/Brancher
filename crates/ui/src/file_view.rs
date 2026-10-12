//! Blame, история файла и сравнение веток в нижней половине центральной панели, на месте diff.
//! Клик по строке blame или по коммиту открывает diff этого коммита.

use brancher_core::{Action, Blame, Commit, FileView, RepoState};
use egui::{pos2, vec2, Align2, FontId, Sense, Ui};

const ROW_HEIGHT: f32 = 20.0;

pub fn show(ui: &mut Ui, repo: &RepoState, view: &FileView, actions: &mut Vec<Action>) {
    if let FileView::Compare {
        base,
        other,
        only_other,
        only_base,
    } = view
    {
        let lists = only_other.as_deref().zip(only_base.as_deref());
        return compare(ui, repo, base, other, lists, actions);
    }
    header(ui, view, actions);
    ui.separator();
    match view {
        FileView::Blame { blame: None, .. } | FileView::History { commits: None, .. } => {
            ui.spinner();
        }
        FileView::Compare { .. } => unreachable!("показано выше"),
        FileView::Blame {
            blame: Some(blame), ..
        } => blame_rows(ui, blame, actions),
        FileView::History {
            commits: Some(commits),
            ..
        } => history_rows(ui, repo, commits, actions),
    }
}

fn header(ui: &mut Ui, view: &FileView, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        let path = view.path().to_owned();
        let (kind, other) = match view {
            FileView::Blame { .. } => ("Blame", "История файла"),
            FileView::History { .. } | FileView::Compare { .. } => ("История", "Blame"),
        };
        ui.add(widgets::Badge::new(kind).secondary());
        ui.add(egui::Label::new(egui::RichText::new(&path).strong()).truncate());
        if let FileView::Blame { rev: Some(rev), .. } = view {
            widgets::muted(ui, format!("в {}", &rev[..rev.len().min(10)]));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let close = ui
                .add(widgets::Button::ghost("×").size(widgets::ButtonSize::Icon))
                .on_hover_text("Закрыть (Esc)");
            let esc = ui.memory(|m| m.focused().is_none()) && ui.input(|i| i.key_pressed(egui::Key::Escape));
            close_on(close, esc, actions);
            if ui.add(widgets::Button::outline(other)).clicked() {
                actions.push(match view {
                    FileView::Blame { .. } => Action::ShowFileHistory(path.clone()),
                    _ => Action::ShowBlame { path, rev: None },
                });
            }
        });
    });
}

fn blame_rows(ui: &mut Ui, blame: &Blame, actions: &mut Vec<Action>) {
    let t = widgets::Theme::get(ui.ctx());
    let mono = FontId::monospace(13.0);
    let body = FontId::proportional(13.0);
    let info_width = 300.0;
    let number_width = 44.0;
    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::both()
        .id_salt("blame")
        .auto_shrink([false, false])
        .show_rows(ui, ROW_HEIGHT, blame.lines.len(), |ui, range| {
            for i in range {
                let (ci, text) = &blame.lines[i];
                let c = &blame.commits[*ci];
                // Сведения о коммите — только в первой строке подряд идущего блока.
                let first = i == 0 || blame.lines[i - 1].0 != *ci;
                let width = ui.available_width().max(info_width + number_width + 600.0);
                let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::click());
                let response = if c.is_uncommitted() {
                    response.on_hover_text("Ещё не закоммичено")
                } else {
                    response.on_hover_text(format!(
                        "{} · {} · {}\n{}",
                        &c.id[..10],
                        c.author,
                        crate::format_date(c.time),
                        c.summary
                    ))
                };
                if response.clicked() && !c.is_uncommitted() {
                    actions.push(Action::SelectCommit(Some(c.id.clone())));
                }
                if !ui.is_rect_visible(rect) {
                    continue;
                }
                let painter = ui.painter_at(rect);
                if response.hovered() {
                    painter.rect_filled(rect, 0.0, t.accent);
                } else if *ci % 2 == 1 {
                    painter.rect_filled(rect, 0.0, t.muted.gamma_multiply(0.5));
                }
                let y = rect.center().y;
                if first {
                    let info = egui::Rect::from_min_size(rect.min, vec2(info_width - 8.0, ROW_HEIGHT));
                    let p = painter.with_clip_rect(info);
                    let (id, who) = if c.is_uncommitted() {
                        ("—".to_owned(), "не закоммичено".to_owned())
                    } else {
                        (c.id[..8].to_owned(), c.author.clone())
                    };
                    let r = p.text(
                        pos2(rect.left() + 6.0, y),
                        Align2::LEFT_CENTER,
                        id,
                        mono.clone(),
                        t.muted_foreground,
                    );
                    if !c.is_uncommitted() {
                        p.text(
                            pos2(info.right(), y),
                            Align2::RIGHT_CENTER,
                            crate::format_date(c.time),
                            body.clone(),
                            t.muted_foreground,
                        );
                    }
                    let who_clip =
                        egui::Rect::from_x_y_ranges(r.right() + 8.0..=info.right() - 84.0, rect.y_range());
                    p.with_clip_rect(who_clip).text(
                        pos2(r.right() + 8.0, y),
                        Align2::LEFT_CENTER,
                        who,
                        body.clone(),
                        t.foreground,
                    );
                }
                painter.text(
                    pos2(rect.left() + info_width + number_width - 8.0, y),
                    Align2::RIGHT_CENTER,
                    (i + 1).to_string(),
                    mono.clone(),
                    t.muted_foreground,
                );
                painter.text(
                    pos2(rect.left() + info_width + number_width, y),
                    Align2::LEFT_CENTER,
                    text.replace('\t', "    "),
                    mono.clone(),
                    t.foreground,
                );
            }
        });
}

fn history_rows(ui: &mut Ui, repo: &RepoState, commits: &[Commit], actions: &mut Vec<Action>) {
    if commits.is_empty() {
        widgets::muted(ui, "Коммитов с этим файлом нет.");
        return;
    }
    commit_rows(ui, "file-history", repo, commits, actions);
}

fn close_on(close: egui::Response, esc: bool, actions: &mut Vec<Action>) {
    if close.clicked() || esc {
        actions.push(Action::CloseFileView);
    }
}

/// Сравнение `other` с текущей веткой: слева коммиты, которые принесёт merge `other`,
/// справа — коммиты текущей ветки, которых нет в `other`.
fn compare(
    ui: &mut Ui,
    repo: &RepoState,
    base: &str,
    other: &str,
    lists: Option<(&[Commit], &[Commit])>,
    actions: &mut Vec<Action>,
) {
    ui.horizontal(|ui| {
        ui.add(widgets::Badge::new("Сравнение").secondary());
        ui.add(egui::Label::new(egui::RichText::new(format!("{other} ↔ {base}")).strong()).truncate());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let close = ui
                .add(widgets::Button::ghost("×").size(widgets::ButtonSize::Icon))
                .on_hover_text("Закрыть (Esc)");
            let esc = ui.memory(|m| m.focused().is_none()) && ui.input(|i| i.key_pressed(egui::Key::Escape));
            close_on(close, esc, actions);
            let diff = ui
                .add(widgets::Button::outline("Diff изменений"))
                .on_hover_text(format!(
                    "Что изменилось в {other} с общего предка с {base}: то, что принесёт слияние"
                ));
            if diff.clicked() {
                actions.push(Action::ShowCompareDiff);
            }
            if lists.is_some_and(|(theirs, _)| !theirs.is_empty())
                && repo.operation().is_none()
                && ui
                    .add(widgets::Button::outline(format!("Слить в {base}")))
                    .clicked()
            {
                actions.push(Action::Merge(other.to_owned()));
            }
        });
    });
    ui.separator();
    let Some((theirs, ours)) = lists else {
        ui.spinner();
        return;
    };
    ui.columns(2, |cols| {
        let sides = [
            (theirs, other, base, "compare-theirs"),
            (ours, base, other, "compare-ours"),
        ];
        for (ui, (commits, from, to, salt)) in cols.iter_mut().zip(sides) {
            ui.label(egui::RichText::new(format!("Только в {from}: {}", commits.len())).strong());
            if commits.is_empty() {
                widgets::muted(ui, format!("Всё из {from} уже есть в {to}."));
            } else {
                commit_rows(ui, salt, repo, commits, actions);
            }
        }
    });
}

fn commit_rows(ui: &mut Ui, salt: &str, repo: &RepoState, commits: &[Commit], actions: &mut Vec<Action>) {
    let t = widgets::Theme::get(ui.ctx());
    let mono = FontId::monospace(13.0);
    let body = FontId::proportional(14.0);
    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::vertical()
        .id_salt(salt)
        .auto_shrink([false, false])
        .show_rows(ui, 24.0, commits.len(), |ui, range| {
            for c in &commits[range] {
                let (rect, response) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                if response.clicked() {
                    actions.push(Action::SelectCommit(Some(c.id.clone())));
                }
                crate::branches::commit_menu(&response, c, repo, actions);
                if !ui.is_rect_visible(rect) {
                    continue;
                }
                let painter = ui.painter_at(rect);
                if response.hovered() || repo.selected_commit.as_deref() == Some(c.id.as_str()) {
                    painter.rect_filled(rect, 0.0, t.accent);
                }
                let y = rect.center().y;
                let right = painter.text(
                    pos2(rect.right() - 8.0, y),
                    Align2::RIGHT_CENTER,
                    format!("{}  {}", c.author.name, crate::format_date(c.author.time)),
                    body.clone(),
                    t.muted_foreground,
                );
                let id = painter.text(
                    pos2(rect.left() + 6.0, y),
                    Align2::LEFT_CENTER,
                    c.short_id(),
                    mono.clone(),
                    t.muted_foreground,
                );
                let clip =
                    egui::Rect::from_x_y_ranges(id.right() + 10.0..=right.left() - 12.0, rect.y_range());
                painter.with_clip_rect(clip).text(
                    pos2(id.right() + 10.0, y),
                    Align2::LEFT_CENTER,
                    &c.subject,
                    body.clone(),
                    t.foreground,
                );
            }
        });
}
