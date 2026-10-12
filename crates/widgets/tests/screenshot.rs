//! Renders the gallery to `tests/snapshots/`.
//! Update the images with `UPDATE_SNAPSHOTS=1 cargo test`.

use egui_kittest::{kittest::Queryable as _, Harness};
use widgets::demo::Gallery;

fn harness(dark: bool) -> Harness<'static> {
    let mut gallery = Gallery::default();
    gallery.dark = dark;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1240.0, 1180.0))
        .build_ui(move |ui| gallery.ui(ui));
    harness.run_steps(4);
    harness
}

#[test]
fn gallery_light() {
    harness(false).snapshot("gallery_light");
}

#[test]
fn gallery_dark() {
    harness(true).snapshot("gallery_dark");
}

#[test]
fn dropdown_menu_open() {
    let mut h = harness(false);
    h.get_by_label("Open menu").click();
    h.run_steps(4);
    h.snapshot("dropdown_menu");
}

#[test]
fn select_open() {
    let mut h = harness(true);
    h.get_by_label("Select a fruit").click();
    h.run_steps(4);
    h.snapshot("select");
}

#[test]
fn dialog_open() {
    let mut h = harness(false);
    h.get_by_label("Edit profile").click();
    h.run_steps(8);
    h.snapshot("dialog");
}

#[test]
fn tree_open() {
    let mut h = harness(true);
    h.get_by_label("components").click();
    h.run_steps(8);
    h.get_by_label("button.rs").click();
    h.run_steps(4);
    h.snapshot("tree");
}

#[test]
fn data_table_sorted() {
    let mut h = harness(false);
    h.get_by_label("Amount").click();
    h.run_steps(2);
    h.get_by_label("Next").click();
    h.run_steps(4);
    h.snapshot("data_table");
}

#[test]
fn data_table_scroll() {
    let rows: Vec<Vec<String>> = (1..=20)
        .map(|i| vec![format!("user{i:02}@example.com"), format!("${}.00", i * 37)])
        .collect();
    let mut selected = std::collections::HashSet::from([2]);
    let mut h = Harness::builder()
        .with_size(egui::vec2(420.0, 420.0))
        .build_ui(move |ui| {
            widgets::Theme::light().install(ui.ctx());
            egui::Frame::new()
                .fill(egui::Color32::WHITE)
                .inner_margin(16)
                .show(ui, |ui| {
                    widgets::DataTable::new(
                        "scroll",
                        vec![
                            widgets::Column::new("Email").weight(2.0).sortable(),
                            widgets::Column::new("Amount").right(),
                        ],
                    )
                    .filter("Filter emails...")
                    .selection(&mut selected)
                    .max_height(220.0)
                    .show(ui, &rows);
                });
        });
    h.run_steps(2);
    h.event(egui::Event::PointerMoved(egui::pos2(200.0, 200.0)));
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -150.0),
        modifiers: egui::Modifiers::NONE,
        phase: egui::TouchPhase::Move,
    });
    h.run_steps(30);
    h.snapshot("data_table_scroll");
}

#[test]
fn tree_scroll() {
    let mut selected = None;
    let mut h = Harness::builder()
        .with_size(egui::vec2(320.0, 300.0))
        .build_ui(move |ui| {
            widgets::Theme::light().install(ui.ctx());
            egui::Frame::new()
                .fill(egui::Color32::WHITE)
                .inner_margin(16)
                .show(ui, |ui| {
                    widgets::Tree::new("big")
                        .default_open_depth(1)
                        .max_height(240.0)
                        .show(ui, &mut selected, |tree| {
                            tree.folder(0, "src", |tree| {
                                for i in 1..=20 {
                                    tree.leaf(i, &format!("file{i:02}.rs"));
                                }
                            });
                        });
                });
        });
    h.run_steps(2);
    h.get_by_label("src").click();
    h.run_steps(2);
    h.get_by_label("src").click(); // reopen: the first click closed it
    h.run_steps(8);
    for _ in 0..12 {
        h.key_press(egui::Key::ArrowDown);
        h.run_steps(2);
    }
    h.run_steps(20);
    h.snapshot("tree_scroll");
}

