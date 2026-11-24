//! Module B: stacking & hold. Conditions and queue first, then a seeded beam search proves a
//! sequence that satisfies them (spec 5.2).

use crate::problem::{fingerprint, Goal, Module, Problem, TargetPlacement, Witness, WitnessStep};
use crate::session::count_holes;
use stacklab_engine::board::{Board, FULL_ROW, WIDTH};
use stacklab_engine::game::{path_inputs, Handling, InitialState, InputEvent};
use stacklab_engine::movegen::find_path;
use stacklab_engine::rng::{Bag7, Rng};
use stacklab_engine::{lock, placements, MoveRules, Piece, Placement, Ruleset};

pub const GENERATOR_VERSION: &str = "B-stacking-1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StackParams {
    pub pieces: u32,
    pub visible_next: u8,
    pub start_height: u32,
    /// None = free stacking, Some(col) = keep that column empty (9-0 / 6-3 / 7-2 shapes).
    pub well: Option<u8>,
    /// Holes allowed at the end (easier problems allow one).
    pub holes_allowed: u32,
}

/// Initial D estimate: anchor = 8 pieces, Next 5, start height 5, no well constraint.
pub fn estimate_difficulty(p: &StackParams) -> f64 {
    1500.0 + 25.0 * (p.pieces as f64 - 8.0) - 40.0 * (p.visible_next as f64 - 5.0)
        + 30.0 * (p.start_height as f64 - 5.0)
        + if p.well.is_some() { 60.0 } else { 0.0 }
        - 80.0 * p.holes_allowed as f64
}

pub fn bucket(p: &StackParams) -> String {
    let shape = match p.well {
        None => "free".to_string(),
        Some(0) | Some(9) => "9-0".to_string(),
        Some(3) | Some(6) => "6-3".to_string(),
        Some(2) | Some(7) => "7-2".to_string(),
        Some(w) => format!("well{w}"),
    };
    format!(
        "B:n{}:p{}:h{}:o{}:{shape}",
        p.pieces, p.visible_next, p.start_height, p.holes_allowed
    )
}

/// Pick parameters whose estimated D is closest to the target: exhaustive grid, random
/// choice among the closest ones (spec 6.2 difficulty spread comes from the jitter upstream).
pub fn params_for(target_d: f64, rng: &mut Rng) -> StackParams {
    let wells = [None, Some(0u8), Some(9), Some(3), Some(6), Some(2), Some(7)];
    let mut all = Vec::new();
    for pieces in 3..=14 {
        for visible_next in 1..=5u8 {
            for start_height in 2..=8 {
                for &well in &wells {
                    for holes_allowed in 0..=1 {
                        all.push(StackParams {
                            pieces,
                            visible_next,
                            start_height,
                            well,
                            holes_allowed,
                        });
                    }
                }
            }
        }
    }
    let gap = |p: &StackParams| (estimate_difficulty(p) - target_d).abs();
    let min = all.iter().map(gap).fold(f64::MAX, f64::min);
    let near: Vec<StackParams> = all.into_iter().filter(|p| gap(p) <= min + 10.0).collect();
    near[rng.below(near.len() as u64) as usize]
}

fn terrain(p: &StackParams, rng: &mut Rng) -> Board {
    let mut heights = [0i64; WIDTH];
    let mut h = p.start_height as i64;
    for (x, hx) in heights.iter_mut().enumerate() {
        if Some(x as u8) == p.well {
            *hx = 0;
            continue;
        }
        h = (h + rng.below(3) as i64 - 1)
            .clamp(p.start_height as i64 - 2, p.start_height as i64 + 1)
            .max(0);
        *hx = h;
    }
    let mut b = Board::default();
    for (x, &hx) in heights.iter().enumerate() {
        for row in b.rows.iter_mut().take(hx as usize) {
            *row |= 1 << x;
        }
    }
    // no row may start full
    for y in 0..40 {
        if b.rows[y] == FULL_ROW {
            let x = rng.below(WIDTH as u64) as usize;
            for row in b.rows.iter_mut().skip(y) {
                *row &= !(1 << x);
            }
        }
    }
    b
}

fn score(b: &Board, well: Option<u8>) -> i32 {
    let mut s = -(count_holes(b) as i32) * 120;
    let mut prev: Option<i32> = None;
    let mut max_h = 0;
    for x in 0..WIDTH {
        let h = b.column_height(x) as i32;
        max_h = max_h.max(h);
        if Some(x as u8) == well {
            continue;
        }
        if let Some(ph) = prev {
            s -= (h - ph).abs() * 6;
        }
        prev = Some(h);
    }
    s - max_h * 10
}

#[derive(Clone)]
struct Node {
    board: Board,
    hold: Option<Piece>,
    idx: usize,
    steps: Vec<(Placement, bool)>,
    score: i32,
}

