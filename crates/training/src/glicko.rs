//! Glicko-2 (Glickman, "Example of the Glicko-2 system", 2012), as used by TETRA LEAGUE.
//! Ratings are shown as R ± RD; one rating period = one cycle (skills, openers) or one
//! evaluation set (match rating), spec 6.1.

use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

pub const SCALE: f64 = 173.7178;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rating {
    pub rating: f64,
    pub rd: f64,
    pub volatility: f64,
}

/// One game in a rating period: opponent rating/RD and score (1 win, 0 loss).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Outcome {
    pub opp_rating: f64,
    pub opp_rd: f64,
    pub score: f64,
}

fn g(phi: f64) -> f64 {
    1.0 / (1.0 + 3.0 * phi * phi / (PI * PI)).sqrt()
}

/// Expected score of `a` against an opponent (rating, rd).
pub fn expected(a_rating: f64, opp_rating: f64, opp_rd: f64) -> f64 {
    let mu = (a_rating - 1500.0) / SCALE;
    let mu_j = (opp_rating - 1500.0) / SCALE;
    1.0 / (1.0 + (-g(opp_rd / SCALE) * (mu - mu_j)).exp())
}

/// Opponent rating at which `a` succeeds with probability `p` (problem difficulty for a target
/// success rate). With a small RD this is about R − 191 for p = 0.75 (spec 6.2).
pub fn rating_for_success(a_rating: f64, opp_rd: f64, p: f64) -> f64 {
    let gg = g(opp_rd / SCALE);
    a_rating - SCALE * (p / (1.0 - p)).ln() / gg
}

impl Rating {
    pub fn new(rating: f64, rd: f64, volatility: f64) -> Rating {
        Rating {
            rating,
            rd,
            volatility,
        }
    }

    pub fn is_provisional(&self, threshold: f64) -> bool {
        self.rd > threshold
    }

    /// Glicko-2 update for one rating period. With no games the rating and RD are unchanged
    /// (inactivity is handled separately, TETR.IO style: see [`Rating::apply_inactivity`]).
    pub fn update(&self, games: &[Outcome], tau: f64, rd_min: f64, rd_max: f64) -> Rating {
        if games.is_empty() {
            return *self;
        }
        let mu = (self.rating - 1500.0) / SCALE;
        let phi = self.rd / SCALE;
        let sigma = self.volatility;
        let mut v_inv = 0.0;
        let mut delta_sum = 0.0;
        for o in games {
            let mu_j = (o.opp_rating - 1500.0) / SCALE;
            let gj = g(o.opp_rd / SCALE);
            let e = 1.0 / (1.0 + (-gj * (mu - mu_j)).exp());
            v_inv += gj * gj * e * (1.0 - e);
            delta_sum += gj * (o.score - e);
        }
        let v = 1.0 / v_inv;
        let delta = v * delta_sum;
        // volatility (Illinois algorithm, step 5)
        let a = (sigma * sigma).ln();
        let f = |x: f64| {
            let ex = x.exp();
            ex * (delta * delta - phi * phi - v - ex) / (2.0 * (phi * phi + v + ex).powi(2))
                - (x - a) / (tau * tau)
        };
        let eps = 1e-6;
        let mut big_a = a;
        let mut big_b = if delta * delta > phi * phi + v {
            (delta * delta - phi * phi - v).ln()
        } else {
            let mut k = 1.0;
            while f(a - k * tau) < 0.0 {
                k += 1.0;
            }
            a - k * tau
        };
        let mut fa = f(big_a);
        let mut fb = f(big_b);
        let mut iters = 0;
        while (big_b - big_a).abs() > eps && iters < 200 {
            let c = big_a + (big_a - big_b) * fa / (fb - fa);
            let fc = f(c);
            if fc * fb <= 0.0 {
                big_a = big_b;
                fa = fb;
            } else {
                fa /= 2.0;
            }
            big_b = c;
            fb = fc;
            iters += 1;
        }
        let sigma_new = (big_a / 2.0).exp();
        let phi_star = (phi * phi + sigma_new * sigma_new).sqrt();
        let phi_new = 1.0 / (1.0 / (phi_star * phi_star) + 1.0 / v).sqrt();
        let mu_new = mu + phi_new * phi_new * delta_sum;
        Rating {
            rating: mu_new * SCALE + 1500.0,
            rd: (phi_new * SCALE).clamp(rd_min, rd_max),
            volatility: sigma_new,
        }
    }

    /// TETR.IO-style inactivity: after `grace_days`, RD grows by `per_day` per idle day up to
    /// `rd_max`; the rating itself never changes.
    pub fn apply_inactivity(
        &self,
        idle_days: u32,
        grace_days: u32,
        per_day: f64,
        rd_max: f64,
    ) -> Rating {
        let days = idle_days.saturating_sub(grace_days) as f64;
        Rating {
            rd: (self.rd + days * per_day).min(rd_max),
            ..*self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The worked example from Glickman's paper.
    #[test]
    fn glickman_example() {
        let p = Rating::new(1500.0, 200.0, 0.06);
        let games = [
            Outcome {
                opp_rating: 1400.0,
                opp_rd: 30.0,
                score: 1.0,
            },
            Outcome {
                opp_rating: 1550.0,
                opp_rd: 100.0,
                score: 0.0,
            },
            Outcome {
                opp_rating: 1700.0,
                opp_rd: 300.0,
                score: 0.0,
            },
        ];
        let r = p.update(&games, 0.5, 0.0, 1000.0);
        assert!((r.rating - 1464.06).abs() < 0.01, "{}", r.rating);
        assert!((r.rd - 151.52).abs() < 0.01, "{}", r.rd);
        assert!((r.volatility - 0.05999).abs() < 0.00001, "{}", r.volatility);
    }

    #[test]
    fn target_success_difficulty_is_about_r_minus_191() {
        let d = rating_for_success(1600.0, 0.0, 0.75);
        assert!((1600.0 - d - 190.85).abs() < 0.1, "{d}");
        assert!((expected(1600.0, d, 0.0) - 0.75).abs() < 1e-9);
        // larger problem RD -> must be easier to keep 75 %
        assert!(rating_for_success(1600.0, 200.0, 0.75) < d);
    }

    #[test]
    fn inactivity_only_raises_rd() {
        let r = Rating::new(1650.0, 80.0, 0.06);
        assert_eq!(r.apply_inactivity(5, 7, 1.0, 350.0), r);
        let later = r.apply_inactivity(37, 7, 1.0, 350.0);
        assert_eq!(later.rating, 1650.0);
        assert_eq!(later.rd, 110.0);
        assert_eq!(r.apply_inactivity(10_000, 7, 1.0, 350.0).rd, 350.0);
        // empty rating period: no change
        assert_eq!(r.update(&[], 0.5, 58.0, 350.0), r);
    }
}
