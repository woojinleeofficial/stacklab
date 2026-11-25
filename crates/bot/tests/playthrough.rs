//! Spec 14.4 automatic play script: a bot plays the user's part through the whole flow —
//! diagnosis, 36 problems, 2 mini matches, BO3 evaluation and the one-time opener
//! recommendation (created, persisted, approved in two steps, applied once).

use stacklab_bot::league::play_round_observed;
use stacklab_bot::{calibration, BotConfig};
use stacklab_engine::replay::VersusReplay;
use stacklab_engine::rng::Rng;
use stacklab_engine::Ruleset;
use stacklab_opener::library::{coverage_table, Library};
use stacklab_opener::loadout::OpenerLoadout;
use stacklab_store::{ApplyOutcome, MatchRow, RoundRow, Store};
use stacklab_training::flow::{ProblemRequest, RoundKind, TrainingState};
use stacklab_training::policy::Policy;
use stacklab_training::recommend::{new_event, RecStatus};
use stacklab_training::service::ProblemService;
use stacklab_training::session::{AttemptSession, Status};
use stacklab_training::set::{opener_built, Observations, Phase};
use std::sync::Arc;

/// Plays the witness (success) or does nothing until the time runs out (failure).
fn attempt(
    p: &stacklab_training::Problem,
    solve: bool,
) -> stacklab_training::session::AttemptResult {
    let mut s = AttemptSession::new(p, p.witness.handling).unwrap();
    let mut inputs = p.witness.inputs.clone();
    inputs.sort_by_key(|e| e.time);
    let mut f = 0u64;
    while *s.status() == Status::Running && f < p.time_limit_secs as u64 * 60 + 10 {
        let frame: Vec<_> = if solve {
            inputs
                .iter()
                .filter(|e| e.time / 10 == f)
                .copied()
                .collect()
        } else {
            vec![]
        };
        s.step(&frame);
        f += 1;
    }
    s.result()
}

