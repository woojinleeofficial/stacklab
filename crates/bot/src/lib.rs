//! AI opponent (spec 7.2): legal-move generation from the shared engine, field evaluation +
//! shallow search, and real key inputs executed through the tick engine ([`player`]).
//!
//! The bot decides from a [`BotView`] that only contains what a human sees on their own side:
//! board, current piece, hold, the Next window and the pending-garbage meter. It never sees
//! pieces beyond the Next window or the opponent's random state.

pub mod league;
pub mod player;

use serde::Deserialize;
use stacklab_engine::attack::{on_lock, ChainState, ClearInfo};
use stacklab_engine::board::{Board, WIDTH};
use stacklab_engine::game::Game;
use stacklab_engine::rng::Rng;
use stacklab_engine::{lock, placements, MoveRules, Piece, Placement, Ruleset};

pub const PRESETS_TOML: &str = include_str!("../../../data/bots/presets.toml");

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct BotConfig {
    pub id: String,
    /// Pieces per second x1000 (pacing between pieces).
    pub pps_milli: u32,
    /// Reaction delay before the first key of each piece.
    pub input_delay_frames: u32,
    /// 0 = current/hold only, 1 = also the next visible piece.
    pub depth: u32,
    pub beam: usize,
    /// Per-mille chance to pick a random move from the top 5 instead of the best.
    pub mistake_pm: u32,
    pub attack: i32,
    pub downstack: i32,
    pub safety: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PresetFile {
    pub reference: String,
    pub preset: Vec<BotConfig>,
}

pub fn presets() -> PresetFile {
    toml::from_str(PRESETS_TOML).expect("bundled bot presets parse")
}

impl BotConfig {
    pub fn preset(id: &str) -> BotConfig {
        presets()
            .preset
            .into_iter()
            .find(|p| p.id == id)
            .unwrap_or_else(|| panic!("unknown preset {id}"))
    }
}

/// Everything the bot may look at (its own side, visible information only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotView {
    pub board: Board,
    pub current: Piece,
    pub hold: Option<Piece>,
    pub hold_available: bool,
    pub next: Vec<Piece>,
    pub chain: ChainState,
    pub pending_garbage: u32,
}

