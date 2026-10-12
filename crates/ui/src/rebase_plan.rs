//! Диалог interactive rebase: порядок коммитов, действие для каждого и новые сообщения.

use brancher_core::{Action, RebaseAction, RebasePlan};
use egui::{Context, RichText};

const ACTIONS: [&str; 5] = ["pick", "reword", "squash", "fixup", "drop"];

pub fn dialog(ctx: &Context, plan: &RebasePlan, actions: &mut Vec<Action>) {
    let error = plan.error();
    let theme = widgets::Theme::get(ctx);
    let resp = widgets::Dialog::new("rebase-plan", format!("Interactive rebase {}", plan.branch))
        .description(
            "Сверху — самый старый коммит, git применит их сверху вниз. squash и fixup сливают \
             коммит с предыдущим, drop удаляет его.",
        )
        .width(720.0)
        .show(ctx, |ui| {
            let n = plan.steps.len();
            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                for (i, step) in plan.steps.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if arrow_button(ui, true, i > 0).clicked() {
                            actions.push(Action::MoveRebaseStep { index: i, up: true });
                        }
                        if arrow_button(ui, false, i + 1 < n).clicked() {
                            actions.push(Action::MoveRebaseStep { index: i, up: false });
                        }
                        let current = RebaseAction::ALL.iter().position(|a| *a == step.action);
                        let mut selected = current;
                        ui.push_id(("rebase-action", i), |ui| {
                            ui.add(widgets::Select::new(&mut selected, &ACTIONS).width(110.0));
                        });
                        if selected != current {
                            if let Some(action) = selected.map(|k| RebaseAction::ALL[k]) {
                                actions.push(Action::SetRebaseAction { index: i, action });
                            }
                        }
                        let short: String = step.commit.chars().take(10).collect();
                        ui.label(RichText::new(short).monospace().color(theme.muted_foreground));
                        let mut subject = RichText::new(&step.subject);
                        if step.action == RebaseAction::Drop {
                            subject = subject.strikethrough().color(theme.muted_foreground);
                        }
                        ui.add(egui::Label::new(subject).truncate());
                    });
                    if step.action == RebaseAction::Reword {
                        match &step.message {
                            Some(m) => {
                                let mut text = m.clone();
                                ui.push_id(("rebase-message", i), |ui| {
                                    ui.add(widgets::Textarea::new(&mut text).rows(3));
                                });
                                if text != *m {
                                    actions.push(Action::SetRebaseMessage {
                                        index: i,
                                        message: text,
                                    });
                                }
                            }
                            None => {
                                widgets::muted(ui, "Загрузка сообщения…");
                            }
                        };
                        ui.add_space(4.0);
                    }
                }
            });
            if let Some(e) = error {
                ui.add_space(4.0);
                ui.label(RichText::new(e).color(theme.destructive));
            }
            widgets::footer(ui, |ui| {
                let run = ui
                    .add_enabled(error.is_none(), widgets::Button::new("Начать rebase"))
                    .clicked();
                let cancel = ui.add(widgets::Button::outline("Отмена")).clicked();
                (run, cancel)
            })
        });
    let (run, cancel) = resp.inner;
    if run {
        actions.push(Action::RunRebasePlan);
    } else if cancel || resp.should_close {
        actions.push(Action::CancelRebasePlan);
    }
}

/// Кнопка со стрелкой-шевроном: рисуется линиями, а не символом шрифта.
fn arrow_button(ui: &mut egui::Ui, up: bool, enabled: bool) -> egui::Response {
    let size = egui::vec2(28.0, 28.0);
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let t = widgets::Theme::get(ui.ctx());
    let p = ui.painter();
    if response.hovered() && enabled {
        p.rect_filled(rect, t.radius_sm(), t.accent);
    }
    let color = if enabled { t.foreground } else { t.border };
    let c = rect.center();
    let (dx, dy) = (5.0, if up { -2.5 } else { 2.5 });
    let tip = c + egui::vec2(0.0, dy);
    let stroke = egui::Stroke::new(1.5, color);
    p.line_segment([c + egui::vec2(-dx, -dy), tip], stroke);
    p.line_segment([tip, c + egui::vec2(dx, -dy)], stroke);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
