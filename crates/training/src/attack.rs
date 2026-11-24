//! Module F: attack (quads, B2B charging, Surge, combos). Terrain + queue first, then an
//! engine-exact beam search finds the strongest line; the target is a share of it.

use crate::gen_common::{assemble, beam_search, eval_board, initial, random_terrain, GameSim};
use crate::problem::{Goal, Module, Problem};
use stacklab_engine::rng::{Bag7, Rng};
use stacklab_engine::{MoveRules, Piece, Ruleset};
use std::sync::Arc;

pub const GENERATOR_VERSION: &str = "F-attack-1";

pub fn estimate_difficulty(target: u32, pieces: u32, start_b2b: i32) -> f64 {
    1200.0 + 45.0 * target as f64 - 15.0 * (pieces as f64 - 8.0)
        + if start_b2b >= 3 { 80.0 } else { 0.0 }
}

pub fn generate(
    rs: &Arc<Ruleset>,
    rules: &MoveRules,
    target_d: f64,
    seed: u64,
    time_limit_secs: u32,
) -> Option<Problem> {
    let mut rng = Rng::new(seed);
    // parameter ranges follow the requested difficulty (fewer pieces / Surge management = harder)
    let (pieces, start_b2b) = if target_d >= 1650.0 {
        (6 + rng.below(3) as u32, [3, 5, 7][rng.below(3) as usize])
    } else if target_d >= 1400.0 {
        (7 + rng.below(4) as u32, [-1, 1, 3][rng.below(3) as usize])
    } else {
        (9 + rng.below(4) as u32, -1)
    };
    let well = Some(if rng.chance_pm(500) { 0 } else { 9 });
    let board = random_terrain(4 + rng.below(4) as u32, well, &mut rng);
    let mut bag = Bag7::new(seed ^ 0xa77, &rs.queue.bag_base_order);
    let queue: Vec<Piece> = (0..=pieces).map(|_| bag.next_piece()).collect();
    let mut init = initial(&board, queue);
    init.b2b = start_b2b;
    let start = GameSim::new(rs.clone(), seed, &init);
    let score = |s: &GameSim| {
        s.game.stats().attack as i64 * 120
            + s.game.chain().b2b.max(0) as i64 * 30
            + eval_board(s.game.board()) as i64
    };
    let best = beam_search(&start, pieces, 8, 10, rules, &score)
        .into_iter()
        .max_by_key(|s| s.game.stats().attack)?;
    let achieved = best.game.stats().attack;
    if achieved == 0 {
        return None;
    }
    // pick the share of the best line whose difficulty matches the request
    let target = (1..=achieved)
        .min_by_key(|&t| (estimate_difficulty(t, pieces, start_b2b) - target_d).abs() as i64)
        .expect("achieved >= 1");
    // shortest prefix of the witness reaching the target
    let used = best.steps.len().min(pieces as usize);
    let mut tags = vec![format!("attack:{target}")];
    if start_b2b >= 3 {
        tags.push("surge".into());
    } else if start_b2b >= 0 {
        tags.push("b2b".into());
    }
    if best
        .events
        .iter()
        .any(|e| matches!(e, stacklab_engine::events::EventKind::Spin { .. }))
    {
        tags.push("spin".into());
    }
    let d = estimate_difficulty(target, pieces, start_b2b);
    Some(assemble(
        Module::F,
        tags,
        seed,
        rs,
        GENERATOR_VERSION,
        init,
        5,
        Goal::Attack {
            min_attack: target,
            within_pieces: used as u32,
        },
        best.witness(),
        d,
        format!("F:n{pieces}:t{target}:b{}", start_b2b.max(-1)),
        time_limit_secs,
    ))
}
