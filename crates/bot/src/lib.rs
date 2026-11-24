//! AI opponent (STEP 0 spike version).
//!
//! * Legal moves come from the shared engine move generator (same reachability as the player).
//! * Field evaluation + shallow beam search over current/hold and the visible Next window only.
//! * Strength knobs: PPS, search depth, beam width, mistake rate, attack weight.
//!
//! STEP 5 replaces the simplified match loop in [`versus`] with the real tick engine.

pub mod versus;

use stacklab_engine::attack::{on_lock, ChainState, ClearInfo};
use stacklab_engine::board::{Board, WIDTH};
use stacklab_engine::rng::Rng;
use stacklab_engine::{lock, placements, MoveRules, Piece, Placement, Ruleset};

#[derive(Debug, Clone)]
pub struct BotConfig {
    pub name: String,
    /// Pieces per second, stored as milli-PPS to stay integer.
    pub pps_milli: u32,
    /// 0 = current piece only, 1 = also the first Next piece, ...
    pub depth: u32,
    pub beam: usize,
    /// Probability (per mille) of picking a random move from the top 5 instead of the best.
    pub mistake_pm: u32,
    pub attack_weight: i32,
}

impl BotConfig {
    pub fn preset(level: u32) -> BotConfig {
        let (pps, depth, beam, mistake, atk) = match level {
            0 => (800, 0, 1, 250, 0),
            1 => (1200, 0, 1, 100, 20),
            2 => (1600, 1, 6, 40, 40),
            3 => (2000, 1, 10, 10, 60),
            _ => (2600, 1, 14, 0, 80),
        };
        BotConfig {
            name: format!("L{level}"),
            pps_milli: pps,
            depth,
            beam,
            mistake_pm: mistake,
            attack_weight: atk,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Decision {
    pub placement: Placement,
    pub used_hold: bool,
}

/// Static field evaluation (higher is better). Integer weights.
pub fn evaluate(board: &Board) -> i32 {
    let mut heights = [0i32; WIDTH];
    for (x, h) in heights.iter_mut().enumerate() {
        *h = board.column_height(x) as i32;
    }
    let mut holes = 0;
    let mut covered = 0;
    for (x, &h) in heights.iter().enumerate() {
        for y in 0..h as usize {
            if board.rows[y] >> x & 1 == 0 {
                holes += 1;
                covered += h - y as i32 - 1;
            }
        }
    }
    let agg: i32 = heights.iter().sum();
    let max_h = *heights.iter().max().unwrap_or(&0);
    // bumpiness ignoring the deepest column (keep one well open for quads)
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
    -(agg * 5) - holes * 60 - covered * 8 - bump * 6 - danger * danger * 30
}

pub struct Bot {
    pub cfg: BotConfig,
    rules: MoveRules,
    rs: Ruleset,
    rng: Rng,
}

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
        next: &[Piece],
    ) -> Vec<(Piece, bool, Option<Piece>, usize)> {
        // (piece to place, used_hold, hold after, next pieces consumed)
        let mut v = vec![(current, false, hold, 0)];
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
        let mut s = evaluate(&nb) + atk.total() as i32 * self.cfg.attack_weight;
        if res.lines > 0 && atk.total() == 0 && pending == 0 {
            s -= 25 * res.lines as i32; // discourage burning lines for nothing
        }
        if pending > 0 && res.lines > 0 {
            s += 30 * res.lines as i32;
        }
        if ch.b2b > chain.b2b {
            s += 40;
        }
        (s, nb, ch)
    }

    /// Choose a move using only current, hold and the visible Next pieces.
    pub fn decide(
        &mut self,
        board: &Board,
        current: Piece,
        hold: Option<Piece>,
        next: &[Piece],
        chain: &ChainState,
        pending: u32,
    ) -> Option<Decision> {
        let mut first: Vec<(i32, Decision, Board, ChainState, Option<Piece>, usize)> = Vec::new();
        for (piece, used_hold, nhold, consumed) in Self::options(current, hold, next) {
            for pl in placements(board, piece, &self.rules) {
                let (s, nb, ch) = self.score_move(board, &pl, chain, pending);
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
        if first.is_empty() {
            return None;
        }
        first.sort_by_key(|c| std::cmp::Reverse(c.0));
        if self.cfg.depth >= 1 && !next.is_empty() {
            let beam = self.cfg.beam.min(first.len());
            for cand in first.iter_mut().take(beam) {
                let (_, _, ref nb, ref ch, nhold, consumed) = *cand;
                let rest = &next[consumed..];
                let Some((&cur2, after)) = rest.split_first() else {
                    continue;
                };
                let mut best2 = i32::MIN;
                for (p2, _, _, _) in Self::options(cur2, nhold, after) {
                    for pl2 in placements(nb, p2, &self.rules) {
                        let (s2, _, _) = self.score_move(nb, &pl2, ch, pending);
                        best2 = best2.max(s2);
                    }
                }
                if best2 > i32::MIN {
                    cand.0 = (cand.0 + best2) / 2;
                }
            }
            first[..beam].sort_by_key(|c| std::cmp::Reverse(c.0));
        }
        let pick = if self.rng.chance_pm(self.cfg.mistake_pm) {
            self.rng.below(first.len().min(5) as u64) as usize
        } else {
            0
        };
        Some(first.swap_remove(pick).1)
    }
}
