//! One attempt at a problem: plays through the real engine, judges from engine state/events,
//! supports 1-piece rewind, hints and "show answer" (spec 4.1, UI-03/UI-04).

use crate::problem::{Goal, Problem, TargetPlacement, WitnessStep};
use serde::{Deserialize, Serialize};
use stacklab_engine::board::Board;
use stacklab_engine::events::{Event, EventKind};
use stacklab_engine::game::{Game, Handling, InputEvent, Mode};
use stacklab_engine::replay::ruleset_by_version;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailReason {
    WrongPlacement,
    TopOut,
    GoalNotMet {
        holes: u32,
        height: u32,
        well_blocked: bool,
    },
    /// Spin / PC / attack / downstack target not reached within the allowed pieces.
    NotAchieved,
    TimeUp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Running,
    Success,
    Failed(FailReason),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttemptResult {
    pub success: bool,
    /// Finesse problems only: placement reached with no more key presses than optimal.
    pub finesse_success: Option<bool>,
    pub used_hint: bool,
    pub used_rewind: bool,
    pub used_answer: bool,
    /// S=1/S=0 rule (spec 6.2): only attempts without assistance update skill ratings.
    pub valid_for_mmr: bool,
    pub elapsed_frames: u64,
    pub inputs_per_piece: Vec<u32>,
    pub reason: Option<FailReason>,
}

#[derive(Debug, Clone)]
pub struct AttemptSession {
    pub problem: Problem,
    game: Game,
    /// checkpoints[k] = state after k pieces were locked.
    checkpoints: Vec<Game>,
    events: Vec<Event>,
    inputs_at_piece_start: u32,
    inputs_per_piece: Vec<u32>,
    used_hint: bool,
    used_rewind: bool,
    used_answer: bool,
    status: Status,
    frames: u64,
}

pub fn count_holes(b: &Board) -> u32 {
    let mut holes = 0;
    for x in 0..stacklab_engine::board::WIDTH {
        let h = b.column_height(x);
        holes += b.rows[..h].iter().filter(|r| *r >> x & 1 == 0).count() as u32;
    }
    holes
}

impl AttemptSession {
    pub fn new(problem: &Problem, handling: Handling) -> Result<AttemptSession, String> {
        let rs = ruleset_by_version(&problem.ruleset_version)
            .ok_or_else(|| format!("unknown ruleset {}", problem.ruleset_version))?;
        let mut game = Game::with_initial(
            rs,
            problem.seed,
            handling,
            Mode::Free,
            Some(&problem.initial),
        );
        let events = game.events();
        Ok(AttemptSession {
            problem: problem.clone(),
            checkpoints: vec![game.snapshot()],
            game,
            events,
            inputs_at_piece_start: 0,
            inputs_per_piece: Vec::new(),
            used_hint: false,
            used_rewind: false,
            used_answer: false,
            status: Status::Running,
            frames: 0,
        })
    }

    pub fn game(&self) -> &Game {
        &self.game
    }
    pub fn status(&self) -> &Status {
        &self.status
    }
    pub fn events(&self) -> &[Event] {
        &self.events
    }
    pub fn pieces_locked(&self) -> usize {
        self.checkpoints.len() - 1
    }
    pub fn time_limit_frames(&self) -> u64 {
        self.problem.time_limit_secs as u64 * self.game.ruleset().timing.frames_per_second as u64
    }
    pub fn remaining_frames(&self) -> u64 {
        self.time_limit_frames().saturating_sub(self.frames)
    }

    pub fn step(&mut self, inputs: &[InputEvent]) {
        if self.status != Status::Running {
            return;
        }
        self.game.step(inputs);
        self.frames += 1;
        let mut locks = 0;
        let mut topped = false;
        for e in self.game.events() {
            match e.kind {
                EventKind::PieceLock { .. } => {
                    locks += 1;
                    let used = self.game.stats().inputs - self.inputs_at_piece_start;
                    self.inputs_per_piece.push(used);
                    self.inputs_at_piece_start = self.game.stats().inputs;
                }
                EventKind::TopOut { .. } => topped = true,
                _ => {}
            }
            self.events.push(e);
        }
        for _ in 0..locks {
            self.checkpoints.push(self.game.snapshot());
        }
        if locks > 0 {
            self.judge();
        }
        if topped && self.status == Status::Running {
            self.status = Status::Failed(FailReason::TopOut);
        }
        if self.status == Status::Running && self.frames > self.time_limit_frames() {
            self.status = Status::Failed(FailReason::TimeUp);
        }
    }

    fn locked_placements(&self) -> Vec<TargetPlacement> {
        self.events
            .iter()
            .filter_map(|e| match e.kind {
                EventKind::PieceLock { piece, rot, x, y } => {
                    Some(TargetPlacement { piece, rot, x, y })
                }
                _ => None,
            })
            .collect()
    }

    fn judge(&mut self) {
        let locked = self.pieces_locked() as u32;
        let goal = self.problem.goal.clone();
        let done_or_fail = |achieved: bool, limit: u32| {
            if achieved {
                Status::Success
            } else if locked >= limit {
                Status::Failed(FailReason::NotAchieved)
            } else {
                Status::Running
            }
        };
        self.status = match goal {
            Goal::Finesse { target, .. } => {
                let first = self.locked_placements()[0];
                if first.state().cell_key() == target.state().cell_key() {
                    Status::Success
                } else {
                    Status::Failed(FailReason::WrongPlacement)
                }
            }
            Goal::Placement { accepted, .. } => {
                let first = self.locked_placements()[0].state().cell_key();
                if accepted.iter().any(|a| a.state().cell_key() == first) {
                    Status::Success
                } else {
                    Status::Failed(FailReason::WrongPlacement)
                }
            }
            Goal::Stacking {
                pieces,
                max_holes,
                max_height,
                well,
            } => {
                if locked >= pieces {
                    let b = self.game.board();
                    let holes = count_holes(b);
                    let height = b.height() as u32;
                    let well_blocked = well.is_some_and(|w| b.column_height(w as usize) > 0);
                    if holes <= max_holes && height <= max_height && !well_blocked {
                        Status::Success
                    } else {
                        Status::Failed(FailReason::GoalNotMet {
                            holes,
                            height,
                            well_blocked,
                        })
                    }
                } else {
                    Status::Running
                }
            }
            Goal::Spin {
                piece,
                spin,
                lines,
                within_pieces,
            } => {
                let hit = self.events.iter().any(|e| {
                    matches!(e.kind, EventKind::Spin { piece: p, spin: s, lines: l }
                        if piece.is_none_or(|q| q == p) && s == spin && l == lines)
                });
                done_or_fail(hit, within_pieces)
            }
            Goal::PerfectClear { within_pieces } => {
                let hit = self
                    .events
                    .iter()
                    .any(|e| matches!(e.kind, EventKind::PerfectClear));
                done_or_fail(hit, within_pieces)
            }
            Goal::Attack {
                min_attack,
                within_pieces,
            } => done_or_fail(self.game.stats().attack >= min_attack, within_pieces),
            Goal::Downstack { pieces, min_lines } => {
                done_or_fail(self.game.stats().lines >= min_lines, pieces)
            }
        };
    }

    /// Undo the last locked piece (spec UI-04 "1블록 되감기"). Marks the attempt as assisted.
    pub fn rewind(&mut self) -> bool {
        if self.checkpoints.len() < 2 {
            return false;
        }
        self.checkpoints.pop();
        let cp = self.checkpoints.last().expect("checkpoint").clone();
        let n = self.checkpoints.len() as u32 - 1;
        self.game.restore(&cp);
        self.events.retain(|e| {
            e.piece_index < n
                || (e.piece_index == n && !matches!(e.kind, EventKind::PieceLock { .. }))
        });
        self.inputs_per_piece.truncate(n as usize);
        self.inputs_at_piece_start = self.game.stats().inputs;
        self.used_rewind = true;
        self.status = Status::Running;
        true
    }

    /// Restart from the beginning (retry). Retries are not counted as extra problems.
    pub fn restart(&mut self) {
        let cp = self.checkpoints[0].clone();
        self.game.restore(&cp);
        self.checkpoints.truncate(1);
        self.events.clear();
        self.inputs_per_piece.clear();
        self.inputs_at_piece_start = 0;
        self.status = Status::Running;
        self.frames = 0;
    }

    /// Next witness placement as a hint. Marks the attempt as assisted.
    pub fn hint(&mut self) -> Option<WitnessStep> {
        self.used_hint = true;
        self.problem
            .witness
            .placements
            .get(self.pieces_locked())
            .copied()
    }

    pub fn show_answer(&mut self) -> Vec<WitnessStep> {
        self.used_answer = true;
        self.problem.witness.placements.clone()
    }

    pub fn result(&self) -> AttemptResult {
        let success = self.status == Status::Success;
        let finesse_success = match &self.problem.goal {
            Goal::Finesse { optimal_inputs, .. } => Some(
                success
                    && self
                        .inputs_per_piece
                        .first()
                        .is_some_and(|&n| n <= *optimal_inputs),
            ),
            _ => None,
        };
        let assisted = self.used_hint || self.used_rewind || self.used_answer;
        AttemptResult {
            success,
            finesse_success,
            used_hint: self.used_hint,
            used_rewind: self.used_rewind,
            used_answer: self.used_answer,
            valid_for_mmr: !assisted && self.status != Status::Running && self.problem.goal.rated(),
            elapsed_frames: self.frames,
            inputs_per_piece: self.inputs_per_piece.clone(),
            reason: match &self.status {
                Status::Failed(r) => Some(r.clone()),
                _ => None,
            },
        }
    }
}
