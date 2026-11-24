//! Module C: spins. Build the spin situation first (slot + overhang, verified reachable with the
//! required engine classification), then extend backwards by removing prep pieces (spec 5.2).

use crate::gen_common::{assemble, initial, GameSim};
use crate::problem::{Goal, Module, Problem};
use stacklab_engine::board::{Board, PieceState, FULL_ROW, WIDTH};
use stacklab_engine::rng::{Bag7, Rng};
use stacklab_engine::{lock, placements, MoveRules, Piece, Rot, Ruleset, Spin};
use std::sync::Arc;

pub const GENERATOR_VERSION: &str = "C-spin-1";

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpinKind {
    pub tag: &'static str,
    pub pieces: &'static [Piece],
    pub spin: Spin,
    pub lines: u32,
    pub base_d: f64,
}

pub const KINDS: [SpinKind; 5] = [
    SpinKind {
        tag: "tss",
        pieces: &[Piece::T],
        spin: Spin::Full,
        lines: 1,
        base_d: 1400.0,
    },
    SpinKind {
        tag: "tsd",
        pieces: &[Piece::T],
        spin: Spin::Full,
        lines: 2,
        base_d: 1450.0,
    },
    SpinKind {
        tag: "tsm",
        pieces: &[Piece::T],
        spin: Spin::Mini,
        lines: 1,
        base_d: 1500.0,
    },
    SpinKind {
        tag: "allspin",
        pieces: &[Piece::S, Piece::Z, Piece::L, Piece::J, Piece::I],
        spin: Spin::Mini,
        lines: 1,
        base_d: 1600.0,
    },
    SpinKind {
        tag: "tst",
        pieces: &[Piece::T],
        spin: Spin::Full,
        lines: 3,
        base_d: 1650.0,
    },
];

pub fn estimate_difficulty(kind: &SpinKind, prep: u32, hold: bool) -> f64 {
    kind.base_d + 120.0 * prep as f64 + if hold { 50.0 } else { 0.0 }
}

/// Random board where `piece` can spin into `target` with the required class and lines.
fn spin_board(
    kind: &SpinKind,
    piece: Piece,
    rng: &mut Rng,
    rules: &MoveRules,
) -> Option<(Board, PieceState)> {
    let rot = Rot::from_index(rng.below(4) as usize);
    let x = 1 + rng.below(8) as i8;
    let base = rng.below(4) as i8;
    let mut s = PieceState {
        piece,
        rot,
        x,
        y: 0,
    };
    let min_y = s.cells().iter().map(|c| c.1).min().unwrap();
    s.y = base - min_y;
    let cells = s.cells();
    if cells.iter().any(|c| c.0 < 0 || c.0 >= WIDTH as i8) {
        return None;
    }
    let mut rows: Vec<i8> = cells.iter().map(|c| c.1).collect();
    rows.sort();
    rows.dedup();
    if (rows.len() as u32) < kind.lines {
        return None;
    }
    let mut b = Board::default();
    // garbage-like rows below the piece (one hole each)
    for y in 0..base {
        b.rows[y as usize] = FULL_ROW & !(1 << rng.below(WIDTH as u64));
    }
    // `lines` of the touched rows become full when the piece locks, the rest stay incomplete
    let mut full_rows = rows.clone();
    while full_rows.len() as u32 > kind.lines {
        full_rows.remove(rng.below(full_rows.len() as u64) as usize);
    }
    let piece_mask = |y: i8| {
        cells
            .iter()
            .filter(|c| c.1 == y)
            .fold(0u16, |m, c| m | 1 << c.0)
    };
    for &y in &rows {
        let pm = piece_mask(y);
        let row = if full_rows.contains(&y) {
            FULL_ROW & !pm
        } else {
            let mut r = 0;
            for xx in 0..WIDTH {
                if pm >> xx & 1 == 0 && rng.chance_pm(700) {
                    r |= 1 << xx;
                }
            }
            // never complete without the piece's help
            if r | pm == FULL_ROW {
                r &= !(1 << rng.below(WIDTH as u64));
                r &= !pm;
            }
            r
        };
        b.rows[y as usize] = row;
    }
    // overhangs: a couple of rows above with random cells
    let top = *rows.last().unwrap();
    for dy in 1..=2 {
        let y = (top + dy) as usize;
        let mut r = 0u16;
        for xx in 0..WIDTH {
            if rng.chance_pm(if dy == 1 { 450 } else { 250 }) {
                r |= 1 << xx;
            }
        }
        if r == FULL_ROW {
            r &= !(1 << rng.below(WIDTH as u64));
        }
        b.rows[y] = r;
    }
    let ok = placements(&b, piece, rules).into_iter().any(|p| {
        if p.state.cell_key() != s.cell_key() || p.spin != kind.spin {
            return false;
        }
        let mut bb = b.clone();
        lock(&mut bb, &p).lines == kind.lines
    });
    ok.then_some((b, s))
}

