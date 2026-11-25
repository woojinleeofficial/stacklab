//! Drives one side of a real engine game with key inputs (spec 7.2 "실제 입력 실행").

use crate::{Bot, BotConfig, BotView};
use stacklab_engine::game::{path_inputs, Game, Handling, InputEvent};
use stacklab_engine::movegen::find_path;
use stacklab_engine::{MoveRules, Ruleset};

/// Bots press keys with this handling (taps only; DAS never charges).
pub fn bot_handling() -> Handling {
    Handling {
        das: 60,
        arr: 0,
        sdf: 0,
        dcd: 0,
    }
}

pub struct BotPlayer {
    pub bot: Bot,
    rules: MoveRules,
    pending: Vec<InputEvent>,
    planned_for: Option<u32>,
    next_free_sub: u64,
    spf: u64,
    frames_per_piece: u64,
}

impl BotPlayer {
    pub fn new(cfg: BotConfig, rs: &Ruleset, seed: u64) -> BotPlayer {
        let fpp = rs.timing.frames_per_second as u64 * 1000 / cfg.pps_milli.max(1) as u64;
        BotPlayer {
            rules: MoveRules::from_ruleset(rs),
            bot: Bot::new(cfg, rs, seed),
            pending: Vec::new(),
            planned_for: None,
            next_free_sub: 0,
            spf: rs.timing.subframes_per_frame as u64,
            frames_per_piece: fpp,
        }
    }

    /// Inputs for the coming frame of `g` (call before `g.step`).
    pub fn frame_inputs(&mut self, g: &Game) -> Vec<InputEvent> {
        let start = g.tick() * self.spf;
        let pieces = g.stats().pieces;
        if !g.is_over() && self.planned_for != Some(pieces) && start >= self.next_free_sub {
            if let Some(view) = BotView::from_game(g) {
                self.planned_for = Some(pieces);
                let delay = self.bot.cfg.input_delay_frames as u64 * self.spf;
                for d in self.bot.rank(&view) {
                    if let Some(path) = find_path(&view.board, &d.placement, &self.rules) {
                        let (inp, end) = path_inputs(&path, d.used_hold, start + delay);
                        self.pending = inp;
                        self.next_free_sub = end.max(start + self.frames_per_piece * self.spf);
                        break;
                    }
                }
            }
        }
        let (now, later): (Vec<_>, Vec<_>) = self
            .pending
            .drain(..)
            .partition(|e| e.time < start + self.spf);
        self.pending = later;
        now
    }
}

struct Job {
    pieces: u32,
    view: BotView,
    start: u64,
}

struct Plan {
    pieces: u32,
    inputs: Option<(Vec<InputEvent>, u64)>,
    think_ms: f64,
}

/// [`BotPlayer`] with the search on a worker thread (spec 14.4: AI search never blocks the UI).
///
/// Determinism is kept: a plan is computed from the same view and its inputs start at the same
/// sub-frame as in the synchronous player. If the plan is not back by the frame where its first
/// input is due, [`AsyncBotPlayer::frame_inputs`] returns `None` and the caller simply does not
/// step the simulation in that UI frame (the UI keeps drawing).
pub struct AsyncBotPlayer {
    pub cfg: BotConfig,
    tx: std::sync::mpsc::Sender<Job>,
    rx: std::sync::mpsc::Receiver<Plan>,
    pending: Vec<InputEvent>,
    planned_for: Option<u32>,
    outstanding: Option<(u32, u64)>,
    next_free_sub: u64,
    spf: u64,
    frames_per_piece: u64,
    delay_sub: u64,
    /// Search time of the latest plan (milliseconds), for the performance overlay.
    pub last_think_ms: f64,
}

impl AsyncBotPlayer {
    pub fn new(cfg: BotConfig, rs: &Ruleset, seed: u64) -> AsyncBotPlayer {
        let sync = BotPlayer::new(cfg.clone(), rs, seed);
        let (tx, jobs) = std::sync::mpsc::channel::<Job>();
        let (ptx, rx) = std::sync::mpsc::channel::<Plan>();
        let mut bot = sync.bot;
        let rules = sync.rules;
        std::thread::spawn(move || {
            for job in jobs {
                let t0 = std::time::Instant::now();
                let mut inputs = None;
                for d in bot.rank(&job.view) {
                    if let Some(path) = find_path(&job.view.board, &d.placement, &rules) {
                        inputs = Some(path_inputs(&path, d.used_hold, job.start));
                        break;
                    }
                }
                let plan = Plan {
                    pieces: job.pieces,
                    inputs,
                    think_ms: t0.elapsed().as_secs_f64() * 1000.0,
                };
                if ptx.send(plan).is_err() {
                    break;
                }
            }
        });
        AsyncBotPlayer {
            delay_sub: cfg.input_delay_frames as u64 * sync.spf,
            cfg,
            tx,
            rx,
            pending: Vec::new(),
            planned_for: None,
            outstanding: None,
            next_free_sub: 0,
            spf: sync.spf,
            frames_per_piece: sync.frames_per_piece,
            last_think_ms: 0.0,
        }
    }

    fn accept(&mut self, p: Plan) {
        if let Some((pieces, start)) = self.outstanding {
            if p.pieces == pieces {
                if let Some((inp, end)) = p.inputs {
                    self.pending = inp;
                    self.next_free_sub = end.max(start + self.frames_per_piece * self.spf);
                }
                self.outstanding = None;
                self.last_think_ms = p.think_ms;
            }
        }
    }

    /// Inputs for the coming frame of `g`, or `None` while a plan needed for this frame is
    /// still being searched.
    pub fn frame_inputs(&mut self, g: &Game) -> Option<Vec<InputEvent>> {
        while let Ok(p) = self.rx.try_recv() {
            self.accept(p);
        }
        let start = g.tick() * self.spf;
        let pieces = g.stats().pieces;
        if self.outstanding.is_none()
            && !g.is_over()
            && self.planned_for != Some(pieces)
            && start >= self.next_free_sub
        {
            if let Some(view) = BotView::from_game(g) {
                self.planned_for = Some(pieces);
                let job_start = start + self.delay_sub;
                if self
                    .tx
                    .send(Job {
                        pieces,
                        view,
                        start: job_start,
                    })
                    .is_ok()
                {
                    self.outstanding = Some((pieces, start));
                }
            }
        }
        if let Some((_, s)) = self.outstanding {
            if start + self.spf > s + self.delay_sub {
                return None;
            }
        }
        let (now, later): (Vec<_>, Vec<_>) = self
            .pending
            .drain(..)
            .partition(|e| e.time < start + self.spf);
        self.pending = later;
        Some(now)
    }

    /// Blocks until the outstanding plan (if any) arrives. For headless runs and tests.
    pub fn wait(&mut self) {
        if self.outstanding.is_some() {
            if let Ok(p) = self.rx.recv() {
                self.accept(p);
            }
        }
    }
}
