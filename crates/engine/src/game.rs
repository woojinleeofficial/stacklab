//! Fixed-tick deterministic game (spec 3.2).
//!
//! `Game::new` (= reset), `step` (one frame), `snapshot`/`restore`, `events`, and replay via
//! [`crate::replay`]. Integer arithmetic only. One frame = `subframes_per_frame` sub-steps;
//! input events carry absolute sub-frame timestamps and are applied in order.

use crate::attack::{on_lock, surge_amount, ChainState, ClearInfo};
use crate::board::{Board, PieceState, MAX_ROWS, WIDTH};
use crate::events::{Event, EventKind, Fnv, MatchEndReason, TopOutReason};
use crate::movegen::MoveRules;
use crate::piece::Piece;
use crate::rng::{Bag7, Rng};
use crate::rotation::{RotDir, Spin};
use crate::ruleset::Ruleset;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Key {
    Left,
    Right,
    SoftDrop,
    HardDrop,
    Cw,
    Ccw,
    Flip,
    Hold,
}

impl Key {
    fn idx(self) -> usize {
        self as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputEvent {
    /// Absolute time in sub-frames since the start of the game.
    pub time: u64,
    pub key: Key,
    pub down: bool,
}

/// Handling settings in sub-frames (TETR.IO units are frames with one decimal: 8.3F = 83).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handling {
    pub das: u32,
    pub arr: u32,
    /// Soft drop factor; 0 means infinite (sonic drop).
    pub sdf: u32,
    pub dcd: u32,
}

impl Default for Handling {
    fn default() -> Self {
        // TETR.IO defaults: DAS 10F, ARR 2F, SDF 6X, DCD 1F
        Handling {
            das: 100,
            arr: 20,
            sdf: 6,
            dcd: 10,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    /// Endless play (problems, versus). Garbage only arrives through `receive_garbage`.
    Free,
    /// Clear `lines` lines as fast as possible (40-line mode).
    Sprint { lines: u32 },
}

/// Custom starting position (problems, fixtures). Rows bottom-up.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct InitialState {
    pub rows: Vec<u16>,
    /// Pieces dealt before the seeded bag continues (current piece first).
    pub queue: Vec<Piece>,
    pub hold: Option<Piece>,
    pub b2b: i32,
    pub combo: i32,
    /// Bag continuation after `queue`: false = only the given queue (game ends when exhausted).
    pub continue_with_bag: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Packet {
    lines: u32,
    active_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Active {
    state: PieceState,
    /// (kick index, kick count) when the last successful action was a rotation.
    last_rot: Option<(u8, usize)>,
    lock_sub: u32,
    resets: u32,
    lowest_y: i8,
    grav_acc: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Stats {
    pub pieces: u32,
    pub lines: u32,
    /// Key presses of movement / rotation / drop / hold keys.
    pub inputs: u32,
    pub attack: u32,
    pub received: u32,
    pub finish_tick: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct Game {
    rs: Arc<Ruleset>,
    rules: Arc<MoveRules>,
    version: Arc<str>,
    handling: Handling,
    mode: Mode,
    spf: u64,

    board: Board,
    bag: Option<Bag7>,
    queue: VecDeque<Piece>,
    hold: Option<Piece>,
    hold_used: bool,
    active: Option<Active>,
    spawn_wait: u32,
    chain: ChainState,

    held: [bool; 8],
    dir: i8,
    das_sub: u32,
    arr_sub: u32,
    das_cut: u32,
    /// The press sub-frame itself does not count towards DAS.
    das_skip: bool,

    incoming: VecDeque<Packet>,
    garbage_rng: Rng,
    hole: u8,
    outbox: Vec<u32>,

    tick: u64,
    sub: u32,
    stats: Stats,
    last_lock_cleared: bool,
    clutch_streak: u32,
    over: bool,
    topped_out_at: Option<u64>,
    events: Vec<Event>,
}

const SCALE: u64 = 1_000_000;

impl Game {
    /// `reset(seed, ruleset)` of the logical API.
    pub fn new(rs: Arc<Ruleset>, seed: u64, handling: Handling, mode: Mode) -> Game {
        Self::with_initial(rs, seed, handling, mode, None)
    }

    pub fn with_initial(
        rs: Arc<Ruleset>,
        seed: u64,
        handling: Handling,
        mode: Mode,
        initial: Option<&InitialState>,
    ) -> Game {
        let rules = Arc::new(MoveRules::from_ruleset(&rs));
        let spf = rs.timing.subframes_per_frame as u64;
        let mut g = Game {
            version: Arc::from(rs.meta.version.as_str()),
            rules,
            handling,
            mode,
            spf,
            board: Board::default(),
            bag: Some(Bag7::new(seed, &rs.queue.bag_base_order)),
            queue: VecDeque::new(),
            hold: None,
            hold_used: false,
            active: None,
            spawn_wait: 0,
            chain: ChainState::default(),
            held: [false; 8],
            dir: 0,
            das_sub: 0,
            arr_sub: 0,
            das_cut: 0,
            das_skip: false,
            incoming: VecDeque::new(),
            garbage_rng: Rng::new(seed ^ 0x9e37_79b9_7f4a_7c15),
            hole: (seed % WIDTH as u64) as u8,
            outbox: Vec::new(),
            tick: 0,
            sub: 0,
            stats: Stats::default(),
            last_lock_cleared: false,
            clutch_streak: 0,
            over: false,
            topped_out_at: None,
            events: Vec::new(),
            rs,
        };
        if let Some(init) = initial {
            for (y, &r) in init.rows.iter().enumerate().take(MAX_ROWS) {
                g.board.rows[y] = r & crate::board::FULL_ROW;
            }
            g.queue.extend(init.queue.iter().copied());
            g.hold = init.hold;
            g.chain = ChainState {
                b2b: init.b2b,
                combo: init.combo,
            };
            if !init.continue_with_bag {
                g.bag = None;
            }
        }
        g.fill_queue();
        g.spawn_next();
        g
    }

    // ----- accessors -------------------------------------------------------------------

    pub fn ruleset(&self) -> &Ruleset {
        &self.rs
    }
    pub fn board(&self) -> &Board {
        &self.board
    }
    pub fn active_piece(&self) -> Option<PieceState> {
        self.active.as_ref().map(|a| a.state)
    }
    pub fn hold_piece(&self) -> Option<Piece> {
        self.hold
    }
    pub fn next_pieces(&self) -> Vec<Piece> {
        self.queue
            .iter()
            .take(self.rs.queue.next_count as usize)
            .copied()
            .collect()
    }
    pub fn chain(&self) -> ChainState {
        self.chain
    }
    pub fn stats(&self) -> Stats {
        self.stats
    }
    pub fn tick(&self) -> u64 {
        self.tick
    }
    pub fn now_sub(&self) -> u64 {
        self.tick * self.spf + self.sub as u64
    }
    pub fn is_over(&self) -> bool {
        self.over
    }
    /// Sub-frame time of the top-out, if the game ended that way.
    pub fn topped_out_at(&self) -> Option<u64> {
        self.topped_out_at
    }
    pub fn pending_garbage(&self) -> u32 {
        self.incoming.iter().map(|p| p.lines).sum()
    }
    pub fn ghost(&self) -> Option<PieceState> {
        self.active.as_ref().map(|a| self.board.drop_y(&a.state))
    }

    /// Drains the events produced since the last call.
    pub fn events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// Outgoing attack packets produced since the last call (for the versus router).
    pub fn take_outgoing(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.outbox)
    }

    pub fn snapshot(&self) -> Game {
        self.clone()
    }
    pub fn restore(&mut self, snap: &Game) {
        *self = snap.clone();
    }

    /// Queue garbage that becomes insertable after the travel delay.
    pub fn receive_garbage(&mut self, lines: u32) {
        if lines == 0 {
            return;
        }
        let active_at = self.tick + self.rs.garbage.travel_frames as u64;
        self.incoming.push_back(Packet { lines, active_at });
    }

    /// End the game from outside (opponent topped out).
    pub fn finish_external(&mut self, reason: MatchEndReason) {
        if !self.over {
            self.over = true;
            self.active = None;
            self.emit(EventKind::MatchEnd { reason });
        }
    }

    // ----- state hash ------------------------------------------------------------------

    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv::default();
        for r in self.board.rows {
            h.u64(r as u64);
        }
        if let Some(a) = &self.active {
            h.bytes(&[
                1,
                a.state.piece as u8,
                a.state.rot as u8,
                a.state.x as u8,
                a.state.y as u8,
            ]);
            h.u64(a.lock_sub as u64);
            h.u64(a.resets as u64);
            h.u64(a.grav_acc);
        } else {
            h.bytes(&[0]);
        }
        h.bytes(&[self.hold.map_or(9, |p| p as u8), self.hold_used as u8]);
        for p in &self.queue {
            h.bytes(&[*p as u8]);
        }
        h.i64(self.chain.b2b as i64);
        h.i64(self.chain.combo as i64);
        for p in &self.incoming {
            h.u64(p.lines as u64);
            h.u64(p.active_at);
        }
        h.u64(self.hole as u64);
        h.u64(self.tick);
        h.u64(self.stats.pieces as u64);
        h.u64(self.stats.lines as u64);
        h.bytes(&[self.over as u8]);
        h.finish()
    }

    // ----- frame stepping --------------------------------------------------------------

    /// Advance one frame. `inputs` should contain the events whose time falls in this frame;
    /// earlier timestamps are applied at the first sub-frame, later ones are ignored.
    pub fn step(&mut self, inputs: &[InputEvent]) {
        if self.over {
            self.tick += 1;
            return;
        }
        let start = self.tick * self.spf;
        let mut sorted: Vec<&InputEvent> = inputs
            .iter()
            .filter(|e| e.time < start + self.spf)
            .collect();
        sorted.sort_by_key(|e| e.time.max(start));
        let mut it = sorted.into_iter().peekable();
        for s in 0..self.spf as u32 {
            self.sub = s;
            let now = start + s as u64;
            while let Some(e) = it.peek() {
                if e.time.max(start) == now {
                    let e = *it.next().expect("peeked");
                    self.apply_input(&e);
                    if self.over {
                        break;
                    }
                } else {
                    break;
                }
            }
            if self.over {
                break;
            }
            self.substep();
            if self.over {
                break;
            }
        }
        self.sub = 0;
        self.tick += 1;
    }

    fn emit(&mut self, kind: EventKind) {
        let ev = Event {
            tick: self.tick,
            sub: self.sub,
            piece_index: self.stats.pieces,
            state_hash: self.state_hash(),
            ruleset_version: self.version.clone(),
            kind,
        };
        self.events.push(ev);
    }

    fn fill_queue(&mut self) {
        let want = self.rs.queue.next_count as usize + 1;
        if let Some(bag) = self.bag.as_mut() {
            while self.queue.len() < want {
                self.queue.push_back(bag.next_piece());
            }
        }
    }

    fn top_out(&mut self, reason: TopOutReason) {
        self.topped_out_at = Some(self.now_sub());
        self.active = None;
        self.over = true;
        self.emit(EventKind::TopOut { reason });
        self.emit(EventKind::MatchEnd {
            reason: MatchEndReason::TopOut(reason),
        });
    }

    fn spawn_next(&mut self) {
        let Some(p) = self.queue.pop_front() else {
            // fixed problem queue exhausted: the game simply stops producing pieces
            self.active = None;
            return;
        };
        self.fill_queue();
        self.spawn_piece(p);
    }

    fn spawn_piece(&mut self, p: Piece) {
        let mut s = self.rules.spawn(p);
        if !self.board.fits(&s) {
            let can_clutch = self.rs.topout.clutch_clear && self.last_lock_cleared;
            let mut placed = false;
            if can_clutch {
                while (s.y as usize) < MAX_ROWS {
                    s.y += 1;
                    if self.board.fits(&s) {
                        placed = true;
                        break;
                    }
                }
            }
            if !placed {
                self.top_out(TopOutReason::BlockOut);
                return;
            }
            self.clutch_streak += 1;
            let streak = self.clutch_streak;
            self.active = Some(Active::new(s));
            self.emit(EventKind::ClutchClear { streak });
        } else {
            self.clutch_streak = 0;
            self.active = Some(Active::new(s));
        }
        self.das_cut = self.handling.dcd;
        self.emit(EventKind::PieceSpawn { piece: p });
    }

    fn apply_input(&mut self, e: &InputEvent) {
        let k = e.key.idx();
        if e.down {
            if self.held[k] {
                return; // repeated key-down without key-up
            }
            self.held[k] = true;
            self.stats.inputs += 1;
            match e.key {
                Key::Left | Key::Right => {
                    self.dir = if e.key == Key::Left { -1 } else { 1 };
                    self.das_sub = 0;
                    self.arr_sub = 0;
                    self.das_skip = true;
                    self.shift(self.dir);
                }
                Key::Cw => self.rotate(RotDir::Cw),
                Key::Ccw => self.rotate(RotDir::Ccw),
                Key::Flip => self.rotate(RotDir::Flip),
                Key::HardDrop => self.hard_drop(),
                Key::Hold => self.do_hold(),
                Key::SoftDrop => {}
            }
        } else {
            self.held[k] = false;
            match e.key {
                Key::Left if self.dir == -1 => {
                    self.dir = if self.held[Key::Right.idx()] { 1 } else { 0 };
                    self.das_sub = 0;
                    self.arr_sub = 0;
                }
                Key::Right if self.dir == 1 => {
                    self.dir = if self.held[Key::Left.idx()] { -1 } else { 0 };
                    self.das_sub = 0;
                    self.arr_sub = 0;
                }
                _ => {}
            }
        }
    }

    /// Lock-delay bookkeeping after a successful move or rotation.
    fn on_moved(&mut self) {
        let limit = self.rs.gravity.lock_reset_limit;
        let restore = self.rs.gravity.reset_count_restores_on_new_low;
        let Some(a) = self.active.as_mut() else {
            return;
        };
        if a.state.y < a.lowest_y {
            a.lowest_y = a.state.y;
            a.lock_sub = 0;
            if restore {
                a.resets = 0;
            }
        } else if a.lock_sub > 0 && a.resets < limit {
            a.lock_sub = 0;
            a.resets += 1;
        }
    }

    fn shift(&mut self, dx: i8) -> bool {
        let Some(a) = self.active.as_mut() else {
            return false;
        };
        let n = a.state.shifted(dx, 0);
        if self.board.fits(&n) {
            a.state = n;
            a.last_rot = None;
            self.on_moved();
            true
        } else {
            false
        }
    }

    fn rotate(&mut self, dir: RotDir) {
        let Some(a) = self.active.as_mut() else {
            return;
        };
        if let Some((n, k)) = self.rules.kicks.try_rotate(&self.board, &a.state, dir) {
            let kc = self.rules.kicks.kick_count(n.piece, a.state.rot, n.rot);
            a.state = n;
            a.last_rot = Some((k, kc));
            self.das_cut = self.handling.dcd;
            self.on_moved();
        }
    }

    fn fall(&mut self, cells: u64) {
        for _ in 0..cells {
            let Some(a) = self.active.as_mut() else {
                return;
            };
            let n = a.state.shifted(0, -1);
            if self.board.fits(&n) {
                a.state = n;
                a.last_rot = None;
                self.on_moved();
            } else {
                return;
            }
        }
    }

    fn hard_drop(&mut self) {
        let Some(a) = self.active.as_mut() else {
            return;
        };
        let d = self.board.drop_y(&a.state);
        if d.y != a.state.y {
            a.state = d;
            a.last_rot = None;
        }
        self.lock_active();
    }

    fn do_hold(&mut self) {
        if !self.rs.queue.hold || self.hold_used {
            return;
        }
        let Some(a) = self.active.take() else {
            return;
        };
        let cur = a.state.piece;
        let took = self.hold.take();
        self.hold = Some(cur);
        self.hold_used = true;
        self.emit(EventKind::HoldUsed { held: cur, took });
        match took {
            Some(p) => self.spawn_piece(p),
            None => self.spawn_next(),
        }
    }

    fn substep(&mut self) {
        if self.active.is_none() {
            if self.spawn_wait > 0 {
                self.spawn_wait -= 1;
                if self.spawn_wait == 0 {
                    self.spawn_next();
                }
            }
            return;
        }
        // auto shift
        if self.dir != 0 && self.held[if self.dir < 0 { 0 } else { 1 }] {
            if self.das_skip {
                self.das_skip = false;
            } else if self.das_cut > 0 {
                self.das_cut -= 1;
            } else if self.das_sub < self.handling.das {
                self.das_sub += 1;
                if self.das_sub == self.handling.das {
                    self.auto_shift_once();
                }
            } else {
                self.arr_sub += 1;
                if self.handling.arr == 0 || self.arr_sub >= self.handling.arr {
                    self.arr_sub = 0;
                    self.auto_shift_once();
                }
            }
        } else if self.das_cut > 0 {
            self.das_cut -= 1;
        }
        // gravity and soft drop
        let g = self.rs.gravity.gravity_micro as u64;
        let soft = self.held[Key::SoftDrop.idx()];
        if soft && self.handling.sdf == 0 {
            if let Some(a) = self.active.as_ref() {
                let d = self.board.drop_y(&a.state);
                let cells = (a.state.y - d.y) as u64;
                self.fall(cells);
            }
        } else {
            let eff = if soft {
                g.max(self.rs.gravity.soft_drop_min_micro as u64) * self.handling.sdf as u64
            } else {
                g
            };
            let threshold = SCALE * self.spf;
            let cells = if let Some(a) = self.active.as_mut() {
                a.grav_acc += eff;
                let c = a.grav_acc / threshold;
                a.grav_acc %= threshold;
                c
            } else {
                0
            };
            if cells > 0 {
                self.fall(cells);
            }
        }
        // lock delay
        let lock_limit = self.rs.gravity.lock_delay_frames * self.spf as u32;
        let grounded = self
            .active
            .as_ref()
            .is_some_and(|a| self.board.grounded(&a.state));
        let mut lock_now = false;
        if let Some(a) = self.active.as_mut() {
            if grounded {
                a.lock_sub += 1;
                lock_now = a.lock_sub >= lock_limit;
            } else {
                a.lock_sub = 0;
            }
        }
        if lock_now {
            self.lock_active();
        }
    }

    fn auto_shift_once(&mut self) {
        if self.handling.arr == 0 {
            while self.shift(self.dir) {}
        } else {
            self.shift(self.dir);
        }
    }

    fn margin_milli(&self) -> u64 {
        crate::attack::margin_multiplier_milli(&self.rs, self.tick)
    }

    fn lock_active(&mut self) {
        let Some(a) = self.active.take() else {
            return;
        };
        let s = a.state;
        let spin = self.rules.spins.detect(&self.board, &s, a.last_rot);
        self.board.place(&s);
        let lines = self.board.clear_lines();
        let pc = lines > 0 && self.board.is_empty();
        self.stats.pieces += 1;
        self.stats.lines += lines;
        self.emit(EventKind::PieceLock {
            piece: s.piece,
            rot: s.rot,
            x: s.x,
            y: s.y,
        });
        if spin != Spin::None {
            self.emit(EventKind::Spin {
                piece: s.piece,
                spin,
                lines,
            });
        }
        if lines > 0 {
            self.emit(EventKind::LineClear { lines });
        }
        if pc {
            self.emit(EventKind::PerfectClear);
        }
        let before = self.chain;
        let res = on_lock(
            &self.rs,
            &mut self.chain,
            &ClearInfo {
                piece: s.piece,
                lines,
                spin,
                perfect_clear: pc,
            },
        );
        if before.combo != self.chain.combo {
            self.emit(EventKind::ComboChange {
                from: before.combo,
                to: self.chain.combo,
            });
        }
        if before.b2b != self.chain.b2b {
            self.emit(EventKind::B2bChange {
                from: before.b2b,
                to: self.chain.b2b,
            });
            let charge = surge_amount(&self.rs, self.chain.b2b);
            if charge > 0 && self.chain.b2b > before.b2b {
                self.emit(EventKind::SurgeCharge { amount: charge });
            }
        }
        if res.surge_released > 0 {
            let segments =
                crate::attack::split_surge(res.surge_released, self.rs.b2b.surge_segments);
            self.emit(EventKind::SurgeRelease {
                amount: res.surge_released,
                segments,
            });
        }

        // attack: margin multiplier, then cancel incoming, then send
        let mult = self.margin_milli();
        let pending = self.pending_garbage();
        let op = self.rs.opener_phase.clone();
        let factor = if op.enabled && self.stats.pieces <= op.pieces && self.stats.attack < pending
        {
            op.cancel_multiplier.max(1)
        } else {
            1
        };
        for pkt in res.packets {
            let lines = (pkt as u64 * mult / 1000) as u32;
            if lines == 0 {
                continue;
            }
            let mut power = lines * factor;
            let mut cancelled = 0;
            while power > 0 {
                let Some(front) = self.incoming.front_mut() else {
                    break;
                };
                let c = front.lines.min(power);
                front.lines -= c;
                power -= c;
                cancelled += c;
                if front.lines == 0 {
                    self.incoming.pop_front();
                }
            }
            if cancelled > 0 {
                self.emit(EventKind::AttackCancelled { lines: cancelled });
            }
            let out = power.div_ceil(factor);
            self.stats.attack += lines;
            if out > 0 {
                self.outbox.push(out);
                self.emit(EventKind::AttackSent { lines: out });
            }
        }
        if op.enabled && self.stats.pieces == op.pieces {
            self.emit(EventKind::OpenerPhaseEnd);
        }

        // garbage insertion
        let insert = lines == 0 || !self.rs.garbage.insert_only_on_non_clearing_lock;
        if insert && !self.insert_garbage() {
            self.top_out(TopOutReason::GarbageOverflow);
            return;
        }

        self.last_lock_cleared = lines > 0;
        self.hold_used = false;

        if let Mode::Sprint { lines: goal } = self.mode {
            if self.stats.lines >= goal {
                self.stats.finish_tick = Some(self.tick);
                self.over = true;
                self.emit(EventKind::MatchEnd {
                    reason: MatchEndReason::GoalReached,
                });
                return;
            }
        }

        let wait = self.rs.timing.are_frames
            + if lines > 0 {
                self.rs.timing.line_clear_are_frames
            } else {
                0
            };
        if wait == 0 {
            self.spawn_next();
        } else {
            self.spawn_wait = wait * self.spf as u32;
        }
    }

    /// Inserts active garbage up to the per-piece cap. Returns false on overflow.
    fn insert_garbage(&mut self) -> bool {
        let mut budget = self.rs.garbage.cap_lines_per_piece;
        let mut first_line_of_packet;
        while budget > 0 {
            let Some(front) = self.incoming.front_mut() else {
                break;
            };
            if front.active_at > self.tick {
                break;
            }
            let n = front.lines.min(budget);
            front.lines -= n;
            budget -= n;
            let done = front.lines == 0;
            if done {
                self.incoming.pop_front();
            }
            first_line_of_packet = true;
            for _ in 0..n {
                let pm = if first_line_of_packet {
                    self.rs.garbage.messiness_between_attacks_pm
                } else {
                    self.rs.garbage.messiness_within_attack_pm
                };
                first_line_of_packet = false;
                if self.garbage_rng.chance_pm(pm) {
                    self.change_hole();
                }
                if !self.board.push_garbage(1, self.hole as usize) {
                    return false;
                }
                self.stats.received += 1;
            }
            let hole = self.hole;
            self.emit(EventKind::GarbageReceived { lines: n, hole });
        }
        true
    }

    fn change_hole(&mut self) {
        let w = WIDTH as u64;
        if self.rs.garbage.hole_change_must_differ {
            let r = self.garbage_rng.below(w - 1) as u8;
            self.hole = (self.hole + 1 + r) % WIDTH as u8;
        } else {
            self.hole = self.garbage_rng.below(w) as u8;
        }
    }
}

impl Active {
    fn new(state: PieceState) -> Active {
        Active {
            state,
            last_rot: None,
            lock_sub: 0,
            resets: 0,
            lowest_y: state.y,
            grav_acc: 0,
        }
    }
}

/// Converts a witness path into key taps starting at sub-frame `t` (each tap: down, then up one
/// sub-frame later). Ends with a hard drop. Returns the events and the next free time.
/// Assumes SDF = infinite so `Move::Sonic` is a soft-drop tap.
pub fn path_inputs(
    moves: &[crate::movegen::Move],
    use_hold: bool,
    mut t: u64,
) -> (Vec<InputEvent>, u64) {
    use crate::movegen::Move;
    let mut out = Vec::new();
    let mut tap = |k: Key, t: &mut u64| {
        out.push(InputEvent {
            time: *t,
            key: k,
            down: true,
        });
        out.push(InputEvent {
            time: *t + 1,
            key: k,
            down: false,
        });
        *t += 2;
    };
    if use_hold {
        tap(Key::Hold, &mut t);
    }
    for m in moves {
        let k = match m {
            Move::Left => Key::Left,
            Move::Right => Key::Right,
            Move::Cw => Key::Cw,
            Move::Ccw => Key::Ccw,
            Move::Flip => Key::Flip,
            Move::Sonic => Key::SoftDrop,
        };
        tap(k, &mut t);
    }
    tap(Key::HardDrop, &mut t);
    (out, t)
}
