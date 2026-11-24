//! Generation pipeline with fallbacks (spec 5.1, 5.3, 11):
//! generate -> verify -> duplicate check -> serve; on failure use the cache (same module,
//! nearest difficulty); if the cache has nothing, serve another module and log a diagnostic.

use crate::cache::ProblemCache;
use crate::policy::Policy;
use crate::problem::{Module, Problem};
use crate::validator::verify;
use stacklab_engine::{MoveRules, Ruleset};
use std::collections::VecDeque;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Generated,
    Cache,
    OtherModule(Module),
}

pub struct ProblemService {
    pub rs: Arc<Ruleset>,
    pub library: stacklab_opener::library::Library,
    /// Active opener loadout (module E needs it).
    pub loadout: Option<stacklab_opener::loadout::OpenerLoadout>,
    rules: MoveRules,
    pub policy: Policy,
    pub cache: ProblemCache,
    recent: VecDeque<u64>,
    pub diagnostics: Vec<String>,
}

/// Modules that have a generator in this build.
pub fn supported(m: Module) -> bool {
    // E (openers) additionally needs a loadout; generation returns None without one
    let _ = m;
    true
}

/// Opener role for an E slot from the policy mix (primary 70 / switch 20 / fallback 10).
pub fn opener_role(policy: &Policy, seed: u64) -> crate::opener_gen::OpenerRole {
    use crate::opener_gen::OpenerRole;
    let mix = &policy.scheduler.opener_mix;
    let p = *mix.get("primary").unwrap_or(&70) as u64;
    let s = *mix.get("switch_decision").unwrap_or(&20) as u64;
    let f = *mix.get("fallback").unwrap_or(&10) as u64;
    let x = stacklab_engine::rng::Rng::new(seed ^ 0x0e0e).below(p + s + f);
    if x < p {
        OpenerRole::Primary
    } else if x < p + s {
        OpenerRole::Switch
    } else {
        OpenerRole::Fallback
    }
}

impl ProblemService {
    pub fn new(rs: Arc<Ruleset>, policy: Policy) -> ProblemService {
        ProblemService {
            rules: MoveRules::from_ruleset(&rs),
            rs,
            library: stacklab_opener::library::Library::bundled(),
            loadout: None,
            policy,
            cache: ProblemCache::default(),
            recent: VecDeque::new(),
            diagnostics: Vec::new(),
        }
    }

    /// ProblemGenerator.generate(module, difficulty, seed) — unvalidated candidate.
    pub fn generate(&self, m: Module, d: f64, seed: u64) -> Option<Problem> {
        let tl = self.policy.time_limit_secs(m, d);
        match m {
            Module::A => crate::finesse::generate(&self.rs, &self.rules, d, seed, tl),
            Module::B => crate::stacking::generate(&self.rs, &self.rules, d, seed, tl),
            Module::C => crate::spin::generate(&self.rs, &self.rules, d, seed, tl),
            Module::D => crate::pcgen::generate(&self.rs, &self.rules, d, seed, tl),
            Module::F => crate::attack::generate(&self.rs, &self.rules, d, seed, tl),
            Module::G => crate::defense::generate(&self.rs, &self.rules, d, seed, tl),
            Module::H => crate::judgment::generate(&self.rs, &self.rules, d, seed, tl),
            Module::E => {
                let lo = self.loadout.as_ref()?;
                let role = opener_role(&self.policy, seed);
                crate::opener_gen::generate(
                    &self.rs,
                    &self.rules,
                    &self.library,
                    lo,
                    role,
                    d,
                    seed,
                    tl,
                )
            }
        }
    }

    fn is_recent(&self, fp: u64) -> bool {
        self.recent.contains(&fp)
    }

    fn remember(&mut self, fp: u64) {
        self.recent.push_back(fp);
        while self.recent.len() > self.policy.scheduler.duplicate_window {
            self.recent.pop_front();
        }
    }

    /// Next problem for (module, difficulty). `attempts` bounds generation work (the "timeout").
    pub fn next(
        &mut self,
        m: Module,
        d: f64,
        seed: u64,
        attempts: u32,
    ) -> Option<(Problem, Source)> {
        for k in 0..attempts as u64 {
            let Some(p) = self.generate(m, d, seed.wrapping_mul(1_000_003).wrapping_add(k)) else {
                continue;
            };
            if self.is_recent(p.fingerprint) {
                continue;
            }
            let v = verify(&p);
            if !v.valid {
                self.diagnostics
                    .push(format!("discarded {} : {:?}", p.id, v.reason));
                continue;
            }
            self.remember(p.fingerprint);
            return Some((p, Source::Generated));
        }
        let exclude: Vec<u64> = self.recent.iter().copied().collect();
        if let Some(p) = self.cache.nearest(m, d, &exclude).cloned() {
            self.remember(p.fingerprint);
            self.diagnostics.push(format!(
                "generation failed for {m:?}@{d:.0}; served cache {}",
                p.id
            ));
            return Some((p, Source::Cache));
        }
        for alt in Module::ALL {
            if alt == m || !supported(alt) {
                continue;
            }
            for k in 0..attempts as u64 {
                if let Some(p) = self.generate(alt, d, seed ^ (0xa17 + k)) {
                    if !self.is_recent(p.fingerprint) && verify(&p).valid {
                        self.diagnostics
                            .push(format!("no problem for {m:?}@{d:.0}; served {alt:?}"));
                        self.remember(p.fingerprint);
                        return Some((p, Source::OtherModule(alt)));
                    }
                }
            }
        }
        self.diagnostics
            .push(format!("no problem available for {m:?}@{d:.0}"));
        None
    }
}

/// Seeds of the fixed anchor set (spec 6.2 / docs/DIFFICULTY_ANCHORS.md): 10 per module.
pub const ANCHOR_SEED_BASE: u64 = 0x57AC_1AB0;
pub const ANCHORS_PER_MODULE: u64 = 10;

impl ProblemService {
    /// Anchor problem `k` (0..10) of a module: generated at D = 1500 from a fixed seed and
    /// placed in the module's anchor bucket.
    pub fn anchor(&self, m: Module, k: u64) -> Option<Problem> {
        (0..20u64).find_map(|retry| {
            let mut p = self.generate(m, 1500.0, ANCHOR_SEED_BASE + k * 101 + retry * 7919)?;
            if !verify(&p).valid {
                return None;
            }
            p.param_bucket = crate::adaptive::anchor_bucket(m);
            p.difficulty = 1500.0;
            Some(p)
        })
    }

    /// Next problem for a scheduled slot: an anchor with the policy share, else a problem
    /// generated at the calibrated request for 75 % expected success.
    pub fn next_for(
        &mut self,
        profile: &crate::adaptive::SkillProfile,
        m: Module,
        rng: &mut stacklab_engine::rng::Rng,
    ) -> Option<(Problem, Source)> {
        self.loadout = profile.loadout.clone();
        if m != Module::E && crate::adaptive::serve_anchor(&self.policy, rng) {
            let k = rng.below(ANCHORS_PER_MODULE);
            if let Some(p) = self.anchor(m, k) {
                if !self.is_recent(p.fingerprint) {
                    self.remember(p.fingerprint);
                    return Some((p, Source::Generated));
                }
            }
        }
        let want = crate::adaptive::target_difficulty(profile, m, &self.policy, rng);
        let req = profile.calibration.request_estimate(m, want);
        let seed = rng.below(u32::MAX as u64);
        self.next(m, req, seed, 8)
    }
}
