//! Module G: defence & recovery. Garbage terrain (+ pending garbage) first; an engine-exact
//! beam search proves how many lines can be cleared while surviving.

use crate::gen_common::{assemble, beam_search, eval_board, initial, GameSim};
use crate::problem::{Goal, Module, Problem};
use stacklab_engine::board::{Board, FULL_ROW, WIDTH};
use stacklab_engine::rng::{Bag7, Rng};
use stacklab_engine::{MoveRules, Piece, Ruleset};
use std::sync::Arc;

pub const GENERATOR_VERSION: &str = "G-defense-1";

pub fn estimate_difficulty(
    garbage_rows: u32,
    messy: bool,
    incoming: u32,
    lines: u32,
    pieces: u32,
) -> f64 {
    1150.0
        + 30.0 * garbage_rows as f64
        + if messy { 100.0 } else { 0.0 }
        + 25.0 * incoming as f64
        + 35.0 * lines as f64
        - 12.0 * (pieces as f64 - 8.0)
}

pub fn generate(
    rs: &Arc<Ruleset>,
    rules: &MoveRules,
    target_d: f64,
    seed: u64,
    time_limit_secs: u32,
) -> Option<Problem> {
    let mut rng = Rng::new(seed);
    let rows = 3 + rng.below(7) as u32;
    let messy = rng.chance_pm(400);
    let incoming = [0u32, 0, 2, 4, 6][rng.below(5) as usize];
    let pieces = 6 + rng.below(7) as u32;
    let mut b = Board::default();
    let mut hole = rng.below(WIDTH as u64) as usize;
    for row in b.rows.iter_mut().take(rows as usize) {
        if messy && rng.chance_pm(500) || !messy && rng.chance_pm(150) {
            hole = (hole + 1 + rng.below(WIDTH as u64 - 1) as usize) % WIDTH;
        }
        *row = FULL_ROW & !(1 << hole);
    }
    // a little residue on top
    for row in b.rows.iter_mut().skip(rows as usize).take(2) {
        for x in 0..WIDTH {
            if rng.chance_pm(300) {
                *row |= 1 << x;
            }
        }
        if *row == FULL_ROW {
            *row &= !1;
        }
    }
    let mut bag = Bag7::new(seed ^ 0xde4, &rs.queue.bag_base_order);
    let queue: Vec<Piece> = (0..=pieces).map(|_| bag.next_piece()).collect();
    let mut init = initial(&b, queue);
    if incoming > 0 {
        init.incoming = vec![incoming];
    }
    let start = GameSim::new(rs.clone(), seed, &init);
    let score = |s: &GameSim| {
        if s.game.is_over() {
            return i64::MIN / 2;
        }
        s.game.stats().lines as i64 * 80 + eval_board(s.game.board()) as i64
    };
    let best = beam_search(&start, pieces, 8, 10, rules, &score)
        .into_iter()
        .filter(|s| !s.game.is_over())
        .max_by_key(|s| s.game.stats().lines)?;
    let achieved = best.game.stats().lines;
    if achieved == 0 {
        return None;
    }
    let target = (1..=achieved)
        .min_by_key(|&l| {
            (estimate_difficulty(rows, messy, incoming, l, pieces) - target_d).abs() as i64
        })
        .expect("achieved >= 1");
    let mut tags = vec![
        if messy {
            "messy".to_string()
        } else {
            "clean".to_string()
        },
        format!("garbage:{rows}"),
    ];
    if incoming > 0 {
        tags.push("incoming".into());
    }
    let d = estimate_difficulty(rows, messy, incoming, target, pieces);
    Some(assemble(
        Module::G,
        tags,
        seed,
        rs,
        GENERATOR_VERSION,
        init,
        5,
        Goal::Downstack {
            pieces,
            min_lines: target,
        },
        best.witness(),
        d,
        format!("G:r{rows}:m{}:i{incoming}:l{target}", messy as u8),
        time_limit_secs,
    ))
}
