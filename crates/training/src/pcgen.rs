//! Module D: perfect clears (scope widened by user preference, D-019).
//!
//! Subtypes
//! * `pc2`     — 2-line PC on a partly filled 2-row field.
//! * `pc4-mid` — a 4-line PC already in progress (cut from a proven full solution).
//! * `pc4-n`   — full 4-line PC from an empty field at the n-th position of a continuous-PC
//!   run (n = 1..7: the bag offset shifts by 3 every PC; 10 pieces each).

use crate::gen_common::{assemble, initial, GameSim};
use crate::problem::{Goal, Module, Problem};
use stacklab_engine::board::{Board, FULL_ROW, WIDTH};
use stacklab_engine::rng::{Bag7, Rng};
use stacklab_engine::{lock, MoveRules, Piece, Ruleset};
use stacklab_solver::pc::{solve, PcConfig};
use stacklab_solver::Step;
use std::sync::Arc;

pub const GENERATOR_VERSION: &str = "D-pc-1";

/// D estimate: pieces needed, lines, hold use, nth PC.
pub fn estimate_difficulty(pieces: u32, lines: u32, hold_used: bool, nth: u32) -> f64 {
    1100.0
        + 70.0 * pieces as f64
        + if lines >= 4 { 100.0 } else { 0.0 }
        + if hold_used { 40.0 } else { 0.0 }
        + 15.0 * nth.saturating_sub(1) as f64
}

fn pieces_for(target_d: f64, lines: u32) -> u32 {
    let extra = if lines >= 4 { 100.0 } else { 0.0 };
    (((target_d - 1100.0 - extra) / 70.0).round() as i64).clamp(2, 10) as u32
}

fn witness(
    rs: &Arc<Ruleset>,
    seed: u64,
    init: &stacklab_engine::game::InitialState,
    steps: &[Step],
    rules: &MoveRules,
) -> Option<GameSim> {
    let mut sim = GameSim::new(rs.clone(), seed, init);
    for s in steps {
        sim = sim.play(&s.placement, s.used_hold, rules)?;
    }
    sim.events
        .iter()
        .any(|e| matches!(e, stacklab_engine::events::EventKind::PerfectClear))
        .then_some(sim)
}

pub fn generate(
    rs: &Arc<Ruleset>,
    rules: &MoveRules,
    target_d: f64,
    seed: u64,
    time_limit_secs: u32,
) -> Option<Problem> {
    let mut rng = Rng::new(seed);
    // below ~1450 use 2-line PCs, otherwise 4-line (in progress or from empty)
    let use_two = target_d < 1450.0 || (target_d < 1550.0 && rng.chance_pm(300));
    let (board, queue, hold, steps, lines, nth, tag) = if use_two {
        let n = pieces_for(target_d, 2).clamp(2, 5);
        two_line(rs, rules, n, &mut rng, seed)?
    } else {
        let n = pieces_for(target_d, 4).clamp(3, 10);
        four_line(rs, rules, n, &mut rng, seed)?
    };
    let pieces = steps.len() as u32;
    let mut init = initial(&board, queue);
    init.hold = hold;
    let sim = witness(rs, seed, &init, &steps, rules)?;
    let hold_used = steps.iter().any(|s| s.used_hold);
    let mut tags = vec![tag.clone(), format!("pieces:{pieces}")];
    if hold_used {
        tags.push("hold".into());
    }
    let d = estimate_difficulty(pieces, lines, hold_used, nth);
    Some(assemble(
        Module::D,
        tags,
        seed,
        rs,
        GENERATOR_VERSION,
        init,
        5,
        Goal::PerfectClear {
            within_pieces: pieces,
        },
        sim.witness(),
        d,
        format!("D:{tag}:n{pieces}:h{}", hold_used as u8),
        time_limit_secs,
    ))
}

type Built = (
    Board,
    Vec<Piece>,
    Option<Piece>,
    Vec<Step>,
    u32,
    u32,
    String,
);

