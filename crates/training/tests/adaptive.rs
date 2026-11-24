//! STEP 3: scheduler constraints, rating periods, and the simulated-user convergence check
//! (spec 14.2: success rate converges to 75 % ± 10 %p).

use stacklab_engine::rng::Rng;
use stacklab_training::adaptive::{
    anchor_bucket, diagnosis_plan, plan_cycle, serve_anchor, target_difficulty, Cycle,
    SkillProfile, SlotResult, ANCHOR,
};
use stacklab_training::glicko::expected;
use stacklab_training::policy::Policy;
use stacklab_training::problem::Module;
use stacklab_training::session::AttemptResult;
use std::collections::BTreeMap;

fn attempt(success: bool, valid: bool) -> AttemptResult {
    AttemptResult {
        success,
        finesse_success: None,
        used_hint: !valid,
        used_rewind: false,
        used_answer: false,
        valid_for_mmr: valid,
        elapsed_frames: 600,
        inputs_per_piece: vec![],
        reason: None,
    }
}

#[test]
fn cycle_plans_respect_constraints() {
    let policy = Policy::bundled();
    let profile = SkillProfile::new(&policy);
    let mut rng = Rng::new(3);
    for _ in 0..2000 {
        let plan = plan_cycle(&profile, &policy, &mut rng);
        assert_eq!(plan.len(), 12);
        let mut c: BTreeMap<Module, usize> = BTreeMap::new();
        for m in &plan {
            *c.entry(*m).or_default() += 1;
        }
        assert!(c.values().all(|&n| n <= 5), "{c:?}");
        assert!(c.len() >= 3);
        assert!(
            c.get(&Module::D).copied().unwrap_or(0) >= 2,
            "PC floor (D-019)"
        );
        assert!(!c.contains_key(&Module::E), "openers arrive in STEP 4");
    }
}

#[test]
fn failures_raise_weight_and_retries_do_not_add_problems() {
    let policy = Policy::bundled();
    let mut profile = SkillProfile::new(&policy);
    let mut cycle = Cycle::new(vec![Module::C; 12]);
    for i in 0..12 {
        let r = SlotResult {
            module: Module::C,
            bucket: "C:x".into(),
            estimate: 1500.0,
            tags: vec!["tsd".into()],
            attempt: attempt(i % 4 == 0, true),
            time_share: 0.8,
        };
        assert!(cycle.record(r.clone()));
    }
    assert!(cycle.is_complete());
    let extra = SlotResult {
        module: Module::C,
        bucket: "C:x".into(),
        estimate: 1500.0,
        tags: vec![],
        attempt: attempt(true, true),
        time_share: 0.1,
    };
    assert!(!cycle.record(extra), "no 13th problem");
    let report = cycle.close(&mut profile, &policy);
    assert_eq!(report.valid_attempts, 12);
    let w = profile.weights(&policy);
    assert!(w[&Module::C] > w[&Module::A], "{w:?}");
    assert!(w.values().all(|&x| (0.8..=1.8).contains(&x)));
    assert!(report.after[&Module::C].rating < report.before[&Module::C].rating);
    assert_eq!(profile.weak_tags(Module::C)[0].0, "tsd");
}

#[test]
fn assisted_attempts_do_not_change_ratings() {
    let policy = Policy::bundled();
    let mut profile = SkillProfile::new(&policy);
    let mut cycle = Cycle::new(vec![Module::A; 12]);
    for _ in 0..12 {
        cycle.record(SlotResult {
            module: Module::A,
            bucket: "A:x".into(),
            estimate: 1500.0,
            tags: vec![],
            attempt: attempt(true, false),
            time_share: 0.5,
        });
    }
    let before = profile.skill(Module::A);
    cycle.close(&mut profile, &policy);
    assert_eq!(profile.skill(Module::A), before);
}

/// Simulated user: true skill per module; problems' true difficulty = generator estimate +
/// a hidden per-module bias (+ per-bucket noise) that the system must learn.
const TRUTH: [(Module, f64); 7] = [
    (Module::A, 1750.0),
    (Module::B, 1400.0),
    (Module::C, 1550.0),
    (Module::D, 1300.0),
    (Module::F, 1650.0),
    (Module::G, 1500.0),
    (Module::H, 1600.0),
];

fn truth_of(m: Module) -> f64 {
    TRUTH.iter().find(|t| t.0 == m).map_or(1500.0, |t| t.1)
}

