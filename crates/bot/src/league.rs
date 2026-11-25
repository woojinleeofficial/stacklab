//! Bot league (spec 7.3): adjacent presets play each other, ratings on the Glicko-2 scale with
//! the reference preset fixed at 1500.

use crate::player::{bot_handling, BotPlayer};
use crate::BotConfig;
use stacklab_engine::versus::Versus;
use stacklab_engine::Ruleset;
use stacklab_training::glicko::Rating;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoundOutcome {
    pub winner: usize,
    pub frames: u64,
    pub pieces: [u32; 2],
    pub attack: [u32; 2],
    /// Safety cap reached (decided by stack height) — should be rare.
    pub capped: bool,
}

pub const ROUND_FRAME_CAP: u64 = 60 * 60 * 12;

/// One round between two bots on the real engine (both sides press keys).
pub fn play_round(a: &BotConfig, b: &BotConfig, rs: &Arc<Ruleset>, seed: u64) -> RoundOutcome {
    let mut v = Versus::new(
        rs.clone(),
        [seed, seed ^ 0xa5a5_5a5a],
        [bot_handling(), bot_handling()],
    );
    let mut pa = BotPlayer::new(a.clone(), rs, seed.wrapping_mul(3));
    let mut pb = BotPlayer::new(b.clone(), rs, seed.wrapping_mul(5).wrapping_add(1));
    let mut frames = 0;
    while v.winner().is_none() && frames < ROUND_FRAME_CAP {
        let ia = pa.frame_inputs(&v.players[0]);
        let ib = pb.frame_inputs(&v.players[1]);
        v.step([&ia, &ib]);
        frames += 1;
    }
    let (winner, capped) = match v.winner() {
        Some(w) => (w, false),
        None => (
            usize::from(v.players[0].board().height() > v.players[1].board().height()),
            true,
        ),
    };
    RoundOutcome {
        winner,
        frames,
        pieces: [v.players[0].stats().pieces, v.players[1].stats().pieces],
        attack: [v.players[0].stats().attack, v.players[1].stats().attack],
        capped,
    }
}

/// A round where side 0 is "the user": also returns side 0's engine events and the board after
/// each of its locks (for observations, spec 7.4) and the full replay.
pub fn play_round_observed(
    user: &BotConfig,
    opponent: &BotConfig,
    rs: &Arc<Ruleset>,
    seed: u64,
) -> (
    RoundOutcome,
    Vec<stacklab_engine::events::EventKind>,
    Vec<stacklab_engine::Board>,
) {
    let mut v = Versus::new(
        rs.clone(),
        [seed, seed ^ 0xa5a5_5a5a],
        [bot_handling(), bot_handling()],
    );
    let mut pa = BotPlayer::new(user.clone(), rs, seed.wrapping_mul(3));
    let mut pb = BotPlayer::new(opponent.clone(), rs, seed.wrapping_mul(5).wrapping_add(1));
    let (mut events, mut boards) = (Vec::new(), Vec::new());
    let mut frames = 0;
    while v.winner().is_none() && frames < ROUND_FRAME_CAP {
        let ia = pa.frame_inputs(&v.players[0]);
        let ib = pb.frame_inputs(&v.players[1]);
        v.step([&ia, &ib]);
        for e in v.players[0].events() {
            if matches!(e.kind, stacklab_engine::events::EventKind::PieceLock { .. }) {
                boards.push(v.players[0].board().clone());
            }
            events.push(e.kind);
        }
        let _ = v.players[1].events();
        frames += 1;
    }
    let (winner, capped) = match v.winner() {
        Some(w) => (w, false),
        None => (
            usize::from(v.players[0].board().height() > v.players[1].board().height()),
            true,
        ),
    };
    let o = RoundOutcome {
        winner,
        frames,
        pieces: [v.players[0].stats().pieces, v.players[1].stats().pieces],
        attack: [v.players[0].stats().attack, v.players[1].stats().attack],
        capped,
    };
    (o, events, boards)
}

/// Game record: (preset i, preset j, score of i).
pub type Game = (usize, usize, f64);

