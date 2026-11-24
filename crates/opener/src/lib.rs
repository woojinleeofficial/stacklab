//! Opener library and feasibility checks.
//!
//! STEP 0 spike: a shape-based opener definition and two feasibility notions from spec 8.2:
//! * `full_sequence_feasible`: the whole piece order is known (offline verification).
//! * `visible_info_feasible`: a strategy exists that only uses the pieces a player can see
//!   (current + Next window, hold, and bag counting), i.e. AND over hidden pieces, OR over moves.
//!
//! The full branch-graph library (TKI / DT Cannon / PCO / free-stacking template) is STEP 4.

use stacklab_engine::board::Board;
use stacklab_engine::{lock, placements, MoveRules, Piece, Rot, Spin};
use stacklab_solver::fill::{inside, remaining, reserve_available};
use stacklab_solver::piece_options;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct OpenerShape {
    pub id: String,
    /// Cells the setup occupies before the finishing piece.
    pub target: Board,
    /// Piece kept for the finisher (e.g. T for a TSD), never used to build the shape.
    pub reserve: Option<Piece>,
}

impl OpenerShape {
    /// Does the reserved piece produce a full spin clearing `lines` on the finished shape?
    pub fn finisher_ok(&self, rules: &MoveRules, lines: u32) -> bool {
        let Some(r) = self.reserve else { return true };
        placements(&self.target, r, rules).iter().any(|pl| {
            let mut b = self.target.clone();
            pl.spin == Spin::Full && lock(&mut b, pl).lines == lines
        })
    }
}

/// Offline check with the complete order known.
pub fn full_sequence_feasible(
    shape: &OpenerShape,
    queue: &[Piece],
    hold: bool,
    rules: &MoveRules,
) -> bool {
    stacklab_solver::fill::solve(
        &Board::default(),
        &shape.target,
        queue,
        None,
        hold,
        shape.reserve,
        rules,
    )
    .is_some()
}

/// Pieces that can come next given everything revealed so far (7-bag counting).
fn bag_candidates(revealed: &[Piece]) -> Vec<Piece> {
    let start = revealed.len() / 7 * 7;
    let cur = &revealed[start..];
    stacklab_engine::piece::ALL_PIECES
        .iter()
        .copied()
        .filter(|p| !cur.contains(p))
        .collect()
}

type Memo = HashMap<(Board, usize, Option<Piece>, Vec<Piece>), bool>;

pub struct VisibleCheck<'a> {
    pub shape: &'a OpenerShape,
    pub rules: &'a MoveRules,
    /// Pieces visible at a decision: current piece + Next window.
    pub visible: usize,
    pub hold: bool,
    memo: Memo,
    pub nodes: u64,
}

impl<'a> VisibleCheck<'a> {
    pub fn new(shape: &'a OpenerShape, rules: &'a MoveRules, visible: usize, hold: bool) -> Self {
        VisibleCheck {
            shape,
            rules,
            visible,
            hold,
            memo: HashMap::new(),
            nodes: 0,
        }
    }

    /// Is the shape guaranteed buildable from the start, whatever the hidden pieces are,
    /// given that `revealed` are the pieces shown so far?
    pub fn feasible(&mut self, revealed: &[Piece]) -> bool {
        self.rec(&Board::default(), 0, None, revealed.to_vec())
    }