/// Removes one reachable prep piece from `b`: returns the smaller board and the piece/state.
/// Candidates are enumerated exhaustively (top-of-stack tetromino shapes), then tried in a
/// seeded random order.
fn remove_prep(
    b: &Board,
    avoid: &[(i8, i8)],
    rng: &mut Rng,
    rules: &MoveRules,
) -> Option<(Board, PieceState)> {
    let mut cands = Vec::new();
    let top = b.height() as i8;
    for p in stacklab_engine::piece::ALL_PIECES {
        for r in 0..4 {
            for x in -1..=WIDTH as i8 {
                for y in 0..=top {
                    let s = PieceState {
                        piece: p,
                        rot: Rot::from_index(r),
                        x,
                        y,
                    };
                    let cells = s.cells();
                    let ok = cells.iter().all(|&(cx, cy)| {
                        (0..WIDTH as i8).contains(&cx)
                            && cy >= 0
                            && b.occupied(cx, cy)
                            && !avoid.contains(&(cx, cy))
                    }) && cells
                        .iter()
                        .all(|&(cx, cy)| !b.occupied(cx, cy + 1) || cells.contains(&(cx, cy + 1)));
                    if ok {
                        cands.push(s);
                    }
                }
            }
        }
    }
    // seeded shuffle
    for i in (1..cands.len()).rev() {
        let j = rng.below(i as u64 + 1) as usize;
        cands.swap(i, j);
    }
    for s in cands {
        let mut smaller = b.clone();
        for &(cx, cy) in &s.cells() {
            smaller.rows[cy as usize] &= !(1 << cx);
        }
        let reachable = placements(&smaller, s.piece, rules).into_iter().any(|pl| {
            if pl.state.cell_key() != s.cell_key() {
                return false;
            }
            let mut bb = smaller.clone();
            lock(&mut bb, &pl).lines == 0 && bb == *b
        });
        if reachable {
            return Some((smaller, s));
        }
    }
    None
}