/// Git-status style tree: custom row content and multiple selection.
#[test]
fn tree_git() {
    use egui::{Color32, Layout, RichText};
    use widgets::{Badge, Theme, Tree};
    let files = [
        ("src/lib.rs", "M"),
        ("src/tree.rs", "A"),
        ("src/old.rs", "D"),
        ("README.md", "M"),
    ];
    let mut staged = std::collections::HashSet::new();
    let mut h = Harness::builder()
        .with_size(egui::vec2(360.0, 210.0))
        .build_ui(move |ui| {
            Theme::dark().install(ui.ctx());
            let t = Theme::get(ui.ctx());
            egui::Frame::new()
                .fill(t.background)
                .inner_margin(16)
                .show(ui, |ui| {
                    Tree::new("git")
                        .default_open_depth(2)
                        .show(ui, &mut staged, |tree| {
                            tree.folder_ui(
                                "main",
                                "main",
                                |ui| {
                                    ui.painter().circle_filled(
                                        ui.cursor().left_center() + egui::vec2(3.0, 0.0),
                                        3.0,
                                        Color32::from_rgb(34, 197, 94),
                                    );
                                    ui.add_space(10.0);
                                    ui.label("main");
                                    ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.add(Badge::new("⬇5").outline());
                                        ui.add(Badge::new("⬆2").secondary());
                                    });
                                },
                                |tree| {
                                    for (path, status) in files {
                                        let color = match status {
                                            "A" => Color32::from_rgb(34, 197, 94),
                                            "D" => t.destructive,
                                            _ => Color32::from_rgb(234, 179, 8),
                                        };
                                        let name = path.rsplit('/').next().unwrap();
                                        tree.leaf_ui(path, path, |ui| {
                                            ui.label(name);
                                            ui.label(
                                                RichText::new(path).size(12.0).color(t.muted_foreground),
                                            );
                                            ui.with_layout(
                                                Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    ui.label(RichText::new(status).monospace().color(color));
                                                },
                                            );
                                        });
                                    }
                                },
                            );
                        });
                });
        });
    h.run_steps(2);
    h.get_all_by_label("src/lib.rs").next().unwrap().click(); // the row; its content repeats the path
    h.run_steps(2);
    h.get_all_by_label("src/old.rs")
        .next()
        .unwrap()
        .click_modifiers(egui::Modifiers::SHIFT);
    h.run_steps(2);
    h.get_all_by_label("README.md")
        .next()
        .unwrap()
        .click_modifiers(egui::Modifiers::COMMAND);
    h.run_steps(4);
    h.snapshot("tree_git");
}

/// Commit history: 20 000 rows, custom cells, cached sort and filter.
#[test]
fn data_table_commits() {
    use egui::{Color32, RichText};
    use widgets::{Badge, Column, DataTable, Theme};
    let rows: Vec<Vec<String>> = (0..20_000)
        .map(|i| {
            vec![
                String::new(),
                format!("Commit message number {i}"),
                ["George", "Ada", "Linus"][i % 3].to_string(),
                format!("{:07x}", i * 2_654_435_761usize % 0xfff_ffff),
            ]
        })
        .collect();
    let mut h = Harness::builder()
        .with_size(egui::vec2(720.0, 420.0))
        .build_ui(move |ui| {
            Theme::light().install(ui.ctx());
            let t = Theme::get(ui.ctx());
            egui::Frame::new()
                .fill(t.background)
                .inner_margin(16)
                .show(ui, |ui| {
                    let lanes = [Color32::from_rgb(59, 130, 246), Color32::from_rgb(168, 85, 247)];
                    DataTable::new(
                        "commits",
                        vec![
                            Column::new("Graph").weight(0.5),
                            Column::new("Subject").weight(3.0).sortable(),
                            Column::new("Author").sortable(),
                            Column::new("Hash").right(),
                        ],
                    )
                    .filter("Filter commits...")
                    .row_height(32.0)
                    .max_height(260.0)
                    .cell_ui(|ui, row, col| match col {
                        0 => {
                            // Two lanes: a straight line and a dot on the commit's lane.
                            let r = ui.max_rect();
                            let x = |lane: usize| r.left() + 6.0 + lane as f32 * 14.0;
                            let p = ui.painter();
                            for (lane, c) in lanes.iter().enumerate() {
                                p.vline(x(lane), r.y_range(), egui::Stroke::new(2.0, *c));
                            }
                            let lane = (row % 5 == 0) as usize;
                            p.circle(
                                egui::pos2(x(lane), r.center().y),
                                4.0,
                                t.background,
                                egui::Stroke::new(2.0, lanes[lane]),
                            );
                            true
                        }
                        1 if row % 7 == 0 => {
                            ui.add(Badge::new("v0.1").outline());
                            ui.label(RichText::new(&rows[row][1]));
                            true
                        }
                        3 => {
                            ui.label(RichText::new(&rows[row][3]).monospace().color(t.muted_foreground));
                            true
                        }
                        _ => false,
                    })
                    .show(ui, &rows);
                });
        });
    h.run_steps(2);
    h.get_by_label("Author").click();
    h.run_steps(2);
    let start = std::time::Instant::now();
    h.run_steps(60);
    let per_frame = start.elapsed() / 60;
    eprintln!("data_table_commits: {per_frame:?} per frame with 20 000 rows");
    h.snapshot("data_table_commits");
}