    fn rec(
        &mut self,
        board: &Board,
        idx: usize,
        hold: Option<Piece>,
        revealed: Vec<Piece>,
    ) -> bool {
        // reveal pieces until the visible window is full: AND over every bag-consistent piece
        if revealed.len() < idx + self.visible {
            return bag_candidates(&revealed).into_iter().all(|p| {
                let mut r = revealed.clone();
                r.push(p);
                self.rec(board, idx, hold, r)
            });
        }
        let rem = remaining(board, &self.shape.target);
        if rem == 0 {
            return reserve_available(&revealed, idx, hold, self.shape.reserve);
        }
        let key = (board.clone(), idx, hold, revealed.clone());
        if let Some(&v) = self.memo.get(&key) {
            return v;
        }
        self.nodes += 1;
        let mut ok = false;
        'outer: for (piece, nidx, nhold, _) in piece_options(&revealed[..], idx, hold, self.hold) {
            if Some(piece) == self.shape.reserve {
                continue;
            }
            for pl in placements(board, piece, self.rules) {
                if !inside(&pl, &self.shape.target) {
                    continue;
                }
                let mut nb = board.clone();
                if lock(&mut nb, &pl).lines > 0 {
                    continue;
                }
                if self.rec(&nb, nidx, nhold, revealed.clone()) {
                    ok = true;
                    break 'outer;
                }
            }
        }
        self.memo.insert(key, ok);
        ok
    }
}

/// STEP 0 sample shape: a left-side T-Spin Double setup built from one bag minus the T.
///
/// Picked by `examples/shape_search.rs` (best sampled coverage among the 3 tileable
/// candidates of this family).
///
/// ```text
/// y3  . . . . . # . . . .
/// y2  # . . # # # # # # .     T slot at x=1..3, overhang at (3,2)
/// y1  # . . . # # # # # #
/// y0  # # . # # # # # # #
/// ```
pub fn sample_tsd_shape() -> OpenerShape {
    OpenerShape {
        id: "spike-tsd-left".into(),
        target: Board::from_ascii(&[".....#....", "#..######.", "#...######", "##.#######"]),
        reserve: Some(Piece::T),
    }
}

/// Every permutation of one bag.
pub fn all_bags() -> Vec<Vec<Piece>> {
    fn perm(rest: &mut Vec<Piece>, cur: &mut Vec<Piece>, out: &mut Vec<Vec<Piece>>) {
        if rest.is_empty() {
            out.push(cur.clone());
            return;
        }
        for i in 0..rest.len() {
            let p = rest.remove(i);
            cur.push(p);
            perm(rest, cur, out);
            cur.pop();
            rest.insert(i, p);
        }
    }
    let mut out = Vec::new();
    perm(
        &mut stacklab_engine::piece::ALL_PIECES.to_vec(),
        &mut Vec::new(),
        &mut out,
    );
    out
}

/// Used by tests: the T in state 2 at the slot.
pub fn sample_tsd_t_state() -> stacklab_engine::PieceState {
    stacklab_engine::PieceState {
        piece: Piece::T,
        rot: Rot::S,
        x: 2,
        y: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stacklab_engine::piece::parse_queue;
    use stacklab_engine::Ruleset;

    #[test]
    fn sample_shape_finisher_is_a_tsd() {
        let rules = MoveRules::from_ruleset(&Ruleset::tl_s2());
        assert!(sample_tsd_shape().finisher_ok(&rules, 2));
    }

    #[test]
    fn sample_shape_buildable_for_a_friendly_bag() {
        let rules = MoveRules::from_ruleset(&Ruleset::tl_s2());
        let shape = sample_tsd_shape();
        let ok = all_bags()
            .iter()
            .take(300)
            .any(|b| full_sequence_feasible(&shape, b, true, &rules));
        assert!(ok);
        assert!(!full_sequence_feasible(
            &shape,
            &parse_queue("TTTTTTT"),
            true,
            &rules
        ));
    }

    #[test]
    fn visible_never_exceeds_full() {
        let rules = MoveRules::from_ruleset(&Ruleset::tl_s2());
        let shape = sample_tsd_shape();
        for bag in all_bags().into_iter().step_by(503) {
            let full = full_sequence_feasible(&shape, &bag, true, &rules);
            let mut vc = VisibleCheck::new(&shape, &rules, 2, true);
            let vis = vc.feasible(&bag[..2]);
            assert!(
                !vis || full,
                "visible-info feasible must imply full-sequence feasible"
            );
        }
    }
}
