//! "Build exactly this shape" search: every placed cell must lie inside `target`, no line may
//! be cleared, and the search succeeds when all target cells are filled.

use crate::{piece_options, Step};
use stacklab_engine::board::{Board, MAX_ROWS};
use stacklab_engine::{lock, placements, MoveRules, Piece};
use std::collections::HashSet;

pub fn remaining(board: &Board, target: &Board) -> u32 {
    (0..MAX_ROWS)
        .map(|y| (target.rows[y] & !board.rows[y]).count_ones())
        .sum()
}

/// Is the shape completable from `board` with the pieces in `queue` (+ `hold`)?
/// Pieces of kind `reserve` are never placed, and the search only succeeds when the reserved
/// piece is still available afterwards (in hold or as the next piece), e.g. the T for a TSD.
pub fn solve(
    board: &Board,
    target: &Board,
    queue: &[Piece],
    hold: Option<Piece>,
    hold_enabled: bool,
    reserve: Option<Piece>,
    rules: &MoveRules,
) -> Option<(Vec<Step>, usize, Option<Piece>)> {
    let mut dead = HashSet::new();
    let mut path = Vec::new();
    let s = Search {
        target,
        queue,
        hold_enabled,
        reserve,
        rules,
    };
    dfs(&s, board, 0, hold, &mut dead, &mut path).map(|(i, h)| (path, i, h))
}

/// True when `reserve` can be played next from (`queue[idx]`, `hold`).
pub fn reserve_available(
    queue: &[Piece],
    idx: usize,
    hold: Option<Piece>,
    reserve: Option<Piece>,
) -> bool {
    match reserve {
        None => true,
        Some(r) => hold == Some(r) || queue.get(idx) == Some(&r),
    }
}

struct Search<'a> {
    target: &'a Board,
    queue: &'a [Piece],
    hold_enabled: bool,
    reserve: Option<Piece>,
    rules: &'a MoveRules,
}

fn dfs(
    s: &Search,
    board: &Board,
    idx: usize,
    hold: Option<Piece>,
    dead: &mut HashSet<(Board, usize, Option<Piece>)>,
    path: &mut Vec<Step>,
) -> Option<(usize, Option<Piece>)> {
    let rem = remaining(board, s.target);
    if rem == 0 {
        return reserve_available(s.queue, idx, hold, s.reserve).then_some((idx, hold));
    }
    let available = (s.queue.len().saturating_sub(idx) + usize::from(hold.is_some())) as u32;
    if !rem.is_multiple_of(4) || rem / 4 > available {
        return None;
    }
    let key = (board.clone(), idx, hold);
    if dead.contains(&key) {
        return None;
    }
    for (piece, nidx, nhold, used_hold) in piece_options(s.queue, idx, hold, s.hold_enabled) {
        if Some(piece) == s.reserve {
            continue;
        }
        for pl in placements(board, piece, s.rules) {
            if !inside(&pl, s.target) {
                continue;
            }
            let mut nb = board.clone();
            if lock(&mut nb, &pl).lines > 0 {
                continue;
            }
            path.push(Step {
                used_hold,
                placement: pl,
            });
            if let Some(r) = dfs(s, &nb, nidx, nhold, dead, path) {
                return Some(r);
            }
            path.pop();
        }
    }
    dead.insert(key);
    None
}

pub fn inside(pl: &stacklab_engine::Placement, target: &Board) -> bool {
    pl.state
        .cells()
        .iter()
        .all(|&(x, y)| (0..MAX_ROWS as i8).contains(&y) && target.rows[y as usize] >> x & 1 == 1)
}
