//! Pre-validated problem cache used when generation times out (spec 5.3).

use crate::problem::{Module, Problem};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct ProblemCache {
    by_module: BTreeMap<Module, Vec<Problem>>,
}

impl ProblemCache {
    /// Only validated problems may be inserted.
    pub fn insert(&mut self, p: Problem) -> bool {
        if !crate::validator::verify(&p).valid {
            return false;
        }
        let v = self.by_module.entry(p.module).or_default();
        if v.iter().any(|q| q.fingerprint == p.fingerprint) {
            return false;
        }
        v.push(p);
        true
    }

    pub fn len(&self, m: Module) -> usize {
        self.by_module.get(&m).map_or(0, |v| v.len())
    }

    pub fn is_empty(&self) -> bool {
        self.by_module.values().all(|v| v.is_empty())
    }

    /// Closest difficulty in the module whose fingerprint is not excluded.
    pub fn nearest(&self, m: Module, d: f64, exclude: &[u64]) -> Option<&Problem> {
        self.by_module
            .get(&m)?
            .iter()
            .filter(|p| !exclude.contains(&p.fingerprint))
            .min_by(|a, b| {
                (a.difficulty - d)
                    .abs()
                    .partial_cmp(&(b.difficulty - d).abs())
                    .expect("finite")
            })
    }

    pub fn all(&self) -> impl Iterator<Item = &Problem> {
        self.by_module.values().flatten()
    }
}
