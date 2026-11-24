//! Module A: finesse. Optimal key-press cost over the input-state graph with the player's
//! handling (tap vs DAS), and the problem generator.

use crate::problem::{fingerprint, Goal, Module, Problem, TargetPlacement, Witness, WitnessStep};
use serde::{Deserialize, Serialize};
use stacklab_engine::board::{Board, PieceState};
use stacklab_engine::game::{Handling, InitialState, InputEvent, Key};
use stacklab_engine::rng::Rng;
use stacklab_engine::rotation::RotDir;
use stacklab_engine::{MoveRules, Piece, Ruleset};
use std::collections::{HashMap, VecDeque};

pub const GENERATOR_VERSION: &str = "A-finesse-1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FMove {
    TapLeft,
    TapRight,
    DasLeft,
    DasRight,
    Cw,
    Ccw,
    Flip,
}

fn apply(board: &Board, rules: &MoveRules, s: &PieceState, m: FMove) -> Option<PieceState> {
    let shift = |dx: i8| {
        let n = s.shifted(dx, 0);
        board.fits(&n).then_some(n)
    };
    let das = |dx: i8| {
        let mut cur = *s;
        let mut moved = false;
        while board.fits(&cur.shifted(dx, 0)) {
            cur = cur.shifted(dx, 0);
            moved = true;
        }
        moved.then_some(cur)
    };
    let rot = |d: RotDir| rules.kicks.try_rotate(board, s, d).map(|(n, _)| n);
    match m {
        FMove::TapLeft => shift(-1),
        FMove::TapRight => shift(1),
        FMove::DasLeft => das(-1),
        FMove::DasRight => das(1),
        FMove::Cw => rot(RotDir::Cw),
        FMove::Ccw => rot(RotDir::Ccw),
        FMove::Flip => rot(RotDir::Flip),
    }
}

/// For every hard-drop result reachable without soft drop: the minimum number of key presses
/// (hard drop included) and one optimal move list. Different paths with the same cost are all
/// optimal; judging compares cost only, so every equal-cost path is accepted.
/// Cells of a landed piece (sorted).
pub type CellKey = [(i8, i8); 4];

/// Optimal cost (key presses incl. hard drop), one optimal path, and the landed state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinesseOption {
    pub cost: u32,
    pub path: Vec<FMove>,
    pub landed: PieceState,
}

pub fn optimal_costs(
    board: &Board,
    piece: Piece,
    rules: &MoveRules,
    allow_180: bool,
) -> HashMap<CellKey, FinesseOption> {
    let mut moves = vec![
        FMove::TapLeft,
        FMove::TapRight,
        FMove::DasLeft,
        FMove::DasRight,
        FMove::Cw,
        FMove::Ccw,
    ];
    if allow_180 {
        moves.push(FMove::Flip);
    }
    let start = rules.spawn(piece);
    let mut out: HashMap<CellKey, FinesseOption> = HashMap::new();
    if !board.fits(&start) {
        return out;
    }
    let mut seen: HashMap<PieceState, Vec<FMove>> = HashMap::new();
    seen.insert(start, Vec::new());
    let mut q = VecDeque::from([start]);
    while let Some(s) = q.pop_front() {
        let path = seen[&s].clone();
        let landed = board.drop_y(&s);
        let key = landed.cell_key();
        let cost = path.len() as u32 + 1;
        out.entry(key).or_insert_with(|| FinesseOption {
            cost,
            path: path.clone(),
            landed,
        });
        if path.len() >= 6 {
            continue;
        }
        for &m in &moves {
            if let Some(n) = apply(board, rules, &s, m) {
                if let std::collections::hash_map::Entry::Vacant(e) = seen.entry(n) {
                    let mut p = path.clone();
                    p.push(m);
                    e.insert(p);
                    q.push_back(n);
                }
            }
        }
    }
    out
}

/// Key events for a finesse path starting at sub-frame `t`, for the given handling. DAS moves
/// hold the key until the auto shift has reached the wall.
pub fn path_to_inputs(path: &[FMove], handling: &Handling, mut t: u64) -> (Vec<InputEvent>, u64) {
    let mut v = Vec::new();
    let hold_time = handling.das as u64 + handling.arr as u64 * 10 + 2;
    for m in path {
        let (k, hold) = match m {
            FMove::TapLeft => (Key::Left, 1),
            FMove::TapRight => (Key::Right, 1),
            FMove::DasLeft => (Key::Left, hold_time),
            FMove::DasRight => (Key::Right, hold_time),
            FMove::Cw => (Key::Cw, 1),
            FMove::Ccw => (Key::Ccw, 1),
            FMove::Flip => (Key::Flip, 1),
        };
        v.push(InputEvent {
            time: t,
            key: k,
            down: true,
        });
        v.push(InputEvent {
            time: t + hold,
            key: k,
            down: false,
        });
        t += hold + 1;
    }
    v.push(InputEvent {
        time: t,
        key: Key::HardDrop,
        down: true,
    });
    v.push(InputEvent {
        time: t + 1,
        key: Key::HardDrop,
        down: false,
    });
    (v, t + 2)
}

