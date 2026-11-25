//! Opener pair recommendation (spec 9.2).
//!
//! Pair fit out of 100 (policy weights, initially 35/25/20/10/10):
//! * ability–requirement fit — user ability vector vs each opener's requirement vector,
//! * observed performance — opener proficiency MMR and match observations (model estimate
//!   when an opener was never observed, flagged as such),
//! * primary + fallback complementarity — share of first bags where the primary or, failing
//!   that, the fallback is possible (exact per-bag bitsets for exhaustive scopes),
//! * attack / follow-up potential — library traits (TL-S2 attack table),
//! * learning burden — library learning cost, lowered by the user's familiarity.
//!
//! Components a, b, d and e are evaluated as a pair: the primary covers its feasible share of
//! bags, the fallback the rest. Every component carries a standard deviation; their sum gives
//! the 80 % interval. A change is recommended only when the best pair is ≥ 8 points higher AND
//! its interval's lower bound exceeds the current pair's upper bound.

use crate::adaptive::SkillProfile;
use crate::policy::Policy;
use crate::problem::Module;
use serde::{Deserialize, Serialize};
use stacklab_opener::library::{CoverageRecord, Kind, Library, OpenerDefinition};
use std::collections::BTreeMap;

/// z for a two-sided 80 % interval.
pub const Z80: f64 = 1.2816;
/// Floor of the model uncertainty of any pair score (points).
pub const MODEL_SD_FLOOR: f64 = 2.0;
/// Uncertainty of a modelled (never observed) performance value, as a fraction of its weight.
pub const UNOBSERVED_SD: f64 = 0.25;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Component {
    pub key: String,
    pub weight: f64,
    pub value: f64,
    pub sd: f64,
    /// Part of the value is a model estimate (not observed data).
    pub estimated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Reason {
    Coverage {
        primary: f64,
        union: f64,
    },
    Shortfall {
        opener: String,
        dimension: String,
        required: u32,
        ability: u32,
    },
    Observed {
        opener: String,
        rating: f64,
        rd: f64,
    },
    Estimated {
        opener: String,
    },
    Learning {
        opener: String,
        cost: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairScore {
    pub primary: String,
    pub fallback: String,
    pub total: f64,
    pub sd: f64,
    pub low80: f64,
    pub high80: f64,
    pub components: Vec<Component>,
    pub reasons: Vec<Reason>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Analysis {
    /// Change recommended: current vs recommended pair.
    Change {
        current: PairScore,
        recommended: PairScore,
    },
    /// Keep the current pair (engine advice).
    NoChange {
        current: PairScore,
        best_alternative: Option<PairScore>,
    },
    /// Not enough verified data to give an honest recommendation.
    Unavailable { reason: String },
}

fn module_for(dim: &str) -> Option<Module> {
    Some(match dim {
        "finesse" => Module::A,
        "stacking" => Module::B,
        "spin" => Module::C,
        "pc" => Module::D,
        "memorization" => Module::E,
        "attack" => Module::F,
        "defense" => Module::G,
        "judgment" => Module::H,
        _ => return None,
    })
}

/// Ability 0..100 (rating 1000 -> 0, 2000 -> 100) and its standard deviation.
pub fn ability(profile: &SkillProfile, dim: &str) -> (f64, f64) {
    let Some(m) = module_for(dim) else {
        return (50.0, 35.0);
    };
    let r = profile.skill(m);
    (
        ((r.rating - 1000.0) / 10.0).clamp(0.0, 100.0),
        (r.rd / 10.0).min(35.0),
    )
}

/// Ability–requirement fit in [0, 1] with its sd, plus the largest shortfall.
fn ability_fit(
    profile: &SkillProfile,
    def: &OpenerDefinition,
) -> (f64, f64, Option<(String, u32, u32)>) {
    let total: f64 = def
        .requirements
        .values()
        .map(|&v| v as f64)
        .sum::<f64>()
        .max(1.0);
    let (mut short, mut var) = (0.0, 0.0);
    let mut worst: Option<(String, u32, u32)> = None;
    let mut worst_gap = 0.0;
    for (dim, &req) in &def.requirements {
        let (ab, sd) = ability(profile, dim);
        let gap = (req as f64 - ab).max(0.0);
        short += gap;
        if gap > 0.0 {
            var += sd * sd;
        }
        if gap > worst_gap {
            worst_gap = gap;
            worst = Some((dim.clone(), req, ab.round() as u32));
        }
    }
    (
        (1.0 - short / total).clamp(0.0, 1.0),
        var.sqrt() / total,
        worst,
    )
}

/// Observed performance in [0, 1] (proficiency rating 1000 -> 0, 2000 -> 1, blended with the
/// opener's match success rate) or a model estimate from the ability fit.
fn performance(
    profile: &SkillProfile,
    def: &OpenerDefinition,
    fit: f64,
) -> (f64, f64, bool, Option<(f64, f64)>) {
    if let Some(r) = profile.openers.get(&def.id).filter(|r| r.rd < 300.0) {
        let mut v = ((r.rating - 1000.0) / 1000.0).clamp(0.0, 1.0);
        let mut sd = r.rd / 1000.0;
        if let Some(&(n, s)) = profile.opener_match_stats.get(&def.id).filter(|x| x.0 >= 2) {
            let rate = s as f64 / n as f64;
            v = 0.7 * v + 0.3 * rate;
            sd = (0.49 * sd * sd + 0.09 * rate * (1.0 - rate) / n as f64).sqrt();
        }
        return (v, sd, false, Some((r.rating, r.rd)));
    }
    (0.8 * fit, UNOBSERVED_SD, true, None)
}

/// Rate of bags where the primary is feasible, the union with the fallback, and the sd of the
/// union estimate.
fn coverage_pair(p: &CoverageRecord, f: &CoverageRecord) -> (f64, f64, f64) {
    let cp = p.rate();
    if let (Some(a), Some(b)) = (p.per_bag(), f.per_bag()) {
        let union = a.iter().zip(&b).filter(|(x, y)| **x || **y).count() as f64 / a.len() as f64;
        return (cp, union, 0.0);
    }
    // sampled template: assume independence of the uncovered bags; use its interval as sd
    let cf = f.rate();
    let sd_f = (f.ci_high - f.ci_low) / (2.0 * 1.96);
    (cp, cp + (1.0 - cp) * cf, (1.0 - cp) * sd_f)
}

fn familiarity(profile: &SkillProfile, id: &str) -> f64 {
    let attempts: u32 = profile
        .branch_stats
        .iter()
        .filter(|(k, _)| k.starts_with(&format!("{id}/")))
        .map(|(_, v)| v.0)
        .sum();
    (attempts as f64 / 20.0).min(1.0)
}

/// Score of one primary/fallback pair.
pub fn score_pair(
    profile: &SkillProfile,
    lib: &Library,
    cov: &BTreeMap<String, CoverageRecord>,
    policy: &Policy,
    primary: &str,
    fallback: &str,
) -> Option<PairScore> {
    let w = &policy.recommendation;
    let (p, f) = (lib.get(primary)?, lib.get(fallback)?);
    let (cp_rec, cf_rec) = (cov.get(primary)?, cov.get(fallback)?);
    let (cp, union, union_sd) = coverage_pair(cp_rec, cf_rec);
    let share = |a: f64, b: f64| cp * a + (1.0 - cp) * b;
    let mut reasons = vec![Reason::Coverage { primary: cp, union }];

    let (fit_p, sd_fp, short_p) = ability_fit(profile, p);
    let (fit_f, sd_ff, short_f) = ability_fit(profile, f);
    for (o, s) in [(p, short_p), (f, short_f)] {
        if let Some((dim, req, have)) = s.filter(|x| x.1 > x.2 + 10) {
            reasons.push(Reason::Shortfall {
                opener: o.id.clone(),
                dimension: dim,
                required: req,
                ability: have,
            });
        }
    }
    let wa = w.weight("ability_fit");
    let a = Component {
        key: "ability_fit".into(),
        weight: wa,
        value: wa * share(fit_p, fit_f),
        sd: wa * (cp * cp * sd_fp * sd_fp + (1.0 - cp).powi(2) * sd_ff * sd_ff).sqrt(),
        estimated: false,
    };

    let (perf_p, sd_pp, est_p, obs_p) = performance(profile, p, fit_p);
    let (perf_f, sd_pf, est_f, obs_f) = performance(profile, f, fit_f);
    for (o, est, obs) in [(p, est_p, obs_p), (f, est_f, obs_f)] {
        match (est, obs) {
            (false, Some((r, rd))) => reasons.push(Reason::Observed {
                opener: o.id.clone(),
                rating: r,
                rd,
            }),
            _ => reasons.push(Reason::Estimated {
                opener: o.id.clone(),
            }),
        }
    }
    let wb = w.weight("observed");
    let b = Component {
        key: "observed".into(),
        weight: wb,
        value: wb * share(perf_p, perf_f),
        sd: wb * (cp * cp * sd_pp * sd_pp + (1.0 - cp).powi(2) * sd_pf * sd_pf).sqrt(),
        estimated: est_p || est_f,
    };

    let wc = w.weight("complement");
    let c = Component {
        key: "complement".into(),
        weight: wc,
        value: wc * union,
        sd: wc * union_sd,
        estimated: union_sd > 0.0,
    };

    let trait_of = |o: &OpenerDefinition, k: &str| *o.traits.get(k).unwrap_or(&50) as f64 / 100.0;
    let pot = |o: &OpenerDefinition| (trait_of(o, "attack") + trait_of(o, "followup")) / 2.0;
    let wd = w.weight("attack_followup");
    let d = Component {
        key: "attack_followup".into(),
        weight: wd,
        value: wd * share(pot(p), pot(f)),
        sd: wd * 0.05,
        estimated: true,
    };

    let cost = |o: &OpenerDefinition| trait_of(o, "learning") * (1.0 - familiarity(profile, &o.id));
    for o in [p, f] {
        if trait_of(o, "learning") >= 0.5 && familiarity(profile, &o.id) < 0.5 {
            reasons.push(Reason::Learning {
                opener: o.id.clone(),
                cost: (cost(o) * 100.0).round() as u32,
            });
        }
    }
    let we = w.weight("learning_cost");
    let e = Component {
        key: "learning_cost".into(),
        weight: we,
        value: we * (1.0 - (0.7 * cost(p) + 0.3 * cost(f))),
        sd: we * 0.05,
        estimated: true,
    };

    let components = vec![a, b, c, d, e];
    let total: f64 = components.iter().map(|c| c.value).sum();
    let sd = (components.iter().map(|c| c.sd * c.sd).sum::<f64>()
        + MODEL_SD_FLOOR * MODEL_SD_FLOOR)
        .sqrt();
    Some(PairScore {
        primary: primary.into(),
        fallback: fallback.into(),
        total,
        sd,
        low80: total - Z80 * sd,
        high80: total + Z80 * sd,
        components,
        reasons,
    })
}

/// All valid pairs (primary memorized, primary ≠ fallback, both in the ruleset).
pub fn candidate_pairs(lib: &Library, ruleset: &str) -> Vec<(String, String)> {
    let ids = lib.ids(ruleset);
    let mut out = Vec::new();
    for p in &ids {
        if lib.get(p).map(|d| d.kind) != Some(Kind::Memorized) {
            continue;
        }
        for f in &ids {
            if f != p {
                out.push((p.clone(), f.clone()));
            }
        }
    }
    out
}

/// Spec 9.2 analysis for the user's current loadout.
pub fn analyze(
    profile: &SkillProfile,
    lib: &Library,
    cov: &BTreeMap<String, CoverageRecord>,
    policy: &Policy,
    ruleset: &str,
) -> Analysis {
    let Some(lo) = &profile.loadout else {
        return Analysis::Unavailable {
            reason: "no-loadout".into(),
        };
    };
    // honest-data guard: without enough opener data the interval would be meaningless
    if profile.opener_valid_attempts < policy.recommendation.min_opener_attempts {
        return Analysis::Unavailable {
            reason: "insufficient-opener-attempts".into(),
        };
    }
    let Some(current) = score_pair(profile, lib, cov, policy, &lo.primary, &lo.fallback) else {
        return Analysis::Unavailable {
            reason: "unverified-opener".into(),
        };
    };
    let mut scored: Vec<PairScore> = candidate_pairs(lib, ruleset)
        .into_iter()
        .filter(|(p, f)| !(p == &lo.primary && f == &lo.fallback))
        .filter_map(|(p, f)| score_pair(profile, lib, cov, policy, &p, &f))
        .collect();
    scored.sort_by(|a, b| b.total.partial_cmp(&a.total).expect("finite"));
    let best = scored.into_iter().next();
    match best {
        Some(b)
            if b.total - current.total >= policy.recommendation.min_score_gap
                && b.low80 > current.high80 =>
        {
            Analysis::Change {
                current,
                recommended: b,
            }
        }
        other => Analysis::NoChange {
            current,
            best_alternative: other,
        },
    }
}

// ---------------------------------------------------------------------------------------
// One-time recommendation event and its state machine (spec 9.1, 9.3)

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecStatus {
    Pending,
    Analyzing,
    Ready,
    Applied,
    Declined,
    Deferred,
    NoChange,
    Unavailable,
}

impl RecStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RecStatus::Pending => "PENDING",
            RecStatus::Analyzing => "ANALYZING",
            RecStatus::Ready => "READY",
            RecStatus::Applied => "APPLIED",
            RecStatus::Declined => "DECLINED",
            RecStatus::Deferred => "DEFERRED",
            RecStatus::NoChange => "NO_CHANGE",
            RecStatus::Unavailable => "UNAVAILABLE",
        }
    }

    pub fn parse(s: &str) -> Option<RecStatus> {
        Some(match s {
            "PENDING" => RecStatus::Pending,
            "ANALYZING" => RecStatus::Analyzing,
            "READY" => RecStatus::Ready,
            "APPLIED" => RecStatus::Applied,
            "DECLINED" => RecStatus::Declined,
            "DEFERRED" => RecStatus::Deferred,
            "NO_CHANGE" => RecStatus::NoChange,
            "UNAVAILABLE" => RecStatus::Unavailable,
            _ => return None,
        })
    }

    /// Allowed transitions (spec 9.3).
    pub fn can_go(self, to: RecStatus) -> bool {
        use RecStatus::*;
        matches!(
            (self, to),
            (Pending, Analyzing)
                | (Analyzing, Ready)
                | (Analyzing, NoChange)
                | (Analyzing, Unavailable)
                | (Ready, Applied)
                | (Ready, Declined)
                | (Ready, Deferred)
                | (Deferred, Applied)
                | (Deferred, Declined)
        )
    }

    pub fn is_final(self) -> bool {
        matches!(
            self,
            RecStatus::Applied | RecStatus::Declined | RecStatus::NoChange | RecStatus::Unavailable
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecommendationEvent {
    pub event_id: String,
    /// The completed evaluation that triggered it (None for manual re-analysis).
    pub trigger_evaluation_id: Option<String>,
    pub automatic: bool,
    pub status: RecStatus,
    pub analysis: Option<Analysis>,
    /// Loadout version the analysis was made for (conflicts need a new approval).
    pub source_loadout_version: u32,
}

impl RecommendationEvent {
    pub fn transition(&mut self, to: RecStatus) -> Result<(), String> {
        if self.status.can_go(to) {
            self.status = to;
            Ok(())
        } else {
            Err(format!(
                "{} -> {} is not allowed",
                self.status.as_str(),
                to.as_str()
            ))
        }
    }

    /// PENDING -> ANALYZING -> READY / NO_CHANGE / UNAVAILABLE.
    pub fn run(
        &mut self,
        profile: &SkillProfile,
        lib: &Library,
        cov: &BTreeMap<String, CoverageRecord>,
        policy: &Policy,
        ruleset: &str,
    ) -> Result<(), String> {
        self.transition(RecStatus::Analyzing)?;
        let a = analyze(profile, lib, cov, policy, ruleset);
        let next = match &a {
            Analysis::Change { .. } => RecStatus::Ready,
            Analysis::NoChange { .. } => RecStatus::NoChange,
            Analysis::Unavailable { .. } => RecStatus::Unavailable,
        };
        self.analysis = Some(a);
        self.transition(next)
    }

    /// The recommended pair, when the analysis advises a change.
    pub fn recommended(&self) -> Option<(&str, &str)> {
        match &self.analysis {
            Some(Analysis::Change { recommended, .. }) => {
                Some((&recommended.primary, &recommended.fallback))
            }
            _ => None,
        }
    }
}

/// Spec 9.1: the automatic event is created only after a normally completed evaluation that is
/// the first to meet the minimum data, and never again once one exists.
pub fn automatic_trigger(
    profile: &SkillProfile,
    set: &crate::set::SetState,
    policy: &Policy,
    automatic_exists: bool,
) -> bool {
    !automatic_exists
        && set.evaluation_completed()
        && profile.loadout.is_some()
        && profile.opener_valid_attempts >= policy.recommendation.min_opener_attempts
        && profile.opener_observed_rounds >= policy.recommendation.min_observed_rounds
}

pub fn new_event(
    event_id: &str,
    trigger: Option<&str>,
    automatic: bool,
    loadout_version: u32,
) -> RecommendationEvent {
    RecommendationEvent {
        event_id: event_id.into(),
        trigger_evaluation_id: trigger.map(String::from),
        automatic,
        status: RecStatus::Pending,
        analysis: None,
        source_loadout_version: loadout_version,
    }
}