/// Ratings with preset `reference` fixed at 1500, on the Glicko-2 logistic scale
/// (expected score = 1 / (1 + exp(-(R_i - R_j) / 173.7178))).
///
/// Fit: Bradley–Terry maximum likelihood with Hunter's MM iteration (always converges, unlike
/// chaining single Glicko-2 periods, which diverged on a 96 % pair). Half a pseudo-win is added
/// to both sides of every pair so sweeps stay finite. RD = 173.7178 / sqrt(Fisher information)
/// with the other presets held at their estimates.
pub fn ratings(n: usize, reference: usize, games: &[Game]) -> Vec<Rating> {
    use stacklab_training::glicko::SCALE;
    let mut wins = vec![vec![0.0f64; n]; n];
    for &(a, b, s) in games {
        wins[a][b] += s;
        wins[b][a] += 1.0 - s;
    }
    for i in 0..n {
        for j in 0..n {
            if i != j && wins[i][j] + wins[j][i] > 0.0 {
                wins[i][j] += 0.5;
            }
        }
    }
    let games_between = |i: usize, j: usize| wins[i][j] + wins[j][i];
    let mut pi = vec![1.0f64; n];
    for _ in 0..10_000 {
        let mut next = pi.clone();
        for i in 0..n {
            let w: f64 = (0..n).map(|j| wins[i][j]).sum();
            let d: f64 = (0..n)
                .filter(|&j| j != i && games_between(i, j) > 0.0)
                .map(|j| games_between(i, j) / (pi[i] + pi[j]))
                .sum();
            if d > 0.0 {
                next[i] = w / d;
            }
        }
        let norm = next[reference];
        for x in next.iter_mut() {
            *x /= norm;
        }
        let delta: f64 = next
            .iter()
            .zip(&pi)
            .map(|(a, b)| (a.ln() - b.ln()).abs())
            .fold(0.0, f64::max);
        pi = next;
        if delta < 1e-10 {
            break;
        }
    }
    (0..n)
        .map(|i| {
            let rating = 1500.0 + SCALE * pi[i].ln();
            if i == reference {
                return Rating::new(1500.0, 30.0, 0.06);
            }
            let info: f64 = (0..n)
                .filter(|&j| j != i)
                .map(|j| {
                    let p = pi[i] / (pi[i] + pi[j]);
                    games_between(i, j) * p * (1.0 - p)
                })
                .sum();
            let rd = if info > 0.0 {
                (SCALE / info.sqrt()).min(350.0)
            } else {
                350.0
            };
            Rating::new(rating, rd, 0.06)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn league_ratings_recover_order_and_survive_sweeps() {
        // synthetic: preset k beats k-1 75 % of the time
        let mut games = Vec::new();
        for k in 1..4 {
            for g in 0..200 {
                games.push((k, k - 1, if g % 4 != 0 { 1.0 } else { 0.0 }));
            }
        }
        let r = ratings(4, 1, &games);
        assert_eq!(r[1].rating, 1500.0);
        assert!(
            r[0].rating < r[1].rating && r[1].rating < r[2].rating && r[2].rating < r[3].rating,
            "{r:?}"
        );
        // 75 % -> about 191 points per step
        assert!((r[2].rating - r[1].rating - 191.0).abs() < 40.0, "{r:?}");
        assert!(r[3].rd < 120.0);
        // a 96 % / 100 % pair stays finite and ordered
        let mut g2 = games.clone();
        for _ in 0..200 {
            g2.push((4, 3, 1.0));
        }
        let r2 = ratings(5, 1, &g2);
        assert!(r2[4].rating > r2[3].rating && r2[4].rating.is_finite());
    }

    #[test]
    fn bot_round_runs_on_real_engine() {
        let rs = Arc::new(Ruleset::tl_s2());
        let strong = BotConfig::preset("L6");
        let weak = BotConfig::preset("L0");
        let o = play_round(&strong, &weak, &rs, 3);
        assert!(o.pieces[0] > 10 && o.pieces[1] > 5, "{o:?}");
        assert!(!o.capped);
    }
}