/// Handling used for witnesses (fast, deterministic; DCD off).
pub fn witness_handling() -> Handling {
    Handling {
        das: 60,
        arr: 0,
        sdf: 0,
        dcd: 0,
    }
}

/// Time pressure levels: 0 = policy time limit, then fixed shorter limits (seconds).
pub const PRESSURE_LIMITS: [u32; 5] = [0, 10, 7, 5, 3];

/// Initial D estimate (spec 6.2 step 2): anchor = 3 key presses on a near-empty board with the
/// policy time limit. Factors: inputs, 180 needed, terrain, time pressure.
pub fn estimate_difficulty(
    optimal_inputs: u32,
    needs_180: bool,
    terrain_height: u32,
    pressure: u32,
) -> f64 {
    1500.0
        + 150.0 * (optimal_inputs as f64 - 3.0)
        + if needs_180 { 80.0 } else { 0.0 }
        + 15.0 * terrain_height as f64
        + 60.0 * pressure as f64
}

pub fn generate(
    rs: &Ruleset,
    rules: &MoveRules,
    target_d: f64,
    seed: u64,
    time_limit_secs: u32,
) -> Option<Problem> {
    let mut rng = Rng::new(seed);
    let allow_180 = true;
    // low random terrain (0..3 rows, no holes) so targets are not always on the floor
    let terrain_h = rng.below(4) as usize;
    let mut board = Board::default();
    for x in 0..10 {
        let h = rng.below(terrain_h as u64 + 1) as usize;
        for row in board.rows.iter_mut().take(h) {
            *row |= 1 << x;
        }
    }
    for row in board.rows.iter_mut() {
        if *row == stacklab_engine::board::FULL_ROW {
            *row &= !(1 << rng.below(10));
        }
    }
    let piece = stacklab_engine::piece::ALL_PIECES[rng.below(7) as usize];
    let costs = optimal_costs(&board, piece, rules, allow_180);
    let want = ((target_d - 1500.0) / 150.0).round() as i64 + 3;
    let mut best: Vec<(&CellKey, &FinesseOption)> = Vec::new();
    let mut best_gap = i64::MAX;
    for (k, v) in &costs {
        let gap = (v.cost as i64 - want).abs();
        if gap < best_gap {
            best_gap = gap;
            best.clear();
        }
        if gap == best_gap {
            best.push((k, v));
        }
    }
    if best.is_empty() {
        return None;
    }
    best.sort_by_key(|(k, _)| **k);
    let FinesseOption { cost, path, landed } = best[rng.below(best.len() as u64) as usize].1;
    let needs_180 = path.contains(&FMove::Flip) && {
        let without = optimal_costs(&board, piece, rules, false);
        without
            .get(&landed.cell_key())
            .is_none_or(|w| w.cost > *cost)
    };
    let handling = witness_handling();
    let (inputs, end) = path_to_inputs(path, &handling, 0);
    let target = TargetPlacement {
        piece,
        rot: landed.rot,
        x: landed.x,
        y: landed.y,
    };
    let initial = InitialState {
        rows: board.rows.to_vec(),
        queue: vec![piece],
        hold: None,
        b2b: -1,
        combo: -1,
        continue_with_bag: true,
        incoming: Vec::new(),
    };
    let goal = Goal::Finesse {
        target,
        optimal_inputs: *cost,
        allow_180,
    };
    let base = estimate_difficulty(*cost, needs_180, terrain_h as u32, 0);
    let pressure =
        (((target_d - base) / 60.0).round().max(0.0) as u32).min(PRESSURE_LIMITS.len() as u32 - 1);
    let time_limit_secs = if pressure == 0 {
        time_limit_secs
    } else {
        PRESSURE_LIMITS[pressure as usize].min(time_limit_secs)
    };
    let difficulty = estimate_difficulty(*cost, needs_180, terrain_h as u32, pressure);
    let fp = fingerprint(Module::A, &initial, &goal);
    Some(Problem {
        id: format!("A-{fp:016x}"),
        module: Module::A,
        subtype_tags: vec![
            format!("piece:{}", piece.to_char()),
            format!("cost:{cost}"),
            if needs_180 {
                "180".into()
            } else {
                "no180".into()
            },
            format!("pressure:{pressure}"),
        ],
        seed,
        ruleset_version: rs.meta.version.clone(),
        generator_version: GENERATOR_VERSION.into(),
        validation_version: crate::validator::VALIDATION_VERSION.into(),
        initial,
        visible_next: 5,
        goal,
        witness: Witness {
            handling,
            inputs,
            frames: end / 10 + 2,
            placements: vec![WitnessStep {
                target,
                use_hold: false,
            }],
        },
        difficulty,
        param_bucket: format!(
            "A:cost{cost}:{}:h{terrain_h}:t{pressure}",
            if needs_180 { "r180" } else { "n180" }
        ),
        time_limit_secs,
        fingerprint: fp,
    })
}
