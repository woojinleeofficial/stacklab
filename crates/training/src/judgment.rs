//! Module H: quick judgment. Several placements may be correct (multi-answer); when the
//! evaluation gap is small the problem is marked unrated (spec 4, H).

use crate::gen_common::{assemble, eval_board, initial, random_terrain, GameSim};
use crate::problem::{Goal, Module, Problem, TargetPlacement};
use stacklab_engine::board::Board;
use stacklab_engine::rng::{Bag7, Rng};
use stacklab_engine::{lock, placements, MoveRules, Piece, Placement, Ruleset};
use std::sync::Arc;

pub const GENERATOR_VERSION: &str = "H-judgment-1";
/// Placements within this score of the best are all accepted.
pub const ACCEPT_MARGIN: i32 = 8;
/// Minimum score gap to the best rejected placement for the problem to be rated.
pub const RATED_GAP: i32 = 40;

pub fn estimate_difficulty(gap: i32, visible: u8) -> f64 {
    1750.0 - 1.5 * gap.min(250) as f64 - 25.0 * (visible as f64 - 3.0)
}

/// Value of a placement: board after it + best follow-up over the next pieces (lookahead).
fn value(board: &Board, pl: &Placement, next: &[Piece], rules: &MoveRules) -> i32 {
    let mut b = board.clone();
    let lines = lock(&mut b, pl).lines as i32;
    let mut best = eval_board(&b) + lines * 20;
    if let Some((&n, rest)) = next.split_first() {
        best = placements(&b, n, rules)
            .iter()
            .map(|p2| {
                let mut bb = b.clone();
                let l2 = lock(&mut bb, p2).lines as i32;
                let follow = rest.first().map_or(eval_board(&bb), |&n3| {
                    placements(&bb, n3, rules)
                        .iter()
                        .map(|p3| {
                            let mut b3 = bb.clone();
                            lock(&mut b3, p3);
                            eval_board(&b3)
                        })
                        .max()
                        .unwrap_or(-10_000)
                });
                follow + (lines + l2) * 20
            })
            .max()
            .unwrap_or(best);
    }
    best
}

pub fn generate(
    rs: &Arc<Ruleset>,
    rules: &MoveRules,
    target_d: f64,
    seed: u64,
    time_limit_secs: u32,
) -> Option<Problem> {
    let mut rng = Rng::new(seed);
    let board = random_terrain(3 + rng.below(6) as u32, None, &mut rng);
    let visible = 1 + rng.below(3) as u8;
    let mut bag = Bag7::new(seed ^ 0x4d, &rs.queue.bag_base_order);
    let queue: Vec<Piece> = (0..4).map(|_| bag.next_piece()).collect();
    let cur = queue[0];
    let lookahead = &queue[1..1 + visible.min(2) as usize];
    let mut scored: Vec<(i32, Placement)> = placements(&board, cur, rules)
        .into_iter()
        .map(|pl| (value(&board, &pl, lookahead, rules), pl))
        .collect();
    scored.sort_by_key(|s| std::cmp::Reverse(s.0));
    let best = scored.first()?.0;
    let accepted: Vec<&(i32, Placement)> = scored
        .iter()
        .filter(|s| s.0 >= best - ACCEPT_MARGIN)
        .collect();
    let gap = scored
        .iter()
        .find(|s| s.0 < best - ACCEPT_MARGIN)
        .map_or(250, |s| best - s.0);
    let rated = gap >= RATED_GAP && accepted.len() <= 3;
    let d = estimate_difficulty(gap, visible);
    // a far-off request is still served (difficulty is recorded honestly); try a few seeds
    // for a closer match first
    if (d - target_d).abs() > 250.0 && seed % 4 != 3 {
        return None;
    }
    let accepted_t: Vec<TargetPlacement> = accepted
        .iter()
        .map(|(_, p)| TargetPlacement {
            piece: p.state.piece,
            rot: p.state.rot,
            x: p.state.x,
            y: p.state.y,
        })
        .collect();
    let init = initial(&board, queue);
    let sim = GameSim::new(rs.clone(), seed, &init).play(&accepted[0].1, false, rules)?;
    let mut tags = vec![
        format!("options:{}", accepted_t.len()),
        format!("visible:{visible}"),
    ];
    if !rated {
        tags.push("unrated".into());
    }
    Some(assemble(
        Module::H,
        tags,
        seed,
        rs,
        GENERATOR_VERSION,
        init,
        visible,
        Goal::Placement {
            accepted: accepted_t,
            rated,
        },
        sim.witness(),
        d,
        format!("H:v{visible}:g{}", (gap / 50).min(5)),
        time_limit_secs,
    ))
}
