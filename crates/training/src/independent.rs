//! Independent skill test (spec 6.4): offered every N sets (policy `independent_test_every_sets`),
//! runnable any time. Fixed composition (2 problems per module A–H), fixed difficulty
//! (D = 1500, the anchor rating) and the policy time limits, with problems not served before.
//! Results never touch skill ratings, cycle counts or scheduler weights; they are only compared
//! with earlier tests.

use crate::problem::{Module, Problem};
use crate::session::AttemptResult;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const PER_MODULE: u32 = 2;
pub const DIFFICULTY: f64 = 1500.0;
/// Seed range disjoint from the anchor set and from the cycle seed stream.
pub const SEED_BASE: u64 = 0x1D7E_5700_0000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndependentTest {
    /// Test number (1-based); selects a fresh seed block.
    pub number: u32,
    pub plan: Vec<Module>,
    pub results: Vec<(Module, bool, f64)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndependentResult {
    pub number: u32,
    pub set_number: u32,
    pub created_at: String,
    /// module -> (attempts, successes)
    pub per_module: BTreeMap<Module, (u32, u32)>,
    pub success_rate: f64,
    pub mean_time_share: f64,
}

impl IndependentTest {
    pub fn new(number: u32) -> IndependentTest {
        let plan = Module::ALL
            .iter()
            .flat_map(|&m| std::iter::repeat_n(m, PER_MODULE as usize))
            .collect();
        IndependentTest {
            number,
            plan,
            results: Vec::new(),
        }
    }

    /// Next (module, seed) to generate at [`DIFFICULTY`]; None when complete.
    pub fn next(&self) -> Option<(Module, u64)> {
        let i = self.results.len();
        let m = *self.plan.get(i)?;
        Some((m, SEED_BASE + self.number as u64 * 10_007 + i as u64 * 131))
    }

    pub fn record(&mut self, p: &Problem, r: &AttemptResult) {
        // hints / rewind / answer make the attempt a failure for the measurement
        let ok = r.success && r.valid_for_mmr;
        let share = (r.elapsed_frames as f64 / (p.time_limit_secs.max(1) as f64 * 60.0)).min(1.0);
        self.results.push((p.module, ok, share));
    }

    pub fn is_complete(&self) -> bool {
        self.results.len() >= self.plan.len()
    }

    pub fn finish(&self, set_number: u32, created_at: String) -> IndependentResult {
        let mut per_module: BTreeMap<Module, (u32, u32)> = BTreeMap::new();
        for (m, ok, _) in &self.results {
            let e = per_module.entry(*m).or_default();
            e.0 += 1;
            e.1 += *ok as u32;
        }
        let n = self.results.len().max(1) as f64;
        IndependentResult {
            number: self.number,
            set_number,
            created_at,
            per_module,
            success_rate: self.results.iter().filter(|r| r.1).count() as f64 / n,
            mean_time_share: self.results.iter().map(|r| r.2).sum::<f64>() / n,
        }
    }
}

/// Offered when `every` sets have been completed since the last test (or since the start).
pub fn due(history: &[IndependentResult], completed_sets: u32, every: u32) -> bool {
    let last = history.iter().map(|h| h.set_number).max().unwrap_or(0);
    every > 0 && completed_sets >= last + every
}

/// Change of the success rate (percentage points) and per-module success deltas against the
/// previous test.
pub fn compare(prev: &IndependentResult, cur: &IndependentResult) -> (f64, BTreeMap<Module, i32>) {
    let mut d = BTreeMap::new();
    for (m, (_, s)) in &cur.per_module {
        let p = prev.per_module.get(m).map_or(0, |x| x.1);
        d.insert(*m, *s as i32 - p as i32);
    }
    (100.0 * (cur.success_rate - prev.success_rate), d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_is_fixed_and_seeds_are_fresh_per_test() {
        let a = IndependentTest::new(1);
        let b = IndependentTest::new(2);
        assert_eq!(a.plan.len(), 16);
        assert_eq!(a.plan, b.plan, "same composition every time");
        assert_ne!(a.next(), b.next(), "new problems every time");
        assert_ne!(
            a.next().unwrap().1 & !0xFFFF_FFFF,
            crate::service::ANCHOR_SEED_BASE & !0xFFFF_FFFF
        );
    }

    #[test]
    fn due_every_three_sets() {
        let r = |s| IndependentResult {
            number: 1,
            set_number: s,
            created_at: String::new(),
            per_module: BTreeMap::new(),
            success_rate: 0.5,
            mean_time_share: 0.5,
        };
        assert!(!due(&[], 2, 3));
        assert!(due(&[], 3, 3));
        assert!(!due(&[r(3)], 5, 3));
        assert!(due(&[r(3)], 6, 3));
        let (pp, _) = compare(
            &r(3),
            &IndependentResult {
                success_rate: 0.75,
                ..r(6)
            },
        );
        assert!((pp - 25.0).abs() < 1e-9);
    }
}
