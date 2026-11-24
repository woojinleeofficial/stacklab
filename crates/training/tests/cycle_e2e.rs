//! One full 12-problem cycle with real generated problems: plan -> serve -> play the witness
//! through the engine -> record -> close the rating period.

use stacklab_engine::rng::Rng;
use stacklab_engine::Ruleset;
use stacklab_training::adaptive::{plan_cycle, Cycle, SkillProfile, SlotResult};
use stacklab_training::policy::Policy;
use stacklab_training::service::ProblemService;
use stacklab_training::session::{AttemptSession, Status};
use std::sync::Arc;

fn play_witness(p: &stacklab_training::Problem, give_up: bool) -> AttemptSession {
    let mut s = AttemptSession::new(p, p.witness.handling).unwrap();
    let mut inputs = p.witness.inputs.clone();
    inputs.sort_by_key(|e| e.time);
    let mut f = 0u64;
    while *s.status() == Status::Running && f < p.time_limit_secs as u64 * 60 + 5 {
        let frame: Vec<_> = if give_up {
            vec![]
        } else {
            inputs
                .iter()
                .filter(|e| e.time / 10 == f)
                .copied()
                .collect()
        };
        s.step(&frame);
        f += 1;
    }
    s
}

#[test]
fn full_cycle_with_real_problems() {
    let policy = Policy::bundled();
    let mut svc = ProblemService::new(Arc::new(Ruleset::tl_s2()), policy.clone());
    let mut profile = SkillProfile::new(&policy);
    let mut rng = Rng::new(2026);
    let plan = plan_cycle(&profile, &policy, &mut rng);
    let mut cycle = Cycle::new(plan);
    let mut n = 0;
    while let Some(m) = cycle.next_module() {
        let (p, _) = svc.next_for(&profile, m, &mut rng).expect("problem served");
        assert!(stacklab_training::validator::verify(&p).valid);
        // the simulated player solves every other problem with the witness, fails the rest
        let s = play_witness(&p, n % 2 == 1);
        let r = s.result();
        assert_eq!(r.success, n % 2 == 0, "{} {:?}", p.id, r.reason);
        let share = (r.elapsed_frames as f64 / (p.time_limit_secs as f64 * 60.0)).min(1.0);
        assert!(cycle.record(SlotResult {
            module: p.module,
            bucket: p.param_bucket.clone(),
            estimate: p.difficulty,
            tags: p.subtype_tags.clone(),
            attempt: r,
            time_share: share
        }));
        n += 1;
    }
    assert_eq!(n, 12);
    let report = cycle.close(&mut profile, &policy);
    assert!(report.valid_attempts > 0);
    assert!(
        report.after.iter().any(|(m, r)| report.before[m] != *r),
        "ratings changed"
    );
    assert!(!profile.calibration.buckets.is_empty() || report.valid_attempts == 0);
}
