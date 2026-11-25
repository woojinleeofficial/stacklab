//! UI-05: AI match (user vs bot) on the shared engine. The bot presses real keys through
//! [`stacklab_bot::player::BotPlayer`]; both sides advance in lock-step at 60 Hz.

use crate::draw::{field, mini_piece, FieldView};
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Vec2};
use stacklab_bot::player::{bot_handling, BotPlayer};
use stacklab_bot::BotConfig;
use stacklab_engine::board::Board;
use stacklab_engine::events::EventKind;
use stacklab_engine::game::{Handling, InputEvent};
use stacklab_engine::replay::VersusReplay;
use stacklab_engine::versus::Versus;
use stacklab_engine::Ruleset;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    Mini(u8),
    /// Evaluation round (1-based index of non-void rounds played so far + 1)
    Evaluation,
    /// A deferred mini match played later (index 0/1)
    DeferredMini(usize),
}

pub struct MatchPlay {
    pub kind: MatchKind,
    pub versus: Versus,
    pub bot: BotPlayer,
    pub bot_id: String,
    pub bot_rating: f64,
    pub bot_rd: f64,
    pub expected: f64,
    pub in_band: bool,
    pub sim_time: f64,
    pub pending: Vec<InputEvent>,
    pub user_events: Vec<EventKind>,
    pub user_boards: Vec<Board>,
    pub finished: Option<bool>,
    /// Replay of this round (stored with the round, spec 10.1).
    pub log: VersusReplay,
    /// Match id in the store.
    pub match_id: String,
}

impl MatchPlay {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kind: MatchKind,
        rs: Arc<Ruleset>,
        handling: Handling,
        cfg: BotConfig,
        bot_rating: f64,
        bot_rd: f64,
        expected: f64,
        in_band: bool,
        seed: u64,
    ) -> MatchPlay {
        let bot = BotPlayer::new(cfg.clone(), &rs, seed ^ 0xb07);
        let seeds = [seed, seed.rotate_left(17) ^ 0x51ed];
        let hs = [handling, bot_handling()];
        let versus = Versus::new(rs, seeds, hs);
        let log = VersusReplay::start(&versus, seeds, hs, [None, None]);
        MatchPlay {
            kind,
            log,
            match_id: String::new(),
            versus,
            bot,
            bot_id: cfg.id,
            bot_rating,
            bot_rd,
            expected,
            in_band,
            sim_time: 0.0,
            pending: Vec::new(),
            user_events: Vec::new(),
            user_boards: Vec::new(),
            finished: None,
        }
    }

    /// Advances the simulation to wall-clock `now` (fixed 60 Hz, at most 8 frames per call).
    pub fn advance(&mut self, now: f64) {
        if self.finished.is_some() {
            return;
        }
        if self.sim_time == 0.0 {
            self.sim_time = now;
        }
        let mut steps = 0;
        while self.sim_time + 1.0 / 60.0 <= now && steps < 8 {
            let user = std::mem::take(&mut self.pending);
            let bot = self.bot.frame_inputs(&self.versus.players[1]);
            self.log.step(&mut self.versus, [&user, &bot]);
            for e in self.versus.players[0].events() {
                if matches!(e.kind, EventKind::PieceLock { .. }) {
                    self.user_boards
                        .push(self.versus.players[0].board().clone());
                }
                self.user_events.push(e.kind);
            }
            let _ = self.versus.players[1].events();
            self.sim_time += 1.0 / 60.0;
            steps += 1;
            if let Some(w) = self.versus.winner() {
                self.finished = Some(w == 0);
                return;
            }
        }
        if steps == 8 {
            self.sim_time = now;
        }
    }

    /// Draws both fields with garbage meters, hold and next for the user.
    pub fn draw(
        &self,
        ui: &mut egui::Ui,
        header: &str,
        you: &str,
        pending_label: &dyn Fn(u32) -> String,
    ) {
        ui.label(RichText::new(header).strong());
        let avail = ui.available_rect_before_wrap();
        let painter = ui.painter();
        let half = avail.width() / 2.0;
        for (i, g) in self.versus.players.iter().enumerate() {
            let area = Rect::from_min_size(
                Pos2::new(avail.left() + half * i as f32, avail.top() + 24.0),
                Vec2::new(half, avail.height() - 28.0),
            );
            let inner = area.shrink2(Vec2::new(60.0, 4.0));
            let cell = field(
                painter,
                inner,
                &FieldView {
                    board: g.board(),
                    active: g.active_piece(),
                    ghost: if i == 0 { g.ghost() } else { None },
                    targets: vec![],
                    colored: vec![],
                    colors: Some(g.colors()),
                },
            );
            // garbage meter (left of the field)
            let fw = cell * 10.0;
            let left = inner.center().x - fw / 2.0;
            let bottom = inner.top() + cell * 22.0;
            let pending = g.pending_garbage();
            let h = (pending as f32 * cell).min(cell * 20.0);
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(left - 10.0, bottom - h),
                    Pos2::new(left - 4.0, bottom),
                ),
                1.0,
                Color32::from_rgb(230, 70, 70),
            );
            painter.text(
                Pos2::new(left, inner.top() - 4.0),
                egui::Align2::LEFT_BOTTOM,
                if i == 0 {
                    you.to_string()
                } else {
                    format!("AI {}", self.bot_id)
                },
                egui::FontId::proportional(14.0),
                Color32::LIGHT_GRAY,
            );
            if pending > 0 {
                painter.text(
                    Pos2::new(left - 12.0, bottom - h - 2.0),
                    egui::Align2::RIGHT_BOTTOM,
                    pending_label(pending),
                    egui::FontId::proportional(12.0),
                    Color32::from_rgb(230, 120, 120),
                );
            }
            if i == 0 {
                if let Some(hp) = g.hold_piece() {
                    mini_piece(
                        painter,
                        Pos2::new(left - 35.0, inner.top() + 30.0),
                        cell * 0.55,
                        hp,
                        false,
                    );
                }
                for (k, p) in g.next_pieces().into_iter().enumerate() {
                    mini_piece(
                        painter,
                        Pos2::new(left + fw + 30.0, inner.top() + 20.0 + k as f32 * cell * 2.2),
                        cell * 0.5,
                        p,
                        false,
                    );
                }
                let c = g.chain();
                if c.b2b >= 1 {
                    painter.text(
                        Pos2::new(left + fw + 8.0, bottom),
                        egui::Align2::LEFT_BOTTOM,
                        format!("B2B ×{}", c.b2b),
                        egui::FontId::proportional(14.0),
                        Color32::from_rgb(240, 200, 90),
                    );
                }
            }
        }
    }
}
