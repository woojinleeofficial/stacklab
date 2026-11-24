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