fn two_line(
    rs: &Ruleset,
    rules: &MoveRules,
    pieces: u32,
    rng: &mut Rng,
    seed: u64,
) -> Option<Built> {
    use stacklab_engine::board::PieceState;
    use stacklab_engine::Rot;
    for attempt in 0..60u64 {
        // start from a full 2-row block and carve out `pieces` tetrominoes (top cells first)
        let mut b = Board::default();
        b.rows[0] = FULL_ROW;
        b.rows[1] = FULL_ROW;
        let mut removed = Vec::new();
        for _ in 0..pieces {
            let mut cands = Vec::new();
            for p in stacklab_engine::piece::ALL_PIECES {
                for r in 0..4 {
                    for x in -1..=WIDTH as i8 {
                        for y in -1..=2 {
                            let st = PieceState {
                                piece: p,
                                rot: Rot::from_index(r),
                                x,
                                y,
                            };
                            let cells = st.cells();
                            let ok = cells.iter().all(|&(cx, cy)| {
                                (0..WIDTH as i8).contains(&cx)
                                    && (0..2).contains(&cy)
                                    && b.occupied(cx, cy)
                            }) && cells.iter().all(|&(cx, cy)| {
                                !b.occupied(cx, cy + 1) || cells.contains(&(cx, cy + 1))
                            });
                            if ok {
                                cands.push(st);
                            }
                        }
                    }
                }
            }
            if cands.is_empty() {
                break;
            }
            let st = cands[rng.below(cands.len() as u64) as usize];
            for (cx, cy) in st.cells() {
                b.rows[cy as usize] &= !(1 << cx);
            }
            removed.push(st.piece);
        }
        if removed.len() as u32 != pieces || b.rows[0] == 0 && b.rows[1] == 0 {
            continue;
        }
        // queue: the carved pieces in a shuffled order + one spare; the solver proves a path
        let mut queue = removed.clone();
        for i in (1..queue.len()).rev() {
            let j = rng.below(i as u64 + 1) as usize;
            queue.swap(i, j);
        }
        let mut bag = Bag7::new(seed.wrapping_add(attempt * 7919), &rs.queue.bag_base_order);
        queue.push(bag.next_piece());
        let out = solve(
            &b,
            &queue,
            None,
            rules,
            &PcConfig {
                height: 2,
                hold: true,
                max_nodes: 200_000,
            },
        );
        if let Some(sol) = out.solution {
            if sol.len() as u32 == pieces {
                return Some((b, queue, None, sol, 2, 0, "pc2".into()));
            }
        }
    }
    None
}

fn four_line(
    rs: &Ruleset,
    rules: &MoveRules,
    pieces: u32,
    rng: &mut Rng,
    seed: u64,
) -> Option<Built> {
    for attempt in 0..6u64 {
        let nth = 1 + rng.below(7) as u32;
        let mut bag = Bag7::new(
            seed.wrapping_add(attempt * 104_729),
            &rs.queue.bag_base_order,
        );
        for _ in 0..10 * (nth - 1) {
            bag.next_piece();
        }
        let queue: Vec<Piece> = (0..11).map(|_| bag.next_piece()).collect();
        let out = solve(
            &Board::default(),
            &queue,
            None,
            rules,
            &PcConfig {
                height: 4,
                hold: true,
                max_nodes: 300_000,
            },
        );
        let Some(sol) = out.solution else { continue };
        // cut the proven solution so that `pieces` placements remain
        let cut = sol.len().saturating_sub(pieces as usize);
        let mut b = Board::default();
        let mut idx = 0usize;
        let mut hold: Option<Piece> = None;
        for s in &sol[..cut] {
            if s.used_hold {
                if hold.is_none() {
                    hold = Some(queue[idx]);
                    idx += 2;
                } else {
                    hold = Some(queue[idx]);
                    idx += 1;
                }
            } else {
                idx += 1;
            }
            lock(&mut b, &s.placement);
        }
        let rest_queue = queue[idx..].to_vec();
        let tag = if cut == 0 {
            format!("pc4-{nth}")
        } else {
            "pc4-mid".to_string()
        };
        return Some((b, rest_queue, hold, sol[cut..].to_vec(), 4, nth, tag));
    }
    None
}