#[test]
fn bot_completes_a_full_set_and_the_recommendation_flow() {
    let rs = Arc::new(Ruleset::tl_s2());
    let mut policy = Policy::bundled();
    // more opener slots so one set reaches the recommendation data condition (10 attempts)
    policy.scheduler.domain_floor.insert("E".into(), 4);
    let lib = Library::bundled();
    let cov = coverage_table();
    let cal = calibration();
    let league: Vec<(String, f64, f64)> = cal
        .bot
        .iter()
        .map(|b| (b.id.clone(), b.rating, b.rd))
        .collect();
    let mut store = Store::open_in_memory().unwrap();
    let lo = store
        .set_loadout("tki", "free-transition", None, "initial", None)
        .unwrap();
    let mut st = TrainingState::new(&policy, 2026);
    st.profile.loadout = Some(lo.clone());
    let mut svc = ProblemService::new(rs.clone(), policy.clone());
    let mut rng = Rng::new(9);
    let user = BotConfig::preset("L4");
    let shapes: Vec<stacklab_engine::Board> = ["tki", "free-transition"]
        .iter()
        .filter_map(|id| lib.get(id))
        .flat_map(|d| d.branches.iter().map(|b| b.target.clone()))
        .collect();
    let (mut problems, mut minis, mut eval_rounds, mut events_created) = (0, 0, 0, 0);
    let mut round_seed = 500;

    for _ in 0..400 {
        match st.set.phase {
            Phase::Diagnosis | Phase::Training(_) => {
                let req = st.next_request().expect("a problem slot");
                let p = match req {
                    ProblemRequest::Anchor { module, k } => {
                        svc.anchor(module, k).expect("anchor problem")
                    }
                    ProblemRequest::Slot { module, seed } => {
                        let mut r = Rng::new(seed);
                        svc.next_for(&st.profile, module, &mut r)
                            .expect("problem served")
                            .0
                    }
                };
                assert!(stacklab_training::validator::verify(&p).valid, "{}", p.id);
                let solve = rng.below(100) < 75;
                let res = attempt(&p, solve);
                assert_eq!(res.success, solve, "{} {:?}", p.id, res.reason);
                st.record_problem(&p, res, &policy);
                problems += 1;
            }
            Phase::Mini(_) | Phase::Evaluation => {
                let (id, r, rd, _, _) = st.opponent(&league, &policy).expect("opponent");
                round_seed += 1;
                let (o, events, boards, replay) =
                    play_round_observed(&user, &BotConfig::preset(&id), &rs, round_seed);
                let mid = match st.set.phase {
                    Phase::Mini(k) => format!("set{}-mini{k}", st.set.number),
                    _ => format!("set{}-eval", st.set.number),
                };
                let kind_s = if matches!(st.set.phase, Phase::Mini(_)) {
                    "mini"
                } else {
                    "evaluation"
                };
                store
                    .begin_match(&MatchRow {
                        id: mid.clone(),
                        kind: kind_s.into(),
                        bot_preset_id: id.clone(),
                        bot_rating: r,
                        calibration_version: cal.calibration_version,
                        out_of_band: false,
                        ruleset_version: "TL-S2".into(),
                        status: "in_progress".into(),
                        result: None,
                        rating_delta: None,
                        created_at: String::new(),
                    })
                    .unwrap();
                let idx = store.rounds(&mid).unwrap().len() as u32;
                store
                    .save_round(&RoundRow {
                        match_id: mid.clone(),
                        round_index: idx,
                        result: Some(o.winner == 0),
                        void: false,
                        replay: Some(replay.to_json()),
                        observations: None,
                    })
                    .unwrap();
                if kind_s == "mini" {
                    store
                        .finish_match(&mid, "completed", Some(o.winner == 0), None)
                        .unwrap();
                }
                let mut obs = Observations::from_events(&events);
                obs.opener_built = Some(opener_built(&boards, &shapes));
                let kind = if matches!(st.set.phase, Phase::Mini(_)) {
                    minis += 1;
                    RoundKind::Mini
                } else {
                    eval_rounds += 1;
                    RoundKind::Evaluation
                };
                let exists = store.has_automatic_recommendation().unwrap();
                let out = st
                    .record_round(
                        kind,
                        o.winner == 0,
                        &id,
                        r,
                        rd,
                        cal.calibration_version,
                        obs,
                        &policy,
                        exists,
                    )
                    .unwrap();
                if let Some(ev) = out {
                    assert!(ev.wins == 2 || ev.losses == 2);
                    store
                        .finish_match(
                            &mid,
                            "completed",
                            Some(ev.wins == 2),
                            Some(ev.after.rating - ev.before.rating),
                        )
                        .unwrap();
                    if ev.trigger_recommendation {
                        let mut e = new_event(
                            "auto-1",
                            Some(&format!("set-{}", st.set.number)),
                            true,
                            st.profile.loadout.as_ref().unwrap().version,
                        );
                        e.run(&st.profile, &lib, &cov, &policy, "TL-S2").unwrap();
                        store.create_recommendation(&e).unwrap();
                        events_created += 1;
                        if e.status == RecStatus::Ready {
                            // two-step approval: confirm -> one transaction; a second click is a no-op
                            match store.apply_recommendation("auto-1", &lib, "TL-S2").unwrap() {
                                ApplyOutcome::Applied(new_lo) => st.profile.loadout = Some(new_lo),
                                other => panic!("{other:?}"),
                            }
                            assert_eq!(
                                store.apply_recommendation("auto-1", &lib, "TL-S2").unwrap(),
                                ApplyOutcome::AlreadyApplied
                            );
                        }
                    }
                }
            }
            Phase::Result => {
                if events_created > 0 && st.set.number >= 2 {
                    break;
                }
                st.next_set().unwrap();
            }
        }
    }
    assert!(problems >= 7 + 36, "diagnosis + 36 problems ({problems})");
    assert!(minis >= 2, "two mini matches");
    assert!(
        (2..=3).contains(&eval_rounds) || eval_rounds >= 4,
        "BO3 rounds played ({eval_rounds})"
    );
    assert_eq!(events_created, 1, "exactly one automatic recommendation");
    assert!(store.has_automatic_recommendation().unwrap());
    let e = store.load_recommendation("auto-1").unwrap().unwrap();
    assert!(
        matches!(
            e.status,
            RecStatus::Applied | RecStatus::NoChange | RecStatus::Unavailable
        ),
        "{:?}",
        e.status
    );
    assert!(st.matchp.evaluations >= 1);
    // every stored round replays to the same final state (spec 10.1 Round.replay)
    let matches = store.list_matches(50).unwrap();
    assert!(matches
        .iter()
        .any(|m| m.kind == "evaluation" && m.status == "completed" && m.rating_delta.is_some()));
    assert!(matches.iter().filter(|m| m.kind == "mini").count() >= 2);
    for m in &matches {
        for r in store.rounds(&m.id).unwrap() {
            let rep = VersusReplay::from_json(r.replay.as_deref().unwrap()).unwrap();
            assert!(rep.verify().unwrap(), "{} round {}", m.id, r.round_index);
        }
    }
    let _ = OpenerLoadout::new(&lib, "TL-S2", "tki", "pco");
}
