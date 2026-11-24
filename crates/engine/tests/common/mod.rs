#![allow(dead_code)]

use stacklab_engine::events::{Event, EventKind};
use stacklab_engine::game::{path_inputs, Game, Handling, InitialState, InputEvent, Mode};
use stacklab_engine::movegen::find_path;
use stacklab_engine::{placements, Board, MoveRules, Piece, Placement, Rot, Ruleset, Spin};
use std::sync::Arc;

pub fn rs() -> Arc<Ruleset> {
    Arc::new(Ruleset::tl_s2())
}

/// Handling used by fixtures: SDF infinite so soft drop = sonic drop.
pub fn fast_handling() -> Handling {
    Handling {
        das: 100,
        arr: 0,
        sdf: 0,
        dcd: 0,
    }
}

pub struct Harness {
    pub g: Game,
    pub rules: MoveRules,
    pub events: Vec<Event>,
    pub t: u64,
}

impl Harness {
    pub fn new(init: InitialState) -> Harness {
        Self::with(init, Mode::Free)
    }

    pub fn with(init: InitialState, mode: Mode) -> Harness {
        let rs = rs();
        let rules = MoveRules::from_ruleset(&rs);
        let mut g = Game::with_initial(rs, 1, fast_handling(), mode, Some(&init));
        let events = g.events();
        Harness {
            g,
            rules,
            events,
            t: 0,
        }
    }

    /// Runs frames until sub-frame time `until` has been processed, feeding `inputs`.
    pub fn run_inputs(&mut self, inputs: &[InputEvent], until: u64) {
        let spf = self.g.ruleset().timing.subframes_per_frame as u64;
        while self.g.now_sub() < until {
            let start = self.g.tick() * spf;
            let frame: Vec<InputEvent> = inputs
                .iter()
                .filter(|e| e.time >= start && e.time < start + spf)
                .copied()
                .collect();
            self.g.step(&frame);
            self.events.extend(self.g.events());
        }
        self.t = self.t.max(until);
    }

    pub fn frames(&mut self, n: u64) {
        let spf = self.g.ruleset().timing.subframes_per_frame as u64;
        let until = (self.g.tick() + n) * spf;
        self.run_inputs(&[], until);
    }

    /// Plays one placement through real key inputs (with optional hold first).
    pub fn play(&mut self, target: &Placement, use_hold: bool) {
        let board = self.g.board().clone();
        let path = find_path(&board, target, &self.rules).expect("placement reachable");
        let t0 = self.g.now_sub();
        let (inputs, end) = path_inputs(&path, use_hold, t0);
        self.run_inputs(&inputs, end + 1);
    }

    /// Finds a reachable placement of the current piece matching the predicate.
    pub fn find(&self, piece: Piece, pred: impl Fn(&Placement) -> bool) -> Placement {
        placements(self.g.board(), piece, &self.rules)
            .into_iter()
            .find(|p| pred(p))
            .expect("placement exists")
    }

    pub fn place_at(&mut self, piece: Piece, rot: Rot, x: i8, y: i8, spin: Spin) {
        let p = self.find(piece, |p| {
            p.state.rot == rot && p.state.x == x && p.state.y == y && p.spin == spin
        });
        self.play(&p, false);
    }

    pub fn kinds(&self) -> Vec<&EventKind> {
        self.events.iter().map(|e| &e.kind).collect()
    }

    pub fn has(&self, f: impl Fn(&EventKind) -> bool) -> bool {
        self.events.iter().any(|e| f(&e.kind))
    }

    pub fn sent(&self) -> Vec<u32> {
        self.events
            .iter()
            .filter_map(|e| match e.kind {
                EventKind::AttackSent { lines } => Some(lines),
                _ => None,
            })
            .collect()
    }
}

pub fn init(rows_top_down: &[&str], queue: &str) -> InitialState {
    let b = Board::from_ascii(rows_top_down);
    InitialState {
        rows: b.rows.to_vec(),
        queue: stacklab_engine::piece::parse_queue(queue),
        hold: None,
        b2b: -1,
        combo: -1,
        continue_with_bag: true,
    }
}

/// Fuzz player: whenever a new piece is up, picks a random reachable placement (biased to low
/// ones so games last) and plays it through real key taps.
pub struct PlacementPlayer {
    pub rng: stacklab_engine::rng::Rng,
    pub rules: MoveRules,
    pending: Vec<InputEvent>,
    busy_until: u64,
    last_pieces: u32,
    planned_for: Option<(u32, u64)>,
}

impl PlacementPlayer {
    pub fn new(seed: u64) -> PlacementPlayer {
        PlacementPlayer {
            rng: stacklab_engine::rng::Rng::new(seed),
            rules: MoveRules::from_ruleset(&Ruleset::tl_s2()),
            pending: Vec::new(),
            busy_until: 0,
            last_pieces: u32::MAX,
            planned_for: None,
        }
    }

    /// Inputs for the coming frame.
    pub fn frame_inputs(&mut self, g: &Game) -> Vec<InputEvent> {
        let spf = g.ruleset().timing.subframes_per_frame as u64;
        let start = g.tick() * spf;
        let key = (g.stats().pieces, g.stats().received as u64);
        if g.active_piece().is_some() && self.planned_for != Some(key) && start >= self.busy_until {
            self.planned_for = Some(key);
            self.last_pieces = g.stats().pieces;
            let use_hold = g.hold_piece().is_some() && self.rng.chance_pm(150);
            let piece = if use_hold {
                g.hold_piece().unwrap()
            } else {
                g.active_piece().unwrap().piece
            };
            let mut opts = placements(g.board(), piece, &self.rules);
            if !opts.is_empty() {
                opts.sort_by_key(|p| std::cmp::Reverse(quick_eval(g.board(), p)));
                let n = if self.rng.chance_pm(150) {
                    opts.len().min(3)
                } else {
                    1
                };
                let pick = opts[self.rng.below(n as u64) as usize];
                if std::env::var("FUZZ_DEBUG").is_ok() {
                    eprintln!(
                        "plan {:?} hold={use_hold} -> {:?}",
                        pick.state,
                        find_path(g.board(), &pick, &self.rules)
                    );
                }
                if let Some(path) = find_path(g.board(), &pick, &self.rules) {
                    let delay = 2 + self.rng.below(20);
                    let (inp, end) = path_inputs(&path, use_hold, start + delay);
                    self.pending.extend(inp);
                    self.busy_until = end + 1;
                }
            }
        }
        let (now, later): (Vec<_>, Vec<_>) =
            self.pending.drain(..).partition(|e| e.time < start + spf);
        self.pending = later;
        now
    }
}

/// Tiny field evaluation for the fuzz player (clear lines, avoid holes and height).
pub fn quick_eval(board: &Board, p: &Placement) -> i32 {
    let mut b = board.clone();
    let res = stacklab_engine::lock(&mut b, p);
    let mut holes = 0;
    let mut bump = 0;
    let mut prev: Option<i32> = None;
    let mut max_h = 0;
    for x in 0..10 {
        let h = b.column_height(x) as i32;
        max_h = max_h.max(h);
        for row in b.rows.iter().take(h as usize) {
            if row >> x & 1 == 0 {
                holes += 1;
            }
        }
        if let Some(ph) = prev {
            bump += (h - ph).abs();
        }
        prev = Some(h);
    }
    res.lines as i32 * 60 + if p.spin == Spin::Full { 200 } else { 0 }
        - holes * 50
        - bump * 4
        - max_h * 6
}