/// Beam search for a sequence of placements meeting the goal. Returns placements + hold flags.
pub fn solve(
    board: &Board,
    queue: &[Piece],
    goal: &Goal,
    rules: &MoveRules,
    beam: usize,
) -> Option<Vec<(Placement, bool)>> {
    let Goal::Stacking {
        pieces,
        max_holes,
        max_height,
        well,
    } = *goal
    else {
        return None;
    };
    let mut frontier = vec![Node {
        board: board.clone(),
        hold: None,
        idx: 0,
        steps: Vec::new(),
        score: 0,
    }];
    for _ in 0..pieces {
        let mut next: Vec<Node> = Vec::new();
        for n in &frontier {
            let Some(&cur) = queue.get(n.idx) else {
                continue;
            };
            let mut opts = vec![(cur, false, n.hold, n.idx + 1)];
            match n.hold {
                Some(h) if h != cur => opts.push((h, true, Some(cur), n.idx + 1)),
                Some(_) => {}
                None => {
                    if let Some(&nx) = queue.get(n.idx + 1) {
                        opts.push((nx, true, Some(cur), n.idx + 2));
                    }
                }
            }
            for (piece, used_hold, nhold, nidx) in opts {
                for pl in placements(&n.board, piece, rules) {
                    if let Some(w) = well {
                        if pl.state.cells().iter().any(|c| c.0 == w as i8) {
                            continue;
                        }
                    }
                    let mut nb = n.board.clone();
                    lock(&mut nb, &pl);
                    let mut steps = n.steps.clone();
                    steps.push((pl, used_hold));
                    next.push(Node {
                        score: score(&nb, well),
                        board: nb,
                        hold: nhold,
                        idx: nidx,
                        steps,
                    });
                }
            }
        }
        next.sort_by_key(|n| std::cmp::Reverse(n.score));
        next.dedup_by(|a, b| a.board == b.board && a.hold == b.hold && a.idx == b.idx);
        next.truncate(beam);
        if next.is_empty() {
            return None;
        }
        frontier = next;
    }
    frontier.into_iter().find_map(|n| {
        let ok = count_holes(&n.board) <= max_holes
            && n.board.height() as u32 <= max_height
            && well.is_none_or(|w| n.board.column_height(w as usize) == 0);
        ok.then_some(n.steps)
    })
}

/// Converts placements into real key inputs, re-checking reachability on the evolving board.
pub fn witness_inputs(
    board: &Board,
    steps: &[(Placement, bool)],
    rules: &MoveRules,
) -> Option<(Vec<InputEvent>, u64)> {
    let mut b = board.clone();
    let mut t = 0;
    let mut out = Vec::new();
    for (pl, hold) in steps {
        let path = find_path(&b, pl, rules)?;
        let (inp, end) = path_inputs(&path, *hold, t);
        out.extend(inp);
        t = end + 2;
        lock(&mut b, pl);
    }
    Some((out, t))
}

pub fn witness_handling() -> Handling {
    Handling {
        das: 60,
        arr: 0,
        sdf: 0,
        dcd: 0,
    }
}

pub fn generate(
    rs: &Ruleset,
    rules: &MoveRules,
    target_d: f64,
    seed: u64,
    time_limit_secs: u32,
) -> Option<Problem> {
    let mut rng = Rng::new(seed);
    let p = params_for(target_d, &mut rng);
    let board = terrain(&p, &mut rng);
    let mut bag = Bag7::new(seed ^ 0xb1b2, &rs.queue.bag_base_order);
    // one spare piece: with hold, one piece always stays in the hold slot at the end
    let queue: Vec<Piece> = (0..=p.pieces).map(|_| bag.next_piece()).collect();
    let grow = (4 * p.pieces).div_ceil(WIDTH as u32 - u32::from(p.well.is_some()));
    let wanted_height = (board.height() as u32 + grow + 2).min(18);
    // solve the hole/well conditions first; the height limit is never tighter than a proven
    // solution's final height (so every served problem is solvable)
    let loose = Goal::Stacking {
        pieces: p.pieces,
        max_holes: p.holes_allowed,
        max_height: 18,
        well: p.well,
    };
    let steps = solve(&board, &queue, &loose, rules, 48)
        .or_else(|| solve(&board, &queue, &loose, rules, 160))?;
    let mut end_board = board.clone();
    for (pl, _) in &steps {
        lock(&mut end_board, pl);
    }
    let goal = Goal::Stacking {
        pieces: p.pieces,
        max_holes: p.holes_allowed,
        max_height: wanted_height.max(end_board.height() as u32),
        well: p.well,
    };
    let (inputs, end) = witness_inputs(&board, &steps, rules)?;
    let initial = InitialState {
        rows: board.rows.to_vec(),
        queue: queue.clone(),
        hold: None,
        b2b: -1,
        combo: -1,
        continue_with_bag: false,
    };
    let fp = fingerprint(Module::B, &initial, &goal);
    let mut tags = vec![bucket(&p)
        .split(':')
        .next_back()
        .unwrap_or("free")
        .to_string()];
    if steps.iter().any(|s| s.1) {
        tags.push("hold".into());
    }
    Some(Problem {
        id: format!("B-{fp:016x}"),
        module: Module::B,
        subtype_tags: tags,
        seed,
        ruleset_version: rs.meta.version.clone(),
        generator_version: GENERATOR_VERSION.into(),
        validation_version: crate::validator::VALIDATION_VERSION.into(),
        initial,
        visible_next: p.visible_next,
        goal,
        witness: Witness {
            handling: witness_handling(),
            inputs,
            frames: end / 10 + 2,
            placements: steps
                .iter()
                .map(|(pl, h)| WitnessStep {
                    target: TargetPlacement {
                        piece: pl.state.piece,
                        rot: pl.state.rot,
                        x: pl.state.x,
                        y: pl.state.y,
                    },
                    use_hold: *h,
                })
                .collect(),
        },
        difficulty: estimate_difficulty(&p),
        param_bucket: bucket(&p),
        time_limit_secs,
        fingerprint: fp,
    })
}
