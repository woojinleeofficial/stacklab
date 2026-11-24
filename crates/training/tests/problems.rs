//! STEP 2 completion: only validated problems are served, and witnesses reproduce.

use stacklab_engine::game::{InputEvent, Key};
use stacklab_engine::{MoveRules, Piece, Rot, Ruleset};
use stacklab_training::finesse::{optimal_costs, witness_handling};
use stacklab_training::policy::Policy;
use stacklab_training::problem::{fingerprint, Goal, Module, Problem, TargetPlacement, Witness};
use stacklab_training::service::{ProblemService, Source};
use stacklab_training::session::{AttemptSession, FailReason, Status};
use stacklab_training::validator::verify;
use std::sync::Arc;

fn service() -> ProblemService {
    ProblemService::new(Arc::new(Ruleset::tl_s2()), Policy::bundled())
}

#[test]
fn served_problems_are_all_valid_across_difficulties() {
    let mut svc = service();
    let mut served = 0;
    for (i, d) in [1100.0, 1300.0, 1500.0, 1700.0, 1900.0].iter().enumerate() {
        for k in 0..40u64 {
            let (p, src) = svc
                .next(Module::A, *d, 1000 * i as u64 + k, 5)
                .expect("finesse problem");
            assert_eq!(src, Source::Generated);
            assert!(verify(&p).valid, "{}", p.id);
            served += 1;
        }
        for k in 0..8u64 {
            let (p, _) = svc
                .next(Module::B, *d, 5000 + 100 * i as u64 + k, 5)
                .expect("stacking problem");
            assert_eq!(p.module, Module::B);
            assert!(verify(&p).valid, "{}", p.id);
            served += 1;
        }
    }
    assert_eq!(served, 240);
}

#[test]
fn difficulty_follows_the_request() {
    let svc = service();
    let mean = |d: f64| {
        let v: Vec<f64> = (0..30)
            .filter_map(|k| svc.generate(Module::A, d, k))
            .map(|p| p.difficulty)
            .collect();
        v.iter().sum::<f64>() / v.len() as f64
    };
    assert!(mean(1200.0) < mean(1500.0) && mean(1500.0) < mean(1800.0));
}

#[test]
fn problem_json_roundtrip_still_validates() {
    let svc = service();
    for k in 0..10 {
        let p = svc
            .generate(Module::B, 1500.0, k)
            .or_else(|| svc.generate(Module::B, 1500.0, k + 100))
            .unwrap();
        let back = Problem::from_json(&p.to_json()).unwrap();
        assert_eq!(back, p);
        assert!(verify(&back).valid);
    }
}

#[test]
fn recent_duplicates_are_not_served() {
    let mut svc = service();
    let (a, _) = svc.next(Module::A, 1500.0, 7, 5).unwrap();
    let (b, _) = svc.next(Module::A, 1500.0, 7, 5).unwrap();
    assert_ne!(a.fingerprint, b.fingerprint);
}

#[test]
fn fallback_to_cache_then_other_module() {
    let mut svc = service();
    let p = svc.generate(Module::B, 1500.0, 3).unwrap();
    assert!(svc.cache.insert(p.clone()));
    // generation budget 0 = immediate timeout -> cache
    let (q, src) = svc.next(Module::B, 1500.0, 1, 0).unwrap();
    assert_eq!((src, q.fingerprint), (Source::Cache, p.fingerprint));
    // no generator and no cache for C (STEP 3) -> another module, with a diagnostic
    let (r, src) = svc.next(Module::C, 1500.0, 1, 2).unwrap();
    assert!(matches!(src, Source::OtherModule(_)));
    assert_ne!(r.module, Module::C);
    assert!(svc.diagnostics.iter().any(|d| d.contains("C")));
}

#[test]
fn invalid_problems_are_rejected() {
    let svc = service();
    let mut p = svc.generate(Module::A, 1500.0, 11).unwrap();
    p.witness.inputs.clear();
    assert!(!verify(&p).valid, "empty witness cannot solve");
    let mut q = svc.generate(Module::A, 1500.0, 12).unwrap();
    q.initial.rows[0] ^= 1;
    assert!(!verify(&q).valid, "tampered state changes the fingerprint");
}

