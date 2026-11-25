//! Spec 14.3 recommendation items on the real database.

use stacklab_opener::library::Library;
use stacklab_store::{ApplyOutcome, Store};
use stacklab_training::recommend::{new_event, Analysis, Component, PairScore, RecStatus};

fn pair(p: &str, f: &str, total: f64) -> PairScore {
    PairScore {
        primary: p.into(),
        fallback: f.into(),
        total,
        sd: 2.0,
        low80: total - 2.6,
        high80: total + 2.6,
        components: vec![Component {
            key: "complement".into(),
            weight: 20.0,
            value: 10.0,
            sd: 1.0,
            estimated: false,
        }],
        reasons: vec![],
    }
}

fn ready(id: &str, version: u32) -> stacklab_training::recommend::RecommendationEvent {
    let mut e = new_event(id, Some("set-1"), true, version);
    e.transition(RecStatus::Analyzing).unwrap();
    e.analysis = Some(Analysis::Change {
        current: pair("tki", "free-transition", 60.0),
        recommended: pair("pco", "tki", 75.0),
    });
    e.transition(RecStatus::Ready).unwrap();
    e
}

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("stacklab-rec-{}-{name}", std::process::id()));
    let _ = std::fs::remove_file(&d);
    d
}

#[test]
fn only_one_automatic_event_ever() {
    let s = Store::open_in_memory().unwrap();
    s.set_loadout_initial();
    assert!(!s.has_automatic_recommendation().unwrap());
    s.create_recommendation(&new_event("e1", Some("set-1"), true, 1))
        .unwrap();
    assert!(s.has_automatic_recommendation().unwrap());
    assert!(
        s.create_recommendation(&new_event("e2", Some("set-2"), true, 1))
            .is_err(),
        "unique automatic event"
    );
    // manual re-analysis is always allowed
    s.create_recommendation(&new_event("m1", None, false, 1))
        .unwrap();
    s.create_recommendation(&new_event("m2", None, false, 1))
        .unwrap();
}

trait Init {
    fn set_loadout_initial(&self);
}

impl Init for Store {
    fn set_loadout_initial(&self) {
        self.conn()
            .execute(
                "INSERT INTO opener_loadout(id, primary_opener_id, fallback_opener_id, version, updated_at, created_at) VALUES (1,'tki','free-transition',1,'x','x')",
                [],
            )
            .unwrap();
    }
}

#[test]
fn apply_needs_ready_is_idempotent_and_survives_restart() {
    let lib = Library::bundled();
    let path = tmp("apply.db");
    {
        let mut s = Store::open(&path).unwrap();
        s.set_loadout_initial();
        let pending = new_event("p1", None, false, 1);
        s.create_recommendation(&pending).unwrap();
        assert!(
            s.apply_recommendation("p1", &lib, "TL-S2").is_err(),
            "no apply before analysis/approval"
        );
        s.create_recommendation(&ready("e1", 1)).unwrap();
    }
    // the READY state persists across a restart
    let mut s = Store::open(&path).unwrap();
    assert_eq!(
        s.load_recommendation("e1").unwrap().unwrap().status,
        RecStatus::Ready
    );
    match s.apply_recommendation("e1", &lib, "TL-S2").unwrap() {
        ApplyOutcome::Applied(lo) => assert_eq!(
            (lo.primary.as_str(), lo.fallback.as_str(), lo.version),
            ("pco", "tki", 2)
        ),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        s.apply_recommendation("e1", &lib, "TL-S2").unwrap(),
        ApplyOutcome::AlreadyApplied,
        "duplicate click"
    );
    let n: i64 = s
        .conn()
        .query_row("SELECT COUNT(*) FROM opener_change_history", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(n, 1, "changed exactly once");
    drop(s);
    let s = Store::open(&path).unwrap();
    assert_eq!(
        s.load_recommendation("e1").unwrap().unwrap().status,
        RecStatus::Applied
    );
    assert_eq!(s.load_loadout().unwrap().unwrap().primary, "pco");
    drop(s);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn manual_change_after_analysis_is_a_conflict() {
    let lib = Library::bundled();
    let mut s = Store::open_in_memory().unwrap();
    s.set_loadout_initial();
    s.create_recommendation(&ready("e1", 1)).unwrap();
    s.set_loadout("tki", "pc-first", Some(1), "manual", None)
        .unwrap();
    assert_eq!(
        s.apply_recommendation("e1", &lib, "TL-S2").unwrap(),
        ApplyOutcome::Conflict {
            current_version: 2,
            event_version: 1
        }
    );
    let lo = s.load_loadout().unwrap().unwrap();
    assert_eq!(
        (lo.primary.as_str(), lo.fallback.as_str()),
        ("tki", "pc-first"),
        "manual choice kept"
    );
    assert_eq!(
        s.load_recommendation("e1").unwrap().unwrap().status,
        RecStatus::Ready,
        "needs a new approval"
    );
}

#[test]
fn failure_inside_the_transaction_rolls_back() {
    let lib = Library::bundled();
    let mut s = Store::open_in_memory().unwrap();
    s.set_loadout_initial();
    s.create_recommendation(&ready("e1", 1)).unwrap();
    // a stale history row with the same approval id makes the history insert fail mid-way
    s.conn()
        .execute("INSERT INTO opener_change_history(before_json, after_json, reason, approval_event_id, created_at) VALUES ('{}','{}','x','e1','x')", [])
        .unwrap();
    assert!(s.apply_recommendation("e1", &lib, "TL-S2").is_err());
    let lo = s.load_loadout().unwrap().unwrap();
    assert_eq!(
        (lo.primary.as_str(), lo.version),
        ("tki", 1),
        "loadout untouched"
    );
    assert_eq!(
        s.load_recommendation("e1").unwrap().unwrap().status,
        RecStatus::Ready
    );
}

#[test]
fn illegal_transitions_are_rejected_on_save() {
    let s = Store::open_in_memory().unwrap();
    let mut e = ready("e1", 1);
    s.create_recommendation(&e).unwrap();
    e.status = RecStatus::NoChange; // READY -> NO_CHANGE is not an edge
    assert!(s.save_recommendation(&e).is_err());
    let mut e = s.load_recommendation("e1").unwrap().unwrap();
    e.transition(RecStatus::Deferred).unwrap();
    s.save_recommendation(&e).unwrap();
    assert_eq!(
        s.load_recommendation("e1").unwrap().unwrap().status,
        RecStatus::Deferred
    );
}
