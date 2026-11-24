//! Field rendering (UI-03): matrix, ghost, active piece, hold and next previews, targets.

use eframe::egui::{self, Color32, Pos2, Rect, Stroke, StrokeKind, Vec2};
use stacklab_engine::board::{Board, PieceState, WIDTH};
use stacklab_engine::{Piece, Rot};

pub fn piece_color(p: Piece) -> Color32 {
    match p {
        Piece::I => Color32::from_rgb(64, 200, 220),
        Piece::O => Color32::from_rgb(230, 200, 60),
        Piece::T => Color32::from_rgb(170, 80, 200),
        Piece::S => Color32::from_rgb(110, 200, 80),
        Piece::Z => Color32::from_rgb(220, 70, 70),
        Piece::J => Color32::from_rgb(70, 110, 220),
        Piece::L => Color32::from_rgb(235, 140, 50),
    }
}

const STACK: Color32 = Color32::from_rgb(120, 120, 130);
const BG: Color32 = Color32::from_rgb(18, 18, 24);
const GRID: Color32 = Color32::from_rgb(34, 34, 44);

/// Draws the visible 20 rows (+2 hidden rows, dimmed) into `rect`. Returns the cell size.
pub struct FieldView<'a> {
    pub board: &'a Board,
    pub active: Option<PieceState>,
    pub ghost: Option<PieceState>,
    /// Target outlines (finesse target, hint, accepted answers).
    pub targets: Vec<(PieceState, Color32)>,
}

pub fn field(painter: &egui::Painter, rect: Rect, v: &FieldView) -> f32 {
    let rows = 22.0;
    let cell = (rect.height() / rows)
        .min(rect.width() / WIDTH as f32)
        .floor();
    let w = cell * WIDTH as f32;
    let origin = Pos2::new(rect.center().x - w / 2.0, rect.top() + cell * rows);
    let cell_rect = |x: i8, y: i8| {
        let min = Pos2::new(
            origin.x + x as f32 * cell,
            origin.y - (y as f32 + 1.0) * cell,
        );
        Rect::from_min_size(min, Vec2::splat(cell))
    };
    let field_rect = Rect::from_min_max(
        Pos2::new(origin.x, origin.y - 20.0 * cell),
        Pos2::new(origin.x + w, origin.y),
    );
    painter.rect_filled(field_rect, 0.0, BG);
    for y in 0..22i8 {
        for x in 0..WIDTH as i8 {
            let r = cell_rect(x, y).shrink(0.5);
            if y < 20 {
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, GRID), StrokeKind::Inside);
            }
            if v.board.occupied(x, y) {
                let c = if y >= 20 {
                    STACK.gamma_multiply(0.5)
                } else {
                    STACK
                };
                painter.rect_filled(r.shrink(1.0), 2.0, c);
            }
        }
    }
    for (t, color) in &v.targets {
        for (x, y) in t.cells() {
            painter.rect_stroke(
                cell_rect(x, y).shrink(2.0),
                2.0,
                Stroke::new(2.5, *color),
                StrokeKind::Inside,
            );
        }
    }
    if let Some(g) = v.ghost {
        for (x, y) in g.cells() {
            painter.rect_stroke(
                cell_rect(x, y).shrink(2.0),
                2.0,
                Stroke::new(1.5, piece_color(g.piece).gamma_multiply(0.6)),
                StrokeKind::Inside,
            );
        }
    }
    if let Some(a) = v.active {
        for (x, y) in a.cells() {
            painter.rect_filled(cell_rect(x, y).shrink(1.0), 2.0, piece_color(a.piece));
        }
    }
    painter.rect_stroke(
        field_rect,
        0.0,
        Stroke::new(2.0, Color32::from_gray(160)),
        StrokeKind::Outside,
    );
    cell
}

/// Small piece preview (hold / next).
pub fn mini_piece(painter: &egui::Painter, center: Pos2, cell: f32, p: Piece, dim: bool) {
    let cells = p.cells(Rot::N);
    let (minx, maxx) = (
        cells.iter().map(|c| c.0).min().unwrap(),
        cells.iter().map(|c| c.0).max().unwrap(),
    );
    let (miny, maxy) = (
        cells.iter().map(|c| c.1).min().unwrap(),
        cells.iter().map(|c| c.1).max().unwrap(),
    );
    let cx = (minx + maxx) as f32 / 2.0;
    let cy = (miny + maxy) as f32 / 2.0;
    let color = if dim {
        piece_color(p).gamma_multiply(0.35)
    } else {
        piece_color(p)
    };
    for (x, y) in cells {
        let min = Pos2::new(
            center.x + (x as f32 - cx - 0.5) * cell,
            center.y - (y as f32 - cy + 0.5) * cell,
        );
        painter.rect_filled(
            Rect::from_min_size(min, Vec2::splat(cell)).shrink(1.0),
            2.0,
            color,
        );
    }
}
