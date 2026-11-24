//! Search utilities shared by the problem validator and opener verification.
//!
//! * [`pc`]: perfect-clear search with hold (algorithm idea after solution-finder: bounded
//!   height, cell-parity and region pruning; no code from that project is used).
//! * [`fill`]: "fill exactly this target shape" search used for opener setups.

pub mod fill;
pub mod pc;

use stacklab_engine::Piece;

/// One decision in a solution: which piece was placed and whether hold was pressed first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    pub used_hold: bool,
    pub placement: stacklab_engine::Placement,
}

/// Piece-choice options at a queue position with hold. Returns (piece, next_idx, next_hold, used_hold).
pub fn piece_options(
    queue: &[Piece],
    idx: usize,
    hold: Option<Piece>,
    hold_enabled: bool,
) -> Vec<(Piece, usize, Option<Piece>, bool)> {
    let mut out = Vec::with_capacity(2);
    let Some(&cur) = queue.get(idx) else {
        // queue exhausted: only the held piece can still be played
        if let Some(h) = hold {
            if hold_enabled {
                out.push((h, idx, None, true));
            }
        }
        return out;
    };
    out.push((cur, idx + 1, hold, false));
    if hold_enabled {
        match hold {
            Some(h) if h != cur => out.push((h, idx + 1, Some(cur), true)),
            Some(_) => {}
            None => {
                if let Some(&nxt) = queue.get(idx + 1) {
                    out.push((nxt, idx + 2, Some(cur), true));
                }
            }
        }
    }
    out
}
