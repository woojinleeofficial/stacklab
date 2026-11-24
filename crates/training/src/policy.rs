//! Policy values from `config/policy.toml` (spec ch.16). Unknown keys are ignored so later
//! steps can add sections without breaking older readers.

use crate::problem::Module;
use serde::Deserialize;
use std::collections::BTreeMap;

pub const POLICY_TOML: &str = include_str!("../../../config/policy.toml");

#[derive(Debug, Clone, Deserialize)]
pub struct Policy {
    pub policy_version: u32,
    pub cycle: CyclePolicy,
    pub rating: RatingPolicy,
    pub difficulty: DifficultyPolicy,
    pub scheduler: SchedulerPolicy,
    pub time_limits: BTreeMap<String, TimeBands>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CyclePolicy {
    pub problems_per_cycle: u32,
    pub cycles_per_set: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RatingPolicy {
    pub initial_rating: f64,
    pub initial_rd: f64,
    pub initial_volatility: f64,
    pub tau: f64,
    pub provisional_rd: f64,
    pub rd_max: f64,
    pub rd_min: f64,
    pub inactivity_grace_days: u32,
    pub inactivity_rd_per_day: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DifficultyPolicy {
    pub target_success: f64,
    pub jitter_rating: f64,
    pub bucket_rd_floor: f64,
    pub anchor_rating: f64,
    pub anchor_rd: f64,
    pub anchor_share: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SchedulerPolicy {
    pub weight_min: f64,
    pub weight_max: f64,
    pub max_per_domain: u32,
    pub min_domains: u32,
    pub recent_window_problems: u32,
    pub duplicate_window: usize,
    #[serde(default)]
    pub domain_floor: BTreeMap<String, u32>,
    #[serde(default)]
    pub opener_mix: BTreeMap<String, u32>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct TimeBands {
    pub easy: u32,
    pub normal: u32,
    pub hard: u32,
}

impl Policy {
    pub fn from_toml(s: &str) -> Result<Policy, String> {
        toml::from_str(s).map_err(|e| e.to_string())
    }

    pub fn bundled() -> Policy {
        Self::from_toml(POLICY_TOML).expect("bundled policy.toml must parse")
    }

    /// Time limit (seconds) for a module at a difficulty rating.
    /// Bands: easy < 1300 <= normal < 1700 <= hard.
    pub fn time_limit_secs(&self, module: Module, difficulty: f64) -> u32 {
        let b = self
            .time_limits
            .get(module.policy_key())
            .copied()
            .unwrap_or(TimeBands {
                easy: 60,
                normal: 60,
                hard: 60,
            });
        if difficulty < 1300.0 {
            b.easy
        } else if difficulty < 1700.0 {
            b.normal
        } else {
            b.hard
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_policy_parses_with_spec_defaults() {
        let p = Policy::bundled();
        assert_eq!(p.cycle.problems_per_cycle, 12);
        assert_eq!(p.cycle.cycles_per_set, 3);
        assert_eq!(p.rating.initial_rating, 1500.0);
        assert_eq!(p.rating.initial_rd, 350.0);
        assert_eq!(p.difficulty.target_success, 0.75);
        assert_eq!(p.scheduler.domain_floor.get("D"), Some(&2));
        for m in Module::ALL {
            assert!(p.time_limits.contains_key(m.policy_key()), "{m:?}");
        }
        assert_eq!(p.time_limit_secs(Module::A, 1500.0), 15);
    }
}
