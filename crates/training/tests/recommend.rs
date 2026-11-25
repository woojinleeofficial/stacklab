//! STEP 6 scoring (spec 9.2): pair fit, 80 % interval, change only with a clear, confident gap.

use stacklab_opener::library::{coverage_table, Library};
use stacklab_opener::loadout::OpenerLoadout;
use stacklab_training::adaptive::SkillProfile;
use stacklab_training::glicko::Rating;
use stacklab_training::policy::Policy;
use stacklab_training::problem::Module;
use stacklab_training::recommend::{analyze, candidate_pairs, score_pair, Analysis};

fn base(primary: &str, fallback: &str) -> (SkillProfile, Library, Policy) {
    let policy = Policy::bundled();
    let lib = Library::bundled();
    let mut p = SkillProfile::new(&policy);
    p.loadout = Some(OpenerLoadout::new(&lib, "TL-S2", primary, fallback).unwrap());
    p.opener_valid_attempts = 12;
    p.opener_observed_rounds = 5;
    (p, lib, policy)
}

#[test]
fn scores_are_bounded_and_intervals_contain_them() {
    let (p, lib, policy) = base("tki", "free-transition");
    let cov = coverage_table();
    let pairs = candidate_pairs(&lib, "TL-S2");
    assert_eq!(pairs.len(), 6, "2 memorized primaries x 3 fallbacks");
    for (a, b) in pairs {
        let s = score_pair(&p, &lib, &cov, &policy, &a, &b).unwrap();
        assert!((0.0..=100.0).contains(&s.total), "{s:?}");
        assert!(s.low80 < s.total && s.total < s.high80);
        assert_eq!(s.components.len(), 5);
        assert!(s
            .components
            .iter()
            .all(|c| c.value <= c.weight + 1e-9 && c.value >= 0.0));
    }
}

#[test]
fn no_data_means_no_confident_change() {
    // default ratings (1500 ± 350) and no opener observations: intervals overlap -> keep
    let (p, lib, policy) = base("tki", "free-transition");
    match analyze(&p, &lib, &coverage_table(), &policy, "TL-S2") {
        Analysis::NoChange { current, .. } => {
            assert!(
                current.components.iter().any(|c| c.estimated),
                "unobserved values are marked as model estimates"
            );
        }
        other => panic!("expected NoChange, got {other:?}"),
    }
}

#[test]
fn insufficient_data_is_unavailable_not_a_guess() {
    let (mut p, lib, policy) = base("tki", "pco");
    p.opener_valid_attempts = 3;
    assert!(matches!(
        analyze(&p, &lib, &coverage_table(), &policy, "TL-S2"),
        Analysis::Unavailable { .. }
    ));
}

#[test]
fn clear_evidence_recommends_a_better_pair() {
    // strong PC skills and PCO proficiency, weak spins and a poor TKI record
    let (mut p, lib, policy) = base("tki", "free-transition");
    let set = |p: &mut SkillProfile, m: Module, r: f64| {
        p.skills.insert(m, Rating::new(r, 60.0, 0.06));
    };
    for (m, r) in [
        (Module::A, 1700.0),
        (Module::B, 1650.0),
        (Module::C, 1050.0),
        (Module::D, 1950.0),
        (Module::E, 1700.0),
        (Module::F, 1500.0),
        (Module::G, 1500.0),
        (Module::H, 1800.0),
    ] {
        set(&mut p, m, r);
    }
    p.openers
        .insert("tki".into(), Rating::new(1050.0, 60.0, 0.06));
    p.openers
        .insert("pco".into(), Rating::new(1900.0, 60.0, 0.06));
    p.openers
        .insert("free-transition".into(), Rating::new(1300.0, 60.0, 0.06));
    p.openers
        .insert("pc-first".into(), Rating::new(1850.0, 60.0, 0.06));
    p.branch_stats.insert("pco/with-i".into(), (30, 27));
    p.branch_stats.insert("tki/j-on-z".into(), (30, 6));
    match analyze(&p, &lib, &coverage_table(), &policy, "TL-S2") {
        Analysis::Change {
            current,
            recommended,
        } => {
            assert_eq!(recommended.primary, "pco");
            assert!(recommended.total - current.total >= 8.0);
            assert!(recommended.low80 > current.high80);
        }
        other => panic!("expected a change, got {other:?}"),
    }
}

#[test]
fn automatic_trigger_rules() {
    use stacklab_training::recommend::{automatic_trigger, new_event, RecStatus};
    use stacklab_training::set::{Observations, RoundRecord, SetState};
    let (mut p, lib, policy) = base("tki", "free-transition");
    let round = |won| RoundRecord {
        won,
        void: false,
        bot: "L4".into(),
        bot_rating: 1500.0,
        bot_rd: 30.0,
        calibration_version: 1000,
        observations: Observations::default(),
    };
    let at_eval = || {
        let mut s = SetState::new_first();
        s.diagnosis_done().unwrap();
        for _ in 0..2 {
            s.cycle_done().unwrap();
            s.skip_mini().unwrap();
        }
        s.cycle_done().unwrap();
        s
    };
    // deferred / closed without evaluation / not finished: never a trigger
    let mut deferred = at_eval();
    deferred.defer_evaluation().unwrap();
    assert!(!automatic_trigger(&p, &deferred, &policy, false));
    let mut unrated = at_eval();
    unrated.close_without_evaluation().unwrap();
    assert!(!automatic_trigger(&p, &unrated, &policy, false));
    let mut done = at_eval();
    done.eval_round(round(true)).unwrap();
    assert!(
        !automatic_trigger(&p, &done, &policy, false),
        "BO3 not finished yet"
    );
    done.eval_round(round(true)).unwrap();
    // data conditions
    p.opener_observed_rounds = 3;
    assert!(
        !automatic_trigger(&p, &done, &policy, false),
        "needs 4 observed rounds"
    );
    p.opener_observed_rounds = 4;
    p.opener_valid_attempts = 9;
    assert!(
        !automatic_trigger(&p, &done, &policy, false),
        "needs 10 opener attempts"
    );
    p.opener_valid_attempts = 10;
    assert!(
        automatic_trigger(&p, &done, &policy, false),
        "first qualifying evaluation"
    );
    assert!(
        !automatic_trigger(&p, &done, &policy, true),
        "never again once an event exists"
    );
    // the event runs PENDING -> ANALYZING -> one of READY / NO_CHANGE / UNAVAILABLE
    let mut e = new_event("e1", Some("set-1"), true, 1);
    e.run(
        &p,
        &lib,
        &stacklab_opener::library::coverage_table(),
        &policy,
        "TL-S2",
    )
    .unwrap();
    assert!(matches!(
        e.status,
        RecStatus::Ready | RecStatus::NoChange | RecStatus::Unavailable
    ));
    assert!(e.analysis.is_some());
    assert!(e.transition(RecStatus::Pending).is_err());
}