/// Runs the simulation; returns per-cycle (successes, valid) and the final profile.
/// Per cycle: (successes, valid) over difficulty-targeted problems only (anchors excluded:
/// they are fixed-difficulty measurement items, not targeted at 75 %).
pub fn simulate(seed: u64, cycles: usize) -> (Vec<(u32, u32)>, SkillProfile) {
    let policy = Policy::bundled();
    let mut profile = SkillProfile::new(&policy);
    let mut rng = Rng::new(seed);
    let mut per_cycle = Vec::new();
    // diagnosis: 4 anchor problems per module (spec 6.4)
    let diag = diagnosis_plan(4);
    let mut d = Cycle::new(diag.clone());
    for m in diag {
        let p = expected(truth_of(m), 1500.0, 0.0);
        let success = rng.below(10_000) < (p * 10_000.0) as u64;
        d.record(SlotResult {
            module: m,
            bucket: anchor_bucket(m),
            estimate: 1500.0,
            tags: vec![],
            attempt: attempt(success, true),
            time_share: 0.5,
        });
    }
    d.close(&mut profile, &policy);
    let truth: BTreeMap<Module, f64> = TRUTH.into();
    let bias: BTreeMap<Module, f64> = [
        (Module::A, 150.0),
        (Module::B, -120.0),
        (Module::C, 60.0),
        (Module::D, 200.0),
        (Module::F, -80.0),
        (Module::G, 0.0),
        (Module::H, 100.0),
    ]
    .into();
    for _ in 0..cycles {
        let mut tally = (0u32, 0u32);
        let plan = plan_cycle(&profile, &policy, &mut rng);
        let mut cycle = Cycle::new(plan.clone());
        for m in plan {
            let anchor = serve_anchor(&policy, &mut rng);
            let (bucket, snapped, true_d) = if anchor {
                // anchor problems define D = 1500 on every module's scale
                (anchor_bucket(m), 1500.0, 1500.0)
            } else {
                let want = target_difficulty(&profile, m, &policy, &mut rng);
                let est = profile.calibration.request_estimate(m, want);
                let snapped = (est / 50.0).round() * 50.0;
                let noise = ((snapped as i64 / 50) % 3 - 1) as f64 * 30.0;
                (
                    format!("{}:{}", m.letter(), snapped as i64),
                    snapped,
                    snapped + bias[&m] + noise,
                )
            };
            let p = expected(truth[&m], true_d, 0.0);
            let success = rng.below(10_000) < (p * 10_000.0) as u64;
            let valid = !rng.chance_pm(100); // 10 % used a hint
            if valid && !anchor {
                tally.0 += success as u32;
                tally.1 += 1;
            }
            cycle.record(SlotResult {
                module: m,
                bucket,
                estimate: snapped,
                tags: vec![],
                attempt: attempt(success, valid),
                time_share: 0.5,
            });
        }
        cycle.close(&mut profile, &policy);
        per_cycle.push(tally);
    }
    (per_cycle, profile)
}

#[test]
fn simulated_user_converges_to_target_success() {
    let mut errs = Vec::new();
    for seed in 1..=10u64 {
        let (per_cycle, profile) = simulate(seed, 60);
        let (s, n) = per_cycle[30..]
            .iter()
            .fold((0, 0), |a, c| (a.0 + c.0, a.1 + c.1));
        let rate = s as f64 / n as f64;
        assert!(
            (0.65..=0.85).contains(&rate),
            "seed {seed}: late success rate {rate:.3}"
        );
        for &(m, t) in &TRUTH {
            let r = profile.skill(m);
            errs.push((r.rating - t).abs());
            assert!(r.rd < 100.0, "seed {seed} {m:?}: {r:?}");
        }
    }
    errs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mae = errs.iter().sum::<f64>() / errs.len() as f64;
    let p90 = errs[errs.len() * 9 / 10];
    println!("rating error vs true skill over 10 users x 7 modules: mean {mae:.0}, p90 {p90:.0}, max {:.0}", errs[errs.len() - 1]);
    // with anchors (diagnosis + 15 % share) the absolute scale is identifiable
    assert!(mae < 100.0, "mean abs error {mae:.0}");
    assert!(p90 < 200.0, "p90 abs error {p90:.0}");
}

#[test]
fn convergence_report() {
    let runs: Vec<Vec<(u32, u32)>> = (1..=10u64).map(|s| simulate(s, 60).0).collect();
    println!(
        "cycles   success rate (10 simulated users, valid targeted attempts; anchors excluded)"
    );
    for w in 0..12 {
        let (s, n) = runs
            .iter()
            .flat_map(|r| r[w * 5..w * 5 + 5].iter())
            .fold((0, 0), |a, c| (a.0 + c.0, a.1 + c.1));
        println!(
            "{:>2}-{:<2}    {:.1}%",
            w * 5 + 1,
            w * 5 + 5,
            100.0 * s as f64 / n as f64
        );
    }
    let (_, p) = simulate(1, 60);
    for (m, r) in &p.skills {
        println!("{m:?}: {:.0} ± {:.0}", r.rating, r.rd);
    }
}

#[test]
fn anchor_constant_matches_policy() {
    let p = Policy::bundled();
    assert_eq!(ANCHOR, (p.difficulty.anchor_rating, p.difficulty.anchor_rd));
}
