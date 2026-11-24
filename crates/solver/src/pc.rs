//! Perfect-clear search.

use crate::{piece_options, Step};
use stacklab_engine::board::{Board, FULL_ROW, WIDTH};
use stacklab_engine::{lock, placements, MoveRules, Piece};
use std::collections::HashSet;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct PcConfig {
    /// Maximum stack height used by the solution (lines that must be cleared).
    pub height: usize,
    pub hold: bool,
    /// Node budget; the search reports `exhausted = true` when it is hit.
    pub max_nodes: u64,
}

impl Default for PcConfig {
    fn default() -> Self {
        PcConfig {
            height: 4,
            hold: true,
            max_nodes: 5_000_000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PcOutcome {
    pub solution: Option<Vec<Step>>,
    pub nodes: u64,
    pub exhausted: bool,
    pub micros: u128,
}

struct Ctx<'a> {
    rules: &'a MoveRules,
    queue: &'a [Piece],
    cfg: &'a PcConfig,
    dead: HashSet<(Vec<u16>, u8, Option<Piece>, usize)>,
    nodes: u64,
    exhausted: bool,
}

/// Searches for a perfect clear that uses only the bottom `cfg.height` rows.
pub fn solve(
    board: &Board,
    queue: &[Piece],
    hold: Option<Piece>,
    rules: &MoveRules,
    cfg: &PcConfig,
) -> PcOutcome {
    let t0 = Instant::now();
    let mut ctx = Ctx {
        rules,
        queue,
        cfg,
        dead: HashSet::new(),
        nodes: 0,
        exhausted: false,
    };
    let mut path = Vec::new();
    let ok = if board.height() > cfg.height {
        false
    } else {
        dfs(&mut ctx, board, cfg.height as u8, 0, hold, &mut path)
    };
    PcOutcome {
        solution: ok.then_some(path),
        nodes: ctx.nodes,
        exhausted: ctx.exhausted,
        micros: t0.elapsed().as_micros(),
    }
}

fn empty_cells(board: &Board, h: usize) -> u32 {
    (0..h)
        .map(|y| WIDTH as u32 - board.rows[y].count_ones())
        .sum()
}

/// Every 4-connected empty region below `h` must have a size divisible by 4.
fn regions_ok(board: &Board, h: usize) -> bool {
    let mut seen = [0u16; 40];
    for y in 0..h {
        for x in 0..WIDTH {
            if board.rows[y] >> x & 1 == 1 || seen[y] >> x & 1 == 1 {
                continue;
            }
            let mut stack = vec![(x, y)];
            seen[y] |= 1 << x;
            let mut size = 0u32;
            while let Some((cx, cy)) = stack.pop() {
                size += 1;
                let mut nb = Vec::with_capacity(4);
                if cx > 0 {
                    nb.push((cx - 1, cy));
                }
                if cx + 1 < WIDTH {
                    nb.push((cx + 1, cy));
                }
                if cy > 0 {
                    nb.push((cx, cy - 1));
                }
                if cy + 1 < h {
                    nb.push((cx, cy + 1));
                }
                for (nx, ny) in nb {
                    if board.rows[ny] >> nx & 1 == 0 && seen[ny] >> nx & 1 == 0 {
                        seen[ny] |= 1 << nx;
                        stack.push((nx, ny));
                    }
                }
            }
            if !size.is_multiple_of(4) {
                return false;
            }
        }
    }
    true
}

fn dfs(
    ctx: &mut Ctx,
    board: &Board,
    h: u8,
    idx: usize,
    hold: Option<Piece>,
    path: &mut Vec<Step>,
) -> bool {
    if h == 0 {
        return board.is_empty();
    }
    ctx.nodes += 1;
    if ctx.nodes > ctx.cfg.max_nodes {
        ctx.exhausted = true;
        return false;
    }
    let hu = h as usize;
    let need = empty_cells(board, hu);
    if !need.is_multiple_of(4) {
        return false;
    }
    let available = (ctx.queue.len().saturating_sub(idx) + usize::from(hold.is_some())) as u32;
    if need / 4 > available {
        return false;
    }
    if !regions_ok(board, hu) {
        return false;
    }
    let key = (board.rows[..hu].to_vec(), h, hold, idx);
    if ctx.dead.contains(&key) {
        return false;
    }
    for (piece, nidx, nhold, used_hold) in piece_options(ctx.queue, idx, hold, ctx.cfg.hold) {
        for pl in placements(board, piece, ctx.rules) {
            if pl.state.cells().iter().any(|&(_, y)| y as usize >= hu) {
                continue;
            }
            let mut nb = board.clone();
            let res = lock(&mut nb, &pl);
            // rows above h are empty, so the remaining target shrinks by the cleared lines
            let nh = h - res.lines as u8;
            // a cleared row must have been completely inside the target
            debug_assert!(nb.rows[hu..].iter().all(|&r| r != FULL_ROW));
            path.push(Step {
                used_hold,
                placement: pl,
            });
            if dfs(ctx, &nb, nh, nidx, nhold, path) {
                return true;
            }
            path.pop();
            if ctx.exhausted {
                return false;
            }
        }
    }
    ctx.dead.insert(key);
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use stacklab_engine::piece::parse_queue;
    use stacklab_engine::Ruleset;

    #[test]
    fn two_line_pc_with_i_and_o() {
        let rules = MoveRules::from_ruleset(&Ruleset::tl_s2());
        let b = Board::from_ascii(&["####..####", "####..####"]);
        let out = solve(
            &b,
            &parse_queue("O"),
            None,
            &rules,
            &PcConfig {
                height: 2,
                ..Default::default()
            },
        );
        assert!(out.solution.is_some());
    }

    #[test]
    fn replayed_solution_ends_empty() {
        let rules = MoveRules::from_ruleset(&Ruleset::tl_s2());
        let b = Board::from_ascii(&["######....", "######...."]);
        let q = parse_queue("IIJL");
        let out = solve(
            &b,
            &q,
            None,
            &rules,
            &PcConfig {
                height: 2,
                ..Default::default()
            },
        );
        let sol = out.solution.expect("solvable");
        let mut bb = b.clone();
        for s in &sol {
            lock(&mut bb, &s.placement);
        }
        assert!(bb.is_empty());
    }

    #[test]
    fn impossible_parity_is_rejected_fast() {
        let rules = MoveRules::from_ruleset(&Ruleset::tl_s2());
        let b = Board::from_ascii(&["#########."]);
        let out = solve(
            &b,
            &parse_queue("OOOO"),
            None,
            &rules,
            &PcConfig {
                height: 4,
                ..Default::default()
            },
        );
        assert!(out.solution.is_none());
    }
}
