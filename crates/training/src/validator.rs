//! ProblemValidator (spec 5.1/5.3): a problem is valid only if its witness, replayed as real
//! key inputs through the tick engine, satisfies the goal (finesse problems: with optimal cost).

use crate::problem::{fingerprint, Goal, Problem};
use crate::session::{AttemptSession, Status};
use stacklab_engine::replay::ruleset_by_version;

pub const VALIDATION_VERSION: &str = "V1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub valid: bool,
    pub validation_version: &'static str,
    pub reason: Option<String>,
}

pub fn verify(problem: &Problem) -> Verdict {
    let fail = |r: String| Verdict {
        valid: false,
        validation_version: VALIDATION_VERSION,
        reason: Some(r),
    };
    if ruleset_by_version(&problem.ruleset_version).is_none() {
        return fail(format!("unknown ruleset {}", problem.ruleset_version));
    }
    if fingerprint(problem.module, &problem.initial, &problem.goal) != problem.fingerprint {
        return fail("fingerprint mismatch".into());
    }
    let mut s = match AttemptSession::new(problem, problem.witness.handling) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let spf = s.game().ruleset().timing.subframes_per_frame as u64;
    let mut inputs = problem.witness.inputs.clone();
    inputs.sort_by_key(|e| e.time);
    let mut i = 0;
    for _ in 0..problem.witness.frames + 60 {
        let end = (s.game().tick() + 1) * spf;
        let start = i;
        while i < inputs.len() && inputs[i].time < end {
            i += 1;
        }
        s.step(&inputs[start..i]);
        if *s.status() != Status::Running {
            break;
        }
    }
    match s.status() {
        Status::Success => {}
        other => return fail(format!("witness does not solve the problem: {other:?}")),
    }
    let r = s.result();
    if let Goal::Finesse { .. } = problem.goal {
        if r.finesse_success != Some(true) {
            return fail(format!(
                "witness not optimal: inputs {:?}",
                r.inputs_per_piece
            ));
        }
    }
    Verdict {
        valid: true,
        validation_version: VALIDATION_VERSION,
        reason: None,
    }
}