pub fn generate(
    rs: &Arc<Ruleset>,
    rules: &MoveRules,
    target_d: f64,
    seed: u64,
    time_limit_secs: u32,
) -> Option<Problem> {
    let mut rng = Rng::new(seed);
    // choose kind / prep / hold closest to the target difficulty
    let mut best = None;
    let mut best_gap = f64::MAX;
    for _ in 0..30 {
        let k = KINDS[rng.below(KINDS.len() as u64) as usize];
        let prep = rng.below(3) as u32;
        let hold = rng.chance_pm(300);
        let gap = (estimate_difficulty(&k, prep, hold) - target_d).abs();
        if gap < best_gap {
            best_gap = gap;
            best = Some((k, prep, hold));
        }
    }
    let (kind, prep, hold) = best?;
    let piece = kind.pieces[rng.below(kind.pieces.len() as u64) as usize];
    let (board, spin_state) = (0..4000).find_map(|_| spin_board(&kind, piece, &mut rng, rules))?;
    let mut start = board;
    let mut prep_states = Vec::new();
    for _ in 0..prep {
        let (smaller, s) = remove_prep(&start, &spin_state.cells(), &mut rng, rules)?;
        start = smaller;
        prep_states.push(s);
    }
    prep_states.reverse();
    let preps: Vec<Piece> = prep_states.iter().map(|s| s.piece).collect();
    // queue: [preps..., spin piece] (+ spin piece first when hold is required) + filler
    let mut queue = Vec::new();
    if hold && prep > 0 {
        queue.push(piece);
        queue.extend(preps.iter().copied());
    } else {
        queue.extend(preps.iter().copied());
        queue.push(piece);
    }
    let mut bag = Bag7::new(seed ^ 0x5eed, &rs.queue.bag_base_order);
    for _ in 0..3 {
        queue.push(bag.next_piece());
    }
    let hold = hold && prep > 0;
    let goal = Goal::Spin {
        piece: if kind.tag == "allspin" {
            None
        } else {
            Some(piece)
        },
        spin: kind.spin,
        lines: kind.lines,
        within_pieces: prep + 1,
    };
    let init = initial(&start, queue);
    // witness: replay the known construction (preps, then the spin); search as a fallback
    let sim = GameSim::new(rs.clone(), seed, &init);
    let witness = planned(&sim, &prep_states, spin_state, kind.spin, hold, rules)
        .or_else(|| search(&sim, &goal, prep + 1, rules))?;
    let mut tags = vec![
        kind.tag.to_string(),
        format!("piece:{}", piece.to_char()),
        format!("prep:{prep}"),
    ];
    if witness.steps.iter().any(|s| s.use_hold) {
        tags.push("hold".into());
    }
    let d = estimate_difficulty(&kind, prep, hold);
    Some(assemble(
        Module::C,
        tags,
        seed,
        rs,
        GENERATOR_VERSION,
        init,
        5,
        goal,
        witness.witness(),
        d,
        format!("C:{}:p{prep}:h{}", kind.tag, hold as u8),
        time_limit_secs,
    ))
}

fn planned(
    sim: &GameSim,
    preps: &[PieceState],
    spin_state: PieceState,
    spin: Spin,
    hold: bool,
    rules: &MoveRules,
) -> Option<GameSim> {
    let mut cur = sim.clone();
    for (i, s) in preps.iter().enumerate() {
        let pl = placements(cur.game.board(), s.piece, rules)
            .into_iter()
            .find(|p| p.state.cell_key() == s.cell_key())?;
        cur = cur.play(&pl, hold && i == 0, rules)?;
    }
    let pl = placements(cur.game.board(), spin_state.piece, rules)
        .into_iter()
        .find(|p| p.state.cell_key() == spin_state.cell_key() && p.spin == spin)?;
    cur.play(&pl, hold, rules)
}

/// Depth-limited search with the real engine for a play that achieves a spin goal.
pub fn search(sim: &GameSim, goal: &Goal, depth: u32, rules: &MoveRules) -> Option<GameSim> {
    let Goal::Spin {
        piece, spin, lines, ..
    } = *goal
    else {
        return None;
    };
    let hit = |s: &GameSim| {
        s.events.iter().any(|e| {
            matches!(*e, stacklab_engine::events::EventKind::Spin { piece: p, spin: sp, lines: l }
                if piece.is_none_or(|q| q == p) && sp == spin && l == lines)
        })
    };
    if hit(sim) {
        return Some(sim.clone());
    }
    if depth == 0 || sim.game.is_over() {
        return None;
    }
    for (p, use_hold) in sim.options() {
        let mut opts = placements(sim.game.board(), p, rules);
        // try the required spin piece's spin placements first
        opts.sort_by_key(|pl| std::cmp::Reverse(pl.spin));
        for pl in opts {
            if depth == 1 && (pl.spin != spin || piece.is_some_and(|q| q != p)) {
                continue;
            }
            if let Some(next) = sim.play(&pl, use_hold, rules) {
                if let Some(found) = search(&next, goal, depth - 1, rules) {
                    return Some(found);
                }
            }
        }
    }
    None
}
