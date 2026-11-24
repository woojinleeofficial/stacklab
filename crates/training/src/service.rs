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
    rules: MoveRules,
    pub policy: Policy,
    pub cache: ProblemCache,
    recent: VecDeque<u64>,
    pub diagnostics: Vec<String>,
}

/// Modules that have a generator in this build.
pub fn supported(m: Module) -> bool {
    matches!(m, Module::A | Module::B)
}

impl ProblemService {
    pub fn new(rs: Arc<Ruleset>, policy: Policy) -> ProblemService {
        ProblemService {
            rules: MoveRules::from_ruleset(&rs),
            rs,
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
            _ => None,
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