impl BotView {
    pub fn from_game(g: &Game) -> Option<BotView> {
        Some(BotView {
            board: g.board().clone(),
            current: g.active_piece()?.piece,
            hold: g.hold_piece(),
            hold_available: !g.hold_used(),
            next: g.next_pieces(),
            chain: g.chain(),
            pending_garbage: g.pending_garbage(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct Decision {
    pub placement: Placement,
    pub used_hold: bool,
}

/// Static field evaluation (higher is better). `safety` scales the height danger term.
pub fn evaluate(board: &Board, safety: i32) -> i32 {
    let mut heights = [0i32; WIDTH];
    for (x, h) in heights.iter_mut().enumerate() {
        *h = board.column_height(x) as i32;
    }
    let mut holes = 0;
    let mut covered = 0;
    for (x, &h) in heights.iter().enumerate() {
        for (y, row) in board.rows.iter().enumerate().take(h as usize) {
            if row >> x & 1 == 0 {
                holes += 1;
                covered += h - y as i32 - 1;
            }
        }
    }
    let agg: i32 = heights.iter().sum();
    let max_h = *heights.iter().max().unwrap_or(&0);
    let well = (0..WIDTH).min_by_key(|&x| heights[x]).unwrap_or(0);
    let mut bump = 0;
    let mut prev: Option<i32> = None;
    for (x, &h) in heights.iter().enumerate() {
        if x == well {
            continue;
        }
        if let Some(p) = prev {
            bump += (h - p).abs();
        }
        prev = Some(h);
    }
    let danger = (max_h - 10).max(0);
    -(agg * 5) - holes * 60 - covered * 8 - bump * 6 - danger * danger * safety
}

pub struct Bot {
    pub cfg: BotConfig,
    rules: MoveRules,
    rs: Ruleset,
    rng: Rng,
}

type Candidate = (i32, Decision, Board, ChainState, Option<Piece>, usize);

impl Bot {
    pub fn new(cfg: BotConfig, rs: &Ruleset, seed: u64) -> Bot {
        Bot {
            cfg,
            rules: MoveRules::from_ruleset(rs),
            rs: rs.clone(),
            rng: Rng::new(seed),
        }
    }

    fn options(
        current: Piece,
        hold: Option<Piece>,
        hold_available: bool,
        next: &[Piece],
    ) -> Vec<(Piece, bool, Option<Piece>, usize)> {
        let mut v = vec![(current, false, hold, 0)];
        if !hold_available {
            return v;
        }
        match hold {
            Some(h) if h != current => v.push((h, true, Some(current), 0)),
            Some(_) => {}
            None => {
                if let Some(&n) = next.first() {
                    v.push((n, true, Some(current), 1));
                }
            }
        }
        v
    }

    fn score_move(
        &self,
        board: &Board,
        pl: &Placement,
        chain: &ChainState,
        pending: u32,
    ) -> (i32, Board, ChainState) {
        let mut nb = board.clone();
        let res = lock(&mut nb, pl);
        let mut ch = *chain;
        let atk = on_lock(
            &self.rs,
            &mut ch,
            &ClearInfo {
                piece: pl.state.piece,
                lines: res.lines,
                spin: pl.spin,
                perfect_clear: res.perfect_clear,
            },
        );
        let mut s = evaluate(&nb, self.cfg.safety) + atk.total() as i32 * self.cfg.attack;
        if res.lines > 0 && atk.total() == 0 && pending == 0 {
            s -= 25 * res.lines as i32;
        }
        if pending > 0 && res.lines > 0 {
            s += self.cfg.downstack * res.lines as i32;
        }
        if ch.b2b > chain.b2b {
            s += self.cfg.attack / 2;
        }
        (s, nb, ch)
    }

    /// Ranked decisions (best first) from visible information only.
    pub fn rank(&mut self, v: &BotView) -> Vec<Decision> {
        let mut first: Vec<Candidate> = Vec::new();
        for (piece, used_hold, nhold, consumed) in
            Self::options(v.current, v.hold, v.hold_available, &v.next)
        {
            for pl in placements(&v.board, piece, &self.rules) {
                let (s, nb, ch) = self.score_move(&v.board, &pl, &v.chain, v.pending_garbage);
                first.push((
                    s,
                    Decision {
                        placement: pl,
                        used_hold,
                    },
                    nb,
                    ch,
                    nhold,
                    consumed,
                ));
            }
        }
        first.sort_by_key(|c| std::cmp::Reverse(c.0));
        if self.cfg.depth >= 1 && !v.next.is_empty() {
            let beam = self.cfg.beam.min(first.len());
            for i in 0..beam {
                let (_, _, ref nb, ref ch, nhold, consumed) = first[i];
                let rest = &v.next[consumed..];
                let Some((&cur2, after)) = rest.split_first() else {
                    continue;
                };
                let mut best2 = i32::MIN;
                for (p2, _, _, _) in Self::options(cur2, nhold, true, after) {
                    for pl2 in placements(nb, p2, &self.rules) {
                        best2 = best2.max(self.score_move(nb, &pl2, ch, v.pending_garbage).0);
                    }
                }
                if best2 > i32::MIN {
                    first[i].0 = (first[i].0 + best2) / 2;
                }
            }
            first[..beam].sort_by_key(|c| std::cmp::Reverse(c.0));
        }
        if !first.is_empty() && self.rng.chance_pm(self.cfg.mistake_pm) {
            let k = self.rng.below(first.len().min(5) as u64) as usize;
            first.swap(0, k);
        }
        first.into_iter().map(|c| c.1).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holes_are_worse_than_flat() {
        let flat = Board::from_ascii(&["#########.", "#########."]);
        let holed = Board::from_ascii(&["#########.", "####.#####"]);
        assert!(evaluate(&flat, 30) > evaluate(&holed, 30));
    }

    #[test]
    fn presets_parse_and_reference_exists() {
        let p = presets();
        assert!(p.preset.len() >= 5);
        assert!(p.preset.iter().any(|x| x.id == p.reference));
        // knobs are monotone in the preset order
        for w in p.preset.windows(2) {
            assert!(w[1].pps_milli > w[0].pps_milli);
            assert!(w[1].mistake_pm <= w[0].mistake_pm);
        }
    }
}

/// League results (data/bots/calibration.toml), spec 7.3. Stored with every match record.
#[derive(Debug, Clone, Deserialize)]
pub struct Calibration {
    pub calibration_version: u32,
    pub games_per_pair: u32,
    pub reference: String,
    pub monotone: bool,
    pub bot: Vec<CalibratedBot>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct CalibratedBot {
    pub id: String,
    pub rating: f64,
    pub rd: f64,
}

pub const CALIBRATION_TOML: &str = include_str!("../../../data/bots/calibration.toml");

pub fn calibration() -> Calibration {
    toml::from_str(CALIBRATION_TOML).expect("bundled bot calibration parses")
}

#[cfg(test)]
mod visibility_tests {
    use super::*;
    use stacklab_engine::game::{Game, InitialState, Mode};
    use stacklab_engine::piece::parse_queue;
    use std::sync::Arc;

    /// Two games that differ only in pieces beyond the Next window give the bot the same view,
    /// so its decisions cannot depend on hidden information (spec 7.2).
    #[test]
    fn bot_view_excludes_hidden_pieces() {
        let rs = Arc::new(Ruleset::tl_s2());
        let mk = |q: &str| {
            let init = InitialState {
                rows: vec![0; 40],
                queue: parse_queue(q),
                hold: None,
                b2b: -1,
                combo: -1,
                continue_with_bag: false,
                incoming: vec![],
            };
            Game::with_initial(
                rs.clone(),
                1,
                crate::player::bot_handling(),
                Mode::Free,
                Some(&init),
            )
        };
        let a = mk("TIOSZJLLLLL");
        let b = mk("TIOSZJOOOOO");
        let (va, vb) = (
            BotView::from_game(&a).unwrap(),
            BotView::from_game(&b).unwrap(),
        );
        assert_eq!(va, vb);
        let cfg = BotConfig::preset("L4");
        let da = Bot::new(cfg.clone(), &rs, 9).rank(&va);
        let db = Bot::new(cfg, &rs, 9).rank(&vb);
        assert_eq!(da[0].placement, db[0].placement);
    }

    #[test]
    fn calibration_matches_presets() {
        let c = calibration();
        let p = presets();
        assert_eq!(c.reference, p.reference);
        assert_eq!(
            c.bot.iter().map(|b| b.id.clone()).collect::<Vec<_>>(),
            p.preset.iter().map(|b| b.id.clone()).collect::<Vec<_>>()
        );
        assert!(
            c.games_per_pair >= 200,
            "spec 7.3: at least 200 games per adjacent pair"
        );
        assert!(c.monotone, "ratings must follow preset strength order");
    }
}
