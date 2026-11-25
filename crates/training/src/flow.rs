//! The whole training flow (spec 1.2, 7.1) as one serialisable state machine, shared by the app
//! and the automatic play-through test: diagnosis → 12 problems ×3 with mini matches → BO3 →
//! result (→ one-time opener recommendation trigger) → next set.

use crate::adaptive::{
    anchor_bucket, diagnosis_plan, plan_cycle, Cycle, CycleReport, SkillProfile, SlotResult,
};
use crate::glicko::Rating;
use crate::policy::Policy;
use crate::problem::{Module, Problem};
use crate::session::AttemptResult;
use crate::set::{
    select_opponent, MatchProfile, Observations, Phase, RoundRecord, SetError, SetState,
};
use serde::{Deserialize, Serialize};
use stacklab_engine::rng::Rng;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrainingState {
    pub profile: SkillProfile,
    pub matchp: MatchProfile,
    pub cycle: Cycle,
    pub diagnosis: bool,
    pub cycles_done: u32,
    pub set: SetState,
    /// Deterministic seed stream (persisted so a resumed session continues it).
    pub seed_state: u64,
}

/// What the problem worker should produce next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProblemRequest {
    /// Diagnosis uses the fixed anchor set (spec 6.4).
    Anchor {
        module: Module,
        k: u64,
    },
    Slot {
        module: Module,
        seed: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundKind {
    Mini,
    DeferredMini(usize),
    Evaluation,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvaluationOutcome {
    pub wins: u32,
    pub losses: u32,
    pub before: Rating,
    pub after: Rating,
    pub weaknesses: Vec<&'static str>,
    /// Spec 9.1: this evaluation is the first one that qualifies for the automatic event.
    pub trigger_recommendation: bool,
}

impl TrainingState {
    pub fn new(policy: &Policy, seed: u64) -> TrainingState {
        TrainingState {
            profile: SkillProfile::new(policy),
            matchp: MatchProfile::new(policy),
            cycle: Cycle::new(diagnosis_plan(1)),
            diagnosis: true,
            cycles_done: 0,
            set: SetState::new_first(),
            seed_state: seed.max(1),
        }
    }

    fn next_seed(&mut self) -> u64 {
        let mut r = Rng::new(self.seed_state);
        self.seed_state = r.below(u64::MAX >> 1).max(1);
        self.seed_state
    }

    /// UI-02 "진단 건너뛰기": start training with the initial (provisional, RD 350) ratings.
    /// Only before any diagnosis problem was recorded.
    pub fn skip_diagnosis(&mut self, policy: &Policy) -> Result<(), SetError> {
        if !self.diagnosis || !self.cycle.results.is_empty() {
            return Err(SetError::WrongPhase(self.set.phase));
        }
        self.set.diagnosis_done()?;
        self.diagnosis = false;
        let mut rng = Rng::new(self.next_seed());
        self.cycle = Cycle::new(plan_cycle(&self.profile, policy, &mut rng));
        Ok(())
    }

    /// Problems are served only in DIAGNOSIS / TRAINING phases (matches come after 12).
    pub fn next_request(&mut self) -> Option<ProblemRequest> {
        if !matches!(self.set.phase, Phase::Diagnosis | Phase::Training(_)) {
            return None;
        }
        let module = self.cycle.next_module()?;
        let seed = self.next_seed();
        Some(if self.diagnosis {
            ProblemRequest::Anchor {
                module,
                k: seed % 10,
            }
        } else {
            ProblemRequest::Slot { module, seed }
        })
    }

    /// Records the first decided attempt of the current slot. Retries never add problems.
    /// Returns the rating report when the 12-problem rating period closes.
    pub fn record_problem(
        &mut self,
        p: &Problem,
        first: AttemptResult,
        policy: &Policy,
    ) -> Option<CycleReport> {
        if !matches!(self.set.phase, Phase::Diagnosis | Phase::Training(_)) {
            return None;
        }
        let share =
            (first.elapsed_frames as f64 / (p.time_limit_secs.max(1) as f64 * 60.0)).min(1.0);
        let bucket = if self.diagnosis {
            anchor_bucket(p.module)
        } else {
            p.param_bucket.clone()
        };
        self.cycle.record(SlotResult {
            module: p.module,
            bucket,
            estimate: p.difficulty,
            tags: p.subtype_tags.clone(),
            attempt: first,
            time_share: share,
        });
        if !self.cycle.is_complete() {
            return None;
        }
        let report = self.cycle.close(&mut self.profile, policy);
        if self.diagnosis {
            let _ = self.set.diagnosis_done();
        } else {
            let _ = self.set.cycle_done();
            self.cycles_done += 1;
        }
        self.diagnosis = false;
        let mut rng = Rng::new(self.next_seed());
        self.cycle = Cycle::new(plan_cycle(&self.profile, policy, &mut rng));
        Some(report)
    }

    /// Opponent for the next round (league ratings adjusted by in-use calibration).
    pub fn opponent(
        &self,
        league: &[(String, f64, f64)],
        policy: &Policy,
    ) -> Option<(String, f64, f64, f64, bool)> {
        let bots: Vec<(String, f64, f64)> = league
            .iter()
            .map(|(id, r, rd)| {
                let (r, rd) = self.matchp.bot_rating(id, *r, *rd);
                (id.clone(), r, rd)
            })
            .collect();
        let band = (
            policy.matches.win_prob_band[0],
            policy.matches.win_prob_band[1],
        );
        let (id, p, in_band) = select_opponent(&self.matchp, &bots, band)?;
        let (_, r, rd) = bots.iter().find(|b| b.0 == id)?.clone();
        Some((id, r, rd, p, in_band))
    }

    /// A round finished. Applies observations, advances the set, rates a completed evaluation.
    #[allow(clippy::too_many_arguments)]
    pub fn record_round(
        &mut self,
        kind: RoundKind,
        won: bool,
        bot: &str,
        bot_rating: f64,
        bot_rd: f64,
        league_version: u32,
        obs: Observations,
        policy: &Policy,
        automatic_exists: bool,
    ) -> Result<Option<EvaluationOutcome>, SetError> {
        self.profile
            .apply_match_observations(std::slice::from_ref(&obs));
        match kind {
            RoundKind::Mini => {
                self.set.mini_result(won)?;
                Ok(None)
            }
            RoundKind::DeferredMini(i) => {
                self.set.deferred_mini_result(i, won);
                Ok(None)
            }
            RoundKind::Evaluation => {
                let rec = RoundRecord {
                    won,
                    void: false,
                    bot: bot.into(),
                    bot_rating,
                    bot_rd,
                    calibration_version: self.matchp.calibration_version(league_version),
                    observations: obs,
                };
                if !self.set.eval_round(rec)? {
                    return Ok(None);
                }
                let (wins, losses) = self.set.score();
                let (before, after) = self.matchp.rate_evaluation(&self.set.eval_rounds, policy);
                let weaknesses = self
                    .set
                    .eval_rounds
                    .iter()
                    .flat_map(|r| r.observations.weaknesses().into_iter().map(|x| x.1))
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect();
                let trigger = crate::recommend::automatic_trigger(
                    &self.profile,
                    &self.set,
                    policy,
                    automatic_exists,
                );
                Ok(Some(EvaluationOutcome {
                    wins,
                    losses,
                    before,
                    after,
                    weaknesses,
                    trigger_recommendation: trigger,
                }))
            }
        }
    }

    /// A technical error voided the round: replay it, never count it (spec 11).
    pub fn void_round(
        &mut self,
        bot: &str,
        bot_rating: f64,
        bot_rd: f64,
        league_version: u32,
    ) -> Result<(), SetError> {
        let rec = RoundRecord {
            won: false,
            void: true,
            bot: bot.into(),
            bot_rating,
            bot_rd,
            calibration_version: self.matchp.calibration_version(league_version),
            observations: Observations::default(),
        };
        self.set.eval_round(rec).map(|_| ())
    }

    pub fn next_set(&mut self) -> Result<(), SetError> {
        self.set = self.set.next_set()?;
        Ok(())
    }

    /// Progress text inputs: (cycle index within the set, 1-based slot).
    pub fn progress(&self) -> (u32, usize) {
        let c = match self.set.phase {
            Phase::Training(k) => k as u32,
            _ => 3,
        };
        (c, self.cycle.results.len() + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnosis_can_be_skipped_only_before_it_starts() {
        let policy = Policy::bundled();
        let mut st = TrainingState::new(&policy, 3);
        st.skip_diagnosis(&policy).unwrap();
        assert!(matches!(st.set.phase, Phase::Training(1)));
        assert!(!st.diagnosis);
        assert_eq!(st.cycle.plan.len(), 12);
        assert!(matches!(
            st.next_request(),
            Some(ProblemRequest::Slot { .. })
        ));
        assert!(st.skip_diagnosis(&policy).is_err());
    }
}