/// Builds a finesse problem for T -> (rot N, x=2) on an empty board.
fn t_left_two() -> Problem {
    let rs = Ruleset::tl_s2();
    let rules = MoveRules::from_ruleset(&rs);
    let costs = optimal_costs(&Default::default(), Piece::T, &rules, true);
    let target = TargetPlacement {
        piece: Piece::T,
        rot: Rot::N,
        x: 2,
        y: 0,
    };
    let opt = costs.get(&target.state().cell_key()).unwrap().clone();
    let (cost, path) = (opt.cost, opt.path);
    assert_eq!(cost, 3, "two left taps + hard drop");
    let h = witness_handling();
    let (inputs, end) = stacklab_training::finesse::path_to_inputs(&path, &h, 0);
    let initial = stacklab_engine::game::InitialState {
        rows: vec![0; 40],
        queue: vec![Piece::T],
        hold: None,
        b2b: -1,
        combo: -1,
        continue_with_bag: true,
    };
    let goal = Goal::Finesse {
        target,
        optimal_inputs: cost,
        allow_180: true,
    };
    let fp = fingerprint(Module::A, &initial, &goal);
    Problem {
        id: "test".into(),
        module: Module::A,
        subtype_tags: vec![],
        seed: 1,
        ruleset_version: "TL-S2".into(),
        generator_version: "test".into(),
        validation_version: "V1".into(),
        initial,
        visible_next: 5,
        goal,
        witness: Witness {
            handling: h,
            inputs,
            frames: end / 10 + 2,
            placements: vec![],
        },
        difficulty: 1500.0,
        param_bucket: "test".into(),
        time_limit_secs: 15,
        fingerprint: fp,
    }
}

fn run(p: &Problem, keys: &[(Key, u64)]) -> AttemptSession {
    let mut s = AttemptSession::new(p, witness_handling()).unwrap();
    let mut ev = Vec::new();
    let mut t = 0;
    for &(k, hold) in keys {
        ev.push(InputEvent {
            time: t,
            key: k,
            down: true,
        });
        ev.push(InputEvent {
            time: t + hold,
            key: k,
            down: false,
        });
        t += hold + 1;
    }
    for f in 0..200u64 {
        let frame: Vec<_> = ev.iter().filter(|e| e.time / 10 == f).copied().collect();
        s.step(&frame);
        if *s.status() != Status::Running {
            break;
        }
    }
    s
}

#[test]
fn equal_cost_finesse_paths_are_all_accepted() {
    let p = t_left_two();
    assert!(verify(&p).valid);
    // path 1: tap, tap, hard drop
    let a = run(&p, &[(Key::Left, 1), (Key::Left, 1), (Key::HardDrop, 1)]);
    // path 2: DAS left to the wall, tap right, hard drop (same cost 3)
    let b = run(&p, &[(Key::Left, 70), (Key::Right, 1), (Key::HardDrop, 1)]);
    // non-optimal: 3 taps left, 1 right
    let c = run(
        &p,
        &[
            (Key::Left, 1),
            (Key::Left, 1),
            (Key::Left, 1),
            (Key::Right, 1),
            (Key::HardDrop, 1),
        ],
    );
    for s in [&a, &b] {
        let r = s.result();
        assert!(r.success && r.finesse_success == Some(true), "{r:?}");
        assert!(r.valid_for_mmr);
    }
    let r = c.result();
    assert!(r.success, "placement is right");
    assert_eq!(
        r.finesse_success,
        Some(false),
        "but not with minimal inputs"
    );
    // wrong place
    let d = run(&p, &[(Key::Left, 1), (Key::HardDrop, 1)]);
    assert_eq!(d.result().reason, Some(FailReason::WrongPlacement));
}

#[test]
fn rewind_and_hint_exclude_attempt_from_rating() {
    let svc = service();
    let p = (0..50)
        .find_map(|k| {
            svc.generate(Module::B, 1500.0, k)
                .filter(|p| matches!(p.goal, Goal::Stacking { pieces, .. } if pieces >= 3))
        })
        .unwrap();
    let mut s = AttemptSession::new(&p, p.witness.handling).unwrap();
    let mut inputs = p.witness.inputs.clone();
    inputs.sort_by_key(|e| e.time);
    // play until two pieces are locked
    let mut f = 0u64;
    while s.pieces_locked() < 2 {
        let frame: Vec<_> = inputs
            .iter()
            .filter(|e| e.time / 10 == f)
            .copied()
            .collect();
        s.step(&frame);
        f += 1;
    }
    let board_after_one = {
        let mut t = AttemptSession::new(&p, p.witness.handling).unwrap();
        let mut g = 0u64;
        while t.pieces_locked() < 1 {
            let frame: Vec<_> = inputs
                .iter()
                .filter(|e| e.time / 10 == g)
                .copied()
                .collect();
            t.step(&frame);
            g += 1;
        }
        t.game().board().clone()
    };
    assert!(s.rewind());
    assert_eq!(s.pieces_locked(), 1);
    assert_eq!(
        s.game().board(),
        &board_after_one,
        "rewind restores the board exactly"
    );
    assert!(!s.result().valid_for_mmr);
    let mut s2 = AttemptSession::new(&p, p.witness.handling).unwrap();
    assert_eq!(
        s2.hint().map(|h| h.target),
        p.witness.placements.first().map(|w| w.target)
    );
    assert!(s2.result().used_hint);
}

#[test]
fn time_limit_fails_the_attempt() {
    let p = t_left_two();
    let mut s = AttemptSession::new(&p, witness_handling()).unwrap();
    for _ in 0..(15 * 60 + 2) {
        s.step(&[]);
    }
    assert_eq!(*s.status(), Status::Failed(FailReason::TimeUp));
}
