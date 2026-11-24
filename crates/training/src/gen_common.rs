//! Shared helpers for generators: field evaluation, exact game-level simulation of placements
//! (so witnesses match the engine, garbage and attack included) and problem assembly.

use crate::problem::{fingerprint, Goal, Module, Problem, TargetPlacement, Witness, WitnessStep};
use crate::session::count_holes;
use stacklab_engine::board::{Board, WIDTH};
use stacklab_engine::events::EventKind;
use stacklab_engine::game::{path_inputs, Game, Handling, InitialState, InputEvent, Mode};
use stacklab_engine::movegen::find_path;
use stacklab_engine::{MoveRules, Placement, Ruleset};
use std::sync::Arc;

pub fn witness_handling() -> Handling {
    Handling {
        das: 60,
        arr: 0,
        sdf: 0,
        dcd: 0,
    }
}

/// Static field score (higher is better).
pub fn eval_board(b: &Board) -> i32 {
    let mut s = -(count_holes(b) as i32) * 100;
    let mut prev: Option<i32> = None;
    let mut max_h = 0;
    let mut heights = [0i32; WIDTH];
    for (x, h) in heights.iter_mut().enumerate() {
        *h = b.column_height(x) as i32;
    }
    let well = (0..WIDTH).min_by_key(|&x| heights[x]).unwrap_or(0);
    for (x, &h) in heights.iter().enumerate() {
        max_h = max_h.max(h);
        if x == well {
            continue;
        }
        if let Some(ph) = prev {
            s -= (h - ph).abs() * 6;
        }
        prev = Some(h);
    }
    s - max_h * 8 - (max_h - 12).max(0).pow(2) * 30
}

/// A game plus the inputs that led to it, for exact witness construction.
#[derive(Clone)]
pub struct GameSim {
    pub game: Game,
    pub inputs: Vec<InputEvent>,
    pub t: u64,
    pub steps: Vec<WitnessStep>,
    pub events: Vec<EventKind>,
}

impl GameSim {
    pub fn new(rs: Arc<Ruleset>, seed: u64, initial: &InitialState) -> GameSim {
        let mut game = Game::with_initial(rs, seed, witness_handling(), Mode::Free, Some(initial));
        let events = game.events().into_iter().map(|e| e.kind).collect();
        GameSim {
            game,
            inputs: Vec::new(),
            t: 0,
            steps: Vec::new(),
            events,
        }
    }

    /// Plays one placement (optionally pressing hold first) with real inputs until it locks.
    pub fn play(&self, pl: &Placement, use_hold: bool, rules: &MoveRules) -> Option<GameSim> {
        let path = find_path(self.game.board(), pl, rules)?;
        let (inp, end) = path_inputs(&path, use_hold, self.t);
        let mut next = self.clone();
        let spf = self.game.ruleset().timing.subframes_per_frame as u64;
        let before = next.game.stats().pieces;
        let mut guard = 0;
        while next.game.stats().pieces == before && !next.game.is_over() {
            let start = next.game.tick() * spf;
            let frame: Vec<InputEvent> = inp
                .iter()
                .filter(|e| e.time >= start && e.time < start + spf)
                .copied()
                .collect();
            next.game.step(&frame);
            next.events
                .extend(next.game.events().into_iter().map(|e| e.kind));
            guard += 1;
            if guard > 400 {
                return None;
            }
        }
        // the lock must be exactly the planned placement
        let locked = next.events.iter().rev().find_map(|e| match *e {
            EventKind::PieceLock { piece, rot, x, y } => {
                Some(stacklab_engine::PieceState { piece, rot, x, y })
            }
            _ => None,
        })?;
        if locked.cell_key() != pl.state.cell_key() {
            return None;
        }
        next.inputs.extend(inp);
        next.t = end.max(next.game.tick() * spf) + 2;
        next.steps.push(WitnessStep {
            target: TargetPlacement {
                piece: pl.state.piece,
                rot: pl.state.rot,
                x: pl.state.x,
                y: pl.state.y,
            },
            use_hold,
        });
        Some(next)
    }

