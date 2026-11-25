//! Spec 6.4: the independent skill test serves its fixed composition at D = 1500 with problems
//! that were not served before, and never touches the training profile.

use stacklab_engine::Ruleset;
use stacklab_opener::library::Library;
use stacklab_opener::loadout::OpenerLoadout;
use stacklab_training::independent::{IndependentTest, DIFFICULTY};
use stacklab_training::policy::Policy;
use stacklab_training::service::ProblemService;
use stacklab_training::session::AttemptSession;
use std::sync::Arc;

#[test]
fn two_tests_serve_sixteen_fresh_verified_problems_each() {
    let policy = Policy::bundled();
    let mut svc = ProblemService::new(Arc::new(Ruleset::tl_s2()), policy.clone());
    svc.loadout =
        Some(OpenerLoadout::new(&Library::bundled(), "TL-S2", "tki", "free-transition").unwrap());
    let mut seen: Vec<String> = Vec::new();
    for n in 1..=2 {
        let mut t = IndependentTest::new(n);
        while let Some((m, seed)) = t.next() {
            let p = svc
                .independent(m, seed, &seen)
                .unwrap_or_else(|| panic!("{m:?} test {n}"));
            assert_eq!(p.difficulty, DIFFICULTY);
            assert_eq!(p.time_limit_secs, policy.time_limit_secs(m, DIFFICULTY));
            assert!(!seen.contains(&p.id));
            seen.push(p.id.clone());
            let s = AttemptSession::new(&p, p.witness.handling).unwrap();
            t.record(&p, &s.result());
        }
        let r = t.finish(3 * n, "2026-01-01T00:00:00Z".into());
        assert_eq!(r.per_module.values().map(|x| x.0).sum::<u32>(), 16);
        assert_eq!(r.success_rate, 0.0, "nothing was played");
    }
}
