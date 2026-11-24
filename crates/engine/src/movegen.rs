//! Reachable final placements for one piece (BFS over position states).
//!
//! Moves: left, right, CW, CCW, 180, one-row gravity step and sonic drop. This is the
//! reachability model shared by the solver, the problem validator and the bot.

use crate::board::{Board, PieceState, MAX_ROWS, WIDTH};
use crate::piece::{Piece, Rot};
use crate::rotation::{KickTables, RotDir, Spin, SpinRules};
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Placement {
    pub state: PieceState,
    pub spin: Spin,
}

/// Per-ruleset helpers needed to generate moves.
#[derive(Debug, Clone)]
pub struct MoveRules {
    pub kicks: KickTables,
    pub spins: SpinRules,
    pub spawn_x: i8,
    pub spawn_y: i8,
}

impl MoveRules {
    pub fn from_ruleset(rs: &crate::ruleset::Ruleset) -> MoveRules {
        MoveRules {
            kicks: KickTables::from_ruleset(rs),
            spins: SpinRules::from_ruleset(rs),
            spawn_x: rs.board.spawn_x,
            spawn_y: rs.board.spawn_y,
        }
    }

    pub fn spawn(&self, piece: Piece) -> PieceState {
        PieceState {
            piece,
            rot: Rot::N,
            x: self.spawn_x,
            y: self.spawn_y,
        }
    }
}

const XS: usize = WIDTH + 4; // x offset 2 so that centres in -2..WIDTH+1 are representable
const YS: usize = MAX_ROWS + 4;

fn idx(s: &PieceState) -> Option<usize> {
    let x = s.x as i32 + 2;
    let y = s.y as i32 + 2;
    if x < 0 || y < 0 || x as usize >= XS || y as usize >= YS {
        return None;
    }
    Some((s.rot.index() * YS + y as usize) * XS + x as usize)
}

/// All distinct lock results reachable from spawn. Placements with identical cells and
/// identical spin classification are merged.
pub fn placements(board: &Board, piece: Piece, rules: &MoveRules) -> Vec<Placement> {
    let start = rules.spawn(piece);
    if !board.fits(&start) {
        return Vec::new();
    }
    let n = 4 * XS * YS;
    let mut seen = vec![false; n];
    // arrival[i] bit 0: arrived by translation/drop; bits 1..: spin kinds via rotation
    let mut arrival = vec![0u8; n];
    let mut q = VecDeque::new();
    let si = idx(&start).expect("spawn inside matrix");
    seen[si] = true;
    arrival[si] |= 1;
    q.push_back(start);

    let push = |q: &mut VecDeque<PieceState>,
                seen: &mut Vec<bool>,
                arrival: &mut Vec<u8>,
                ns: PieceState,
                tag: u8| {
        if let Some(i) = idx(&ns) {
            arrival[i] |= tag;
            if !seen[i] {
                seen[i] = true;
                q.push_back(ns);
            }
        }
    };

    while let Some(s) = q.pop_front() {
        for dx in [-1i8, 1] {
            let ns = s.shifted(dx, 0);
            if board.fits(&ns) {
                push(&mut q, &mut seen, &mut arrival, ns, 1);
            }
        }
        let down = s.shifted(0, -1);
        if board.fits(&down) {
            push(&mut q, &mut seen, &mut arrival, down, 1);
            let sonic = board.drop_y(&s);
            push(&mut q, &mut seen, &mut arrival, sonic, 1);
        }
        for dir in [RotDir::Cw, RotDir::Ccw, RotDir::Flip] {
            if let Some((ns, k)) = rules.kicks.try_rotate(board, &s, dir) {
                let kc = rules.kicks.kick_count(piece, s.rot, ns.rot);
                let tag = if board.grounded(&ns) {
                    match rules.spins.detect(board, &ns, Some((k, kc))) {
                        Spin::None => 2,
                        Spin::Mini => 4,
                        Spin::Full => 8,
                    }
                } else {
                    2
                };
                push(&mut q, &mut seen, &mut arrival, ns, tag);
            }
        }
    }

    let mut out: Vec<Placement> = Vec::new();
    let mut keys: Vec<([(i8, i8); 4], Spin)> = Vec::new();
    for rot in 0..4 {
        for y in 0..YS {
            for x in 0..XS {
                let i = (rot * YS + y) * XS + x;
                if !seen[i] {
                    continue;
                }
                let s = PieceState {
                    piece,
                    rot: Rot::from_index(rot),
                    x: x as i8 - 2,
                    y: y as i8 - 2,
                };
                if !board.grounded(&s) {
                    continue;
                }
                let a = arrival[i];
                let mut spins = Vec::new();
                if a & 3 != 0 {
                    spins.push(Spin::None);
                }
                if a & 4 != 0 {
                    spins.push(Spin::Mini);
                }
                if a & 8 != 0 {
                    spins.push(Spin::Full);
                }
                let key = s.cell_key();
                for sp in spins {
                    if !keys.contains(&(key, sp)) {
                        keys.push((key, sp));
                        out.push(Placement { state: s, spin: sp });
                    }
                }
            }
        }
    }
    out
}

/// Result of locking a placement onto a board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LockResult {
    pub lines: u32,
    pub perfect_clear: bool,
}

pub fn lock(board: &mut Board, p: &Placement) -> LockResult {
    board.place(&p.state);
    let lines = board.clear_lines();
    LockResult {
        lines,
        perfect_clear: lines > 0 && board.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ruleset::Ruleset;

    fn rules() -> MoveRules {
        MoveRules::from_ruleset(&Ruleset::tl_s2())
    }

    #[test]
    fn empty_board_placement_counts() {
        let r = rules();
        let b = Board::default();
        let count = |p| {
            placements(&b, p, &r)
                .iter()
                .filter(|pl| pl.spin == Spin::None)
                .count()
        };
        // distinct resting cell-sets on an empty 10-wide floor
        assert_eq!(count(Piece::O), 9);
        assert_eq!(count(Piece::I), 7 + 10);
        assert_eq!(count(Piece::T), 8 + 8 + 9 + 9);
        assert_eq!(count(Piece::S), 8 + 9);
        assert_eq!(count(Piece::J), 8 + 8 + 9 + 9);
    }

    #[test]
    fn tsd_slot_is_detected_as_full_spin() {
        let r = rules();
        let b = Board::from_ascii(&["##........", "#...######", "##.#######"]);
        let tsd = placements(&b, Piece::T, &r).into_iter().find(|p| {
            p.spin == Spin::Full && p.state.rot == Rot::S && p.state.x == 2 && p.state.y == 1
        });
        let tsd = tsd.expect("TSD placement reachable");
        let mut bb = b.clone();
        let res = lock(&mut bb, &tsd);
        assert_eq!(res.lines, 2);
    }
}
