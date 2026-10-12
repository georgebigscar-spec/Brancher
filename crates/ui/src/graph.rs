//! Рисование графа коммитов. Раскладка уже посчитана в фоне ([`brancher_core::CommitGraph`]),
//! здесь только отрисовка строки истории.

use brancher_core::{CommitGraph, GraphRow};
use egui::epaint::CubicBezierShape;
use egui::{pos2, Color32, Pos2, Rect, Stroke, Ui};

pub const LANE_WIDTH: f32 = 14.0;
/// Шире колонка не растёт, дальние дорожки обрезаются.
const MAX_VISIBLE_LANES: u16 = 16;
const NODE_RADIUS: f32 = 4.0;
const LINE_WIDTH: f32 = 2.0;

/// Цвета веток; читаются и на светлой, и на тёмной теме.
const PALETTE: [Color32; 8] = [
    Color32::from_rgb(0x3b, 0x82, 0xf6),
    Color32::from_rgb(0x22, 0xa5, 0x5e),
    Color32::from_rgb(0xf5, 0x9e, 0x0b),
    Color32::from_rgb(0xe1, 0x4b, 0x7a),
    Color32::from_rgb(0x8b, 0x5c, 0xf6),
    Color32::from_rgb(0x06, 0xb6, 0xd4),
    Color32::from_rgb(0xef, 0x44, 0x44),
    Color32::from_rgb(0x84, 0xa1, 0x16),
];

pub fn color(index: u16) -> Color32 {
    PALETTE[index as usize % PALETTE.len()]
}

/// Ширина колонки графа для всей истории.
pub fn column_width(graph: &CommitGraph) -> f32 {
    f32::from(graph.width.clamp(1, MAX_VISIBLE_LANES)) * LANE_WIDTH + LANE_WIDTH / 2.0
}

/// Рисует граф одной строки истории в прямоугольнике `rect` (колонка графа этой строки).
pub fn paint_row(ui: &Ui, rect: Rect, row: &GraphRow, merge: bool) {
    let painter = ui.painter().with_clip_rect(rect);
    let x = |lane: u16| rect.left() + LANE_WIDTH * (f32::from(lane) + 0.5);
    let (top, mid, bottom) = (rect.top(), rect.center().y, rect.bottom());
    let node = pos2(x(row.lane), mid);

    for e in &row.through {
        painter.line_segment([pos2(x(e.lane), top), pos2(x(e.lane), bottom)], stroke(e.color));
    }
    for e in &row.incoming {
        curve(&painter, pos2(x(e.lane), top), node, e.color);
    }
    for e in &row.outgoing {
        curve(&painter, node, pos2(x(e.lane), bottom), e.color);
    }

    let c = color(row.color);
    if merge {
        painter.circle_filled(node, NODE_RADIUS, ui.visuals().panel_fill);
        painter.circle_stroke(node, NODE_RADIUS - 0.5, Stroke::new(LINE_WIDTH, c));
    } else {
        painter.circle_filled(node, NODE_RADIUS, c);
    }
}

fn stroke(color_index: u16) -> Stroke {
    Stroke::new(LINE_WIDTH, color(color_index))
}

/// Вертикальная линия, если дорожка та же, иначе плавный S-переход между дорожками.
fn curve(painter: &egui::Painter, from: Pos2, to: Pos2, color_index: u16) {
    if (from.x - to.x).abs() < f32::EPSILON {
        painter.line_segment([from, to], stroke(color_index));
        return;
    }
    let bend = (to.y - from.y) / 2.0;
    let points = [from, pos2(from.x, from.y + bend), pos2(to.x, to.y - bend), to];
    painter.add(CubicBezierShape::from_points_stroke(
        points,
        false,
        Color32::TRANSPARENT,
        stroke(color_index),
    ));
}