    /// Pieces playable now: (piece, use_hold).
    pub fn options(&self) -> Vec<(stacklab_engine::Piece, bool)> {
        let Some(cur) = self.game.active_piece().map(|p| p.piece) else {
            return Vec::new();
        };
        let mut v = vec![(cur, false)];
        match self.game.hold_piece() {
            Some(h) if h != cur => v.push((h, true)),
            Some(_) => {}
            None => {
                if let Some(&n) = self.game.next_pieces().first() {
                    v.push((n, true));
                }
            }
        }
        v
    }

    pub fn witness(&self) -> Witness {
        Witness {
            handling: witness_handling(),
            inputs: self.inputs.clone(),
            frames: self.t / 10 + 4,
            placements: self.steps.clone(),
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn assemble(
    module: Module,
    tags: Vec<String>,
    seed: u64,
    rs: &Ruleset,
    generator_version: &str,
    initial: InitialState,
    visible_next: u8,
    goal: Goal,
    witness: Witness,
    difficulty: f64,
    param_bucket: String,
    time_limit_secs: u32,
) -> Problem {
    let fp = fingerprint(module, &initial, &goal);
    Problem {
        id: format!("{}-{fp:016x}", module.letter()),
        module,
        subtype_tags: tags,
        seed,
        ruleset_version: rs.meta.version.clone(),
        generator_version: generator_version.into(),
        validation_version: crate::validator::VALIDATION_VERSION.into(),
        initial,
        visible_next,
        goal,
        witness,
        difficulty,
        param_bucket,
        time_limit_secs,
        fingerprint: fp,
    }
}

pub fn initial(board: &Board, queue: Vec<stacklab_engine::Piece>) -> InitialState {
    InitialState {
        rows: board.rows.to_vec(),
        queue,
        hold: None,
        b2b: -1,
        combo: -1,
        continue_with_bag: false,
        incoming: Vec::new(),
    }
}

/// Random no-hole terrain around `height` with an optional empty well column.
pub fn random_terrain(height: u32, well: Option<u8>, rng: &mut stacklab_engine::rng::Rng) -> Board {
    let mut b = Board::default();
    let mut h = height as i64;
    for x in 0..WIDTH {
        if Some(x as u8) == well {
            continue;
        }
        h = (h + rng.below(3) as i64 - 1)
            .clamp(height as i64 - 2, height as i64 + 1)
            .max(0);
        for row in b.rows.iter_mut().take(h as usize) {
            *row |= 1 << x;
        }
    }
    for y in 0..stacklab_engine::board::MAX_ROWS {
        if b.rows[y] == stacklab_engine::board::FULL_ROW {
            let x = rng.below(WIDTH as u64) as usize;
            for row in b.rows.iter_mut().skip(y) {
                *row &= !(1 << x);
            }
        }
    }
    b
}

/// Beam search with the real engine. `score` ranks game states; candidates per piece are
/// pre-filtered by a static board evaluation to keep generation fast.
pub fn beam_search(
    start: &GameSim,
    depth: u32,
    width: usize,
    per_piece: usize,
    rules: &MoveRules,
    score: &dyn Fn(&GameSim) -> i64,
) -> Vec<GameSim> {
    let mut frontier = vec![start.clone()];
    for _ in 0..depth {
        let mut next: Vec<(i64, GameSim)> = Vec::new();
        for sim in &frontier {
            if sim.game.is_over() {
                continue;
            }
            for (p, hold) in sim.options() {
                let mut opts: Vec<(i32, Placement)> =
                    stacklab_engine::placements(sim.game.board(), p, rules)
                        .into_iter()
                        .map(|pl| {
                            let mut b = sim.game.board().clone();
                            let lines = stacklab_engine::lock(&mut b, &pl).lines as i32;
                            let spin_bonus = if pl.spin != stacklab_engine::Spin::None {
                                150
                            } else {
                                0
                            };
                            (eval_board(&b) + lines * 40 + spin_bonus, pl)
                        })
                        .collect();
                opts.sort_by_key(|o| std::cmp::Reverse(o.0));
                for (_, pl) in opts.into_iter().take(per_piece) {
                    if let Some(child) = sim.play(&pl, hold, rules) {
                        next.push((score(&child), child));
                    }
                }
            }
        }
        if next.is_empty() {
            break;
        }
        next.sort_by_key(|n| std::cmp::Reverse(n.0));
        next.truncate(width);
        frontier = next.into_iter().map(|n| n.1).collect();
    }
    frontier
}
