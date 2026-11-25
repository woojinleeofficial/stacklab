//! Adaptive layer (spec 6): skill ratings per module, difficulty-bucket calibration,
//! weighted scheduler and the 12-problem cycle as one Glicko-2 rating period.

use crate::glicko::{Outcome, Rating};
use crate::policy::Policy;
use crate::problem::Module;
use crate::session::AttemptResult;
use serde::{Deserialize, Serialize};
use stacklab_engine::rng::Rng;
use std::collections::{BTreeMap, VecDeque};

/// Anchor rating / RD (mirrors policy `difficulty.anchor_rating` / `anchor_rd`; checked by a test).
pub const ANCHOR: (f64, f64) = (1500.0, 30.0);

/// Extra uncertainty of a calibrated (non-anchor) bucket when it is used to rate the user.
pub const MODEL_RD: f64 = 150.0;

/// Initial RD of a new difficulty bucket (estimate only).
pub const BUCKET_INITIAL_RD: f64 = 350.0;

/// Anchor buckets (spec 6.2 step 1) are named "<letter>:anchor" and fixed at the anchor rating.
pub fn anchor_bucket(m: Module) -> String {
    format!("{}:anchor", m.letter())
}

pub fn is_anchor(bucket: &str) -> bool {
    bucket.ends_with(":anchor")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Calibration {
    pub version: u32,
    pub buckets: BTreeMap<String, Rating>,
    /// Generator estimate of each bucket (to learn estimate -> real difficulty offsets).
    pub estimates: BTreeMap<String, f64>,
}

impl Default for Calibration {
    fn default() -> Self {
        Calibration {
            version: 1,
            buckets: BTreeMap::new(),
            estimates: BTreeMap::new(),
        }
    }
}

impl Calibration {
    /// Current difficulty (rating, rd) of a problem: its bucket's calibrated rating when the
    /// bucket has been seen, else the generator's estimate.
    pub fn difficulty(&self, bucket: &str, estimate: f64) -> (f64, f64) {
        if is_anchor(bucket) {
            return (ANCHOR.0, ANCHOR.1);
        }
        self.buckets
            .get(bucket)
            .map_or((estimate, BUCKET_INITIAL_RD), |r| (r.rating, r.rd))
    }

    /// Mean (calibrated − estimated) over a module's buckets, weighted by certainty.
    pub fn module_offset(&self, m: Module) -> f64 {
        let prefix = format!("{}:", m.letter());
        let (mut num, mut den) = (0.0, 0.0);
        for (b, r) in self.buckets.range(prefix.clone()..) {
            if !b.starts_with(&prefix) {
                break;
            }
            if let Some(e) = self.estimates.get(b) {
                let w = 1.0 / (r.rd * r.rd);
                num += w * (r.rating - e);
                den += w;
            }
        }
        if den > 0.0 {
            num / den
        } else {
            0.0
        }
    }

    /// Generator request that should land on real difficulty `d` (spec 6.2 automatic calibration).
    pub fn request_estimate(&self, m: Module, d: f64) -> f64 {
        d - self.module_offset(m)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentResult {
    pub module: Module,
    pub tags: Vec<String>,
    pub success: bool,
    /// elapsed / time limit, in [0, 1]
    pub time_share: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillProfile {
    pub skills: BTreeMap<Module, Rating>,
    pub recent: VecDeque<RecentResult>,
    pub calibration: Calibration,
    /// Opener proficiency per opener id, independent of the loadout role (spec 6.1, 9.5).
    #[serde(default)]
    pub openers: BTreeMap<String, Rating>,
    /// Per-branch (attempts, successes) from decided opener problems.
    #[serde(default)]
    pub branch_stats: BTreeMap<String, (u32, u32)>,
    #[serde(default)]
    pub loadout: Option<stacklab_opener::loadout::OpenerLoadout>,
    /// Weight boosts from engine-observed match weaknesses (spec 7.4), halved every cycle.
    #[serde(default)]
    pub observed: BTreeMap<Module, f64>,
    /// Valid (rated) opener-domain attempts so far — recommendation data condition (spec 9.1).
    #[serde(default)]
    pub opener_valid_attempts: u32,
    /// Match rounds (minis included) with an opener observation (spec 9.1).
    #[serde(default)]
    pub opener_observed_rounds: u32,
    /// Opener success observed in matches: (rounds, successes) per opener id.
    #[serde(default)]
    pub opener_match_stats: BTreeMap<String, (u32, u32)>,
}

impl SkillProfile {
    pub fn new(policy: &Policy) -> SkillProfile {
        let r = Rating::new(
            policy.rating.initial_rating,
            policy.rating.initial_rd,
            policy.rating.initial_volatility,
        );
        SkillProfile {
            skills: Module::ALL.iter().map(|&m| (m, r)).collect(),
            recent: VecDeque::new(),
            calibration: Calibration::default(),
            openers: BTreeMap::new(),
            branch_stats: BTreeMap::new(),
            loadout: None,
            observed: BTreeMap::new(),
            opener_valid_attempts: 0,
            opener_observed_rounds: 0,
            opener_match_stats: BTreeMap::new(),
        }
    }

    pub fn skill(&self, m: Module) -> Rating {
        self.skills[&m]
    }

    /// Scheduler weights (spec 6.5 step 2): recent failures and slowness raise the weight,
    /// clamped to [weight_min, weight_max]; modules without data get 1.0.
    pub fn weights(&self, policy: &Policy) -> BTreeMap<Module, f64> {
        let mut out = BTreeMap::new();
        for m in Module::ALL {
            let rs: Vec<&RecentResult> = self.recent.iter().filter(|r| r.module == m).collect();
            let w = if rs.is_empty() {
                1.0
            } else {
                let fail = rs.iter().filter(|r| !r.success).count() as f64 / rs.len() as f64;
                let slow = rs.iter().map(|r| r.time_share).sum::<f64>() / rs.len() as f64;
                // 25 % failures and half the time used is "on target" (~1.0)
                0.8 + 1.6 * (fail - 0.25).max(-0.125) + 0.6 * (slow - 0.5).max(0.0)
            };
            let w = w + self.observed.get(&m).copied().unwrap_or(0.0);
            out.insert(
                m,
                w.clamp(policy.scheduler.weight_min, policy.scheduler.weight_max),
            );
        }
        out
    }

    /// Adds engine-observed match weaknesses to the next cycle's weights (spec 7.4).
    pub fn apply_match_observations(&mut self, rounds: &[crate::set::Observations]) {
        for o in rounds {
            if let Some(built) = o.opener_built {
                self.opener_observed_rounds += 1;
                if let Some(lo) = &self.loadout {
                    let e = self
                        .opener_match_stats
                        .entry(lo.primary.clone())
                        .or_default();
                    e.0 += 1;
                    e.1 += built as u32;
                }
            }
        }
        for (m, b) in crate::set::weakness_boosts(rounds) {
            *self.observed.entry(m).or_insert(0.0) += b;
        }
    }

    /// Weak sub-skills by tag (failure rate over the recent window), strongest first.
    pub fn weak_tags(&self, m: Module) -> Vec<(String, f64)> {
        let mut agg: BTreeMap<String, (u32, u32)> = BTreeMap::new();
        for r in self.recent.iter().filter(|r| r.module == m) {
            for t in &r.tags {
                let e = agg.entry(t.clone()).or_default();
                e.1 += 1;
                if !r.success {
                    e.0 += 1;
                }
            }
        }
        let mut v: Vec<(String, f64)> = agg
            .into_iter()
            .filter(|(_, (_, n))| *n >= 2)
            .map(|(t, (f, n))| (t, f as f64 / n as f64))
            .collect();
        v.sort_by(|a, b| b.1.partial_cmp(&a.1).expect("finite"));
        v
    }
}

/// Modules the scheduler may use: all generators; openers (E) only with a loadout.
pub fn available_for(profile: &SkillProfile) -> Vec<Module> {
    Module::ALL
        .into_iter()
        .filter(|&m| m != Module::E || profile.loadout.is_some())
        .collect()
}

/// Modules usable before an opener loadout exists (initial diagnosis).
pub fn available() -> Vec<Module> {
    Module::ALL
        .into_iter()
        .filter(|&m| m != Module::E)
        .collect()
}

/// Spec 6.5: pick 12 domains with weights, max 5 per domain, at least 3 domains, floors.
pub fn plan_cycle(profile: &SkillProfile, policy: &Policy, rng: &mut Rng) -> Vec<Module> {
    let n = policy.cycle.problems_per_cycle as usize;
    let cap = policy.scheduler.max_per_domain as usize;
    let mods = available_for(profile);
    let w = profile.weights(policy);
    let mut counts: BTreeMap<Module, usize> = BTreeMap::new();
    let mut plan = Vec::with_capacity(n);
    for (k, &floor) in &policy.scheduler.domain_floor {
        if let Some(&m) = mods.iter().find(|m| m.policy_key().starts_with(k.as_str())) {
            for _ in 0..floor.min(cap as u32) {
                plan.push(m);
                *counts.entry(m).or_default() += 1;
            }
        }
    }
    while plan.len() < n {
        let pool: Vec<(Module, f64)> = mods
            .iter()
            .filter(|m| counts.get(m).copied().unwrap_or(0) < cap)
            .map(|&m| (m, w[&m]))
            .collect();
        let total: f64 = pool.iter().map(|p| p.1).sum();
        // integer-free sampling from the deterministic RNG
        let mut x = rng.below(1_000_000) as f64 / 1_000_000.0 * total;
        let mut pick = pool[pool.len() - 1].0;
        for (m, wt) in &pool {
            if x < *wt {
                pick = *m;
                break;
            }
            x -= wt;
        }
        plan.push(pick);
        *counts.entry(pick).or_default() += 1;
    }
    // at least `min_domains` distinct domains
    let min_d = policy.scheduler.min_domains as usize;
    while counts.len() < min_d.min(mods.len()) {
        let missing = *mods
            .iter()
            .find(|m| !counts.contains_key(m))
            .expect("a missing module");
        let (&most, _) = counts.iter().max_by_key(|(_, &c)| c).expect("non-empty");
        let floor_of = |m: Module| {
            policy
                .scheduler
                .domain_floor
                .iter()
                .find(|(k, _)| m.policy_key().starts_with(k.as_str()))
                .map_or(0, |(_, &f)| f as usize)
        };
        if counts[&most] <= floor_of(most).max(1) {
            break;
        }
        let i = plan.iter().rposition(|&m| m == most).expect("present");
        plan[i] = missing;
        *counts.get_mut(&most).expect("present") -= 1;
        counts.insert(missing, 1);
    }
    for i in (1..plan.len()).rev() {
        let j = rng.below(i as u64 + 1) as usize;
        plan.swap(i, j);
    }
    plan
}

/// Initial diagnosis (spec 6.4): anchor problems per module, closed as one rating period so
/// the user's starting ratings sit on the anchor scale before calibration starts.
pub fn diagnosis_plan(per_module: usize) -> Vec<Module> {
    available()
        .into_iter()
        .flat_map(|m| std::iter::repeat_n(m, per_module))
        .collect()
}

/// Whether the next problem should come from the module's anchor set.
pub fn serve_anchor(policy: &Policy, rng: &mut Rng) -> bool {
    rng.below(1_000_000) < (policy.difficulty.anchor_share * 1_000_000.0) as u64
}

/// Spec 6.2: difficulty with 75 % expected success, plus jitter against monotony. The centre is
/// solved so that the success rate *averaged over the jitter* is the target (the logistic
/// curve is not symmetric around 75 %, so plain R − 191 ± jitter lands a few points low).
pub fn target_difficulty(profile: &SkillProfile, m: Module, policy: &Policy, rng: &mut Rng) -> f64 {
    let r = profile.skill(m);
    let j = policy.difficulty.jitter_rating;
    // both the user's rating and the problem's difficulty are uncertain: combine their RDs
    let rd = (r.rd * r.rd + policy.difficulty.bucket_rd_floor.powi(2)).sqrt();
    let base = jitter_centred(r.rating, rd, policy.difficulty.target_success, j);
    base + (rng.below(2001) as f64 / 1000.0 - 1.0) * j
}

/// Centre `d0` with mean over u ~ U[-j, j] of expected(r, d0 + u) = p (bisection).
pub fn jitter_centred(r: f64, rd: f64, p: f64, j: f64) -> f64 {
    let mean = |d0: f64| {
        let n = 41;
        (0..n)
            .map(|i| {
                crate::glicko::expected(r, d0 + j * (2.0 * i as f64 / (n - 1) as f64 - 1.0), rd)
            })
            .sum::<f64>()
            / n as f64
    };
    let (mut lo, mut hi) = (r - 2000.0, r + 2000.0);
    for _ in 0..60 {
        let mid = (lo + hi) / 2.0;
        if mean(mid) > p {
            lo = mid; // still too easy -> harder
        } else {
            hi = mid;
        }
    }
    (lo + hi) / 2.0
}

/// One answered slot of a cycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotResult {
    pub module: Module,
    pub bucket: String,
    pub estimate: f64,
    pub tags: Vec<String>,
    pub attempt: AttemptResult,
    pub time_share: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cycle {
    pub plan: Vec<Module>,
    pub results: Vec<SlotResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CycleReport {
    pub before: BTreeMap<Module, Rating>,
    pub after: BTreeMap<Module, Rating>,
    pub valid_attempts: u32,
    pub successes: u32,
}

impl Cycle {
    pub fn new(plan: Vec<Module>) -> Cycle {
        Cycle {
            plan,
            results: Vec::new(),
        }
    }

    /// The module for the next slot, None when all 12 are answered (matches may start).
    pub fn next_module(&self) -> Option<Module> {
        self.plan.get(self.results.len()).copied()
    }

    pub fn is_complete(&self) -> bool {
        self.results.len() >= self.plan.len()
    }

    /// Records the decided attempt for the current slot. Retries are not recorded here, so they
    /// never add problems (spec 7.1). When the planned module could not be served and another
    /// module was substituted (spec 5.3), the slot is re-labelled with the actual module.
    pub fn record(&mut self, r: SlotResult) -> bool {
        if self.is_complete() {
            return false;
        }
        let i = self.results.len();
        self.plan[i] = r.module;
        self.results.push(r);
        true
    }

    /// Closes the rating period: Glicko-2 update of every module skill from valid attempts,
    /// then calibration of the difficulty buckets (RD floor keeps them slow), spec 6.1/6.2.
    pub fn close(&self, profile: &mut SkillProfile, policy: &Policy) -> CycleReport {
        let before = profile.skills.clone();
        let tau = policy.rating.tau;
        let mut per_module: BTreeMap<Module, Vec<Outcome>> = BTreeMap::new();
        let mut per_bucket: BTreeMap<String, Vec<Outcome>> = BTreeMap::new();
        let (mut valid, mut succ) = (0, 0);
        for r in &self.results {
            profile.recent.push_back(RecentResult {
                module: r.module,
                tags: r.tags.clone(),
                success: r.attempt.success,
                time_share: r.time_share,
            });
            if !r.attempt.valid_for_mmr {
                continue;
            }
            valid += 1;
            succ += r.attempt.success as u32;
            let (d, d_rd) = profile.calibration.difficulty(&r.bucket, r.estimate);
            let s = if r.attempt.success { 1.0 } else { 0.0 };
            // non-anchor difficulties are themselves estimates: widen their RD for the user's
            // update so the fixed anchors set the absolute scale (spec 6.2)
            let opp_rd = if is_anchor(&r.bucket) {
                d_rd
            } else {
                (d_rd * d_rd + MODEL_RD * MODEL_RD).sqrt()
            };
            per_module.entry(r.module).or_default().push(Outcome {
                opp_rating: d,
                opp_rd,
                score: s,
            });
            let user = before[&r.module];
            per_bucket
                .entry(r.bucket.clone())
                .or_default()
                .push(Outcome {
                    opp_rating: user.rating,
                    opp_rd: user.rd,
                    score: 1.0 - s,
                });
        }
        while profile.recent.len() > policy.scheduler.recent_window_problems as usize {
            profile.recent.pop_front();
        }
        for (m, games) in per_module {
            let r = before[&m].update(&games, tau, policy.rating.rd_min, policy.rating.rd_max);
            profile.skills.insert(m, r);
        }
        // opener proficiency ratings (per opener, role-independent) and branch statistics
        let mut per_opener: BTreeMap<String, Vec<Outcome>> = BTreeMap::new();
        for r in &self.results {
            let Some(op) = r.tags.iter().find_map(|t| t.strip_prefix("opener:")) else {
                continue;
            };
            if let Some(br) = r.tags.iter().find_map(|t| t.strip_prefix("branch:")) {
                let e = profile
                    .branch_stats
                    .entry(format!("{op}/{br}"))
                    .or_default();
                e.0 += 1;
                e.1 += r.attempt.success as u32;
            }
            if r.attempt.valid_for_mmr {
                profile.opener_valid_attempts += 1;
                let (d, d_rd) = profile.calibration.difficulty(&r.bucket, r.estimate);
                let opp_rd = (d_rd * d_rd + MODEL_RD * MODEL_RD).sqrt();
                per_opener.entry(op.to_string()).or_default().push(Outcome {
                    opp_rating: d,
                    opp_rd,
                    score: if r.attempt.success { 1.0 } else { 0.0 },
                });
            }
        }
        for (op, games) in per_opener {
            let cur = profile.openers.get(&op).copied().unwrap_or(Rating::new(
                policy.rating.initial_rating,
                policy.rating.initial_rd,
                policy.rating.initial_volatility,
            ));
            profile.openers.insert(
                op,
                cur.update(&games, tau, policy.rating.rd_min, policy.rating.rd_max),
            );
        }
        for (b, games) in per_bucket {
            if is_anchor(&b) {
                continue; // anchors define the scale and never move
            }
            let est = self
                .results
                .iter()
                .find(|r| r.bucket == b)
                .map(|r| r.estimate)
                .unwrap_or(1500.0);
            let cur = profile
                .calibration
                .buckets
                .get(&b)
                .copied()
                .unwrap_or(Rating::new(est, BUCKET_INITIAL_RD, 0.06));
            let mut upd = cur.update(&games, tau, policy.difficulty.bucket_rd_floor, 350.0);
            upd.rd = upd.rd.max(policy.difficulty.bucket_rd_floor);
            profile.calibration.estimates.insert(b.clone(), est);
            profile.calibration.buckets.insert(b, upd);
        }
        if valid > 0 {
            profile.calibration.version += 1;
        }
        // match observations fade: halve the boosts each cycle
        for v in profile.observed.values_mut() {
            *v *= 0.5;
        }
        profile.observed.retain(|_, v| *v > 0.01);
        CycleReport {
            before,
            after: profile.skills.clone(),
            valid_attempts: valid,
            successes: succ,
        }
    }
}
