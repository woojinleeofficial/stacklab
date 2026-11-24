//! Bot league (spec 7.3): adjacent presets play each other, ratings on the Glicko-2 scale with
//! the reference preset fixed at 1500.

use crate::player::{bot_handling, BotPlayer};
use crate::BotConfig;
use stacklab_engine::versus::Versus;
use stacklab_engine::Ruleset;
use stacklab_training::glicko::{Outcome, Rating};
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

/// Game record: (preset i, preset j, score of i).
pub type Game = (usize, usize, f64);

/// Ratings with preset `reference` fixed at 1500 (± small RD). Each iteration gives every other
/// preset a Glicko-2 rating period over all its games, centred on its current estimate with a
/// wide RD (a Newton step towards the maximum-likelihood rating), against the opponents'
/// current estimates. Converges to the joint fit; the final RD is the update's RD.
pub fn ratings(n: usize, reference: usize, games: &[Game]) -> Vec<Rating> {
    let fixed = Rating::new(1500.0, 30.0, 0.06);
    let prior = Rating::new(1500.0, 350.0, 0.06);
    let mut r = vec![prior; n];
    r[reference] = fixed;
    for _ in 0..60 {
        let mut next = r.clone();
        for i in 0..n {
            if i == reference {
                continue;
            }
            let mut out = Vec::new();
            for &(a, b, s) in games {
                if a == i {
                    out.push(Outcome {
                        opp_rating: r[b].rating,
                        opp_rd: r[b].rd,
                        score: s,
                    });
                } else if b == i {
                    out.push(Outcome {
                        opp_rating: r[a].rating,
                        opp_rd: r[a].rd,
                        score: 1.0 - s,
                    });
                }
            }
            let centred = Rating::new(r[i].rating, 350.0, 0.06);
            next[i] = centred.update(&out, 0.5, 1.0, 350.0);
        }
        r = next;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn league_ratings_recover_order() {
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
    }

    #[test]
    fn bot_round_runs_on_real_engine() {
        let rs = Arc::new(Ruleset::tl_s2());
        let strong = BotConfig::preset("L5");
        let weak = BotConfig::preset("L0");
        let o = play_round(&strong, &weak, &rs, 3);
        assert!(o.pieces[0] > 10 && o.pieces[1] > 5, "{o:?}");
        assert!(!o.capped);
    }
}
