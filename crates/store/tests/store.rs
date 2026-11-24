use stacklab_engine::Ruleset;
use stacklab_store::{migrate, Store, MIGRATIONS};
use stacklab_training::policy::Policy;
use stacklab_training::problem::Module;
use stacklab_training::service::ProblemService;
use stacklab_training::session::AttemptSession;
use std::sync::Arc;

fn problems(n: u64) -> Vec<stacklab_training::Problem> {
    let svc = ProblemService::new(Arc::new(Ruleset::tl_s2()), Policy::bundled());
    (0..n * 3)
        .filter_map(|k| svc.generate(if k % 2 == 0 { Module::A } else { Module::B }, 1500.0, k))
        .take(n as usize)
        .collect()
}

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("stacklab-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_file(&d);
    d
}

fn count(s: &Store) -> usize {
    s.cached_problems(Module::A).unwrap().len() + s.cached_problems(Module::B).unwrap().len()
}

#[test]
fn schema_is_migrated_and_ruleset_registered() {
    let s = Store::open_in_memory().unwrap();
    assert_eq!(s.schema_version().unwrap(), 1);
    let n: i64 = s
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM ruleset WHERE version='TL-S2'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
}

#[test]
fn problems_roundtrip_and_stay_valid() {
    let s = Store::open_in_memory().unwrap();
    for p in problems(6) {
        s.save_problem(&p, true).unwrap();
        let back = s.load_problem(&p.id).unwrap().unwrap();
        assert_eq!(back, p);
        assert!(stacklab_training::validator::verify(&back).valid);
    }
    assert_eq!(count(&s), 6);
}

#[test]
fn a_cycle_slot_is_counted_once() {
    let s = Store::open_in_memory().unwrap();
    let p = &problems(1)[0];
    s.save_problem(p, false).unwrap();
    s.conn()
        .execute(
            "INSERT INTO training_set(id, status, created_at) VALUES ('s1','TRAINING_1','x')",
            [],
        )
        .unwrap();
    s.conn()
        .execute(
            "INSERT INTO cycle(id, set_id, cycle_index, status, created_at) VALUES ('c1','s1',1,'open','x')",
            [],
        )
        .unwrap();
    let r = AttemptSession::new(p, p.witness.handling).unwrap().result();
    s.save_attempt(Some("c1"), Some(1), true, p, &r, "[]")
        .unwrap();
    // a retry of the same slot is stored but not counted
    s.save_attempt(Some("c1"), Some(1), false, p, &r, "[]")
        .unwrap();
    assert!(
        s.save_attempt(Some("c1"), Some(1), true, p, &r, "[]")
            .is_err(),
        "a second counted attempt for the slot is rejected"
    );
}

#[test]
fn backup_restore_roundtrip_on_disk() {
    let path = tmp("backup.db");
    let backup = {
        let s = Store::open(&path).unwrap();
        for p in problems(4) {
            s.save_problem(&p, true).unwrap();
        }
        s.backup_json().unwrap()
    };
    let s = Store::open(&path).unwrap();
    assert_eq!(count(&s), 4, "data persisted across reopen");
    drop(s);
    let fresh_path = tmp("restore.db");
    let mut fresh = Store::open(&fresh_path).unwrap();
    fresh.restore_json(&backup).unwrap();
    let strip = |b: &str| {
        let mut v: serde_json::Value = serde_json::from_str(b).unwrap();
        v.as_object_mut().unwrap().remove("created_at");
        v
    };
    assert_eq!(strip(&fresh.backup_json().unwrap()), strip(&backup));
    // a broken backup leaves the data untouched
    assert!(fresh
        .restore_json("{\"schema_version\":1,\"tables\":{\"problem\":[{\"nope\":1}]}}")
        .is_err());
    assert_eq!(count(&fresh), 4);
    drop(fresh);
    for p in [&path, &fresh_path] {
        let _ = std::fs::remove_file(p);
    }
}

#[test]
fn future_migration_keeps_existing_data() {
    let path = tmp("migrate.db");
    {
        let s = Store::open(&path).unwrap();
        for p in problems(2) {
            s.save_problem(&p, true).unwrap();
        }
    }
    let mut conn = rusqlite::Connection::open(&path).unwrap();
    let mut list: Vec<(u32, &str)> = MIGRATIONS.to_vec();
    list.push((
        2,
        "ALTER TABLE problem ADD COLUMN note TEXT NOT NULL DEFAULT '';",
    ));
    assert_eq!(migrate(&mut conn, &list).unwrap(), 2);
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM problem WHERE note = ''", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(n, 2);
    // a failing migration rolls back and keeps the previous version
    list.push((
        3,
        "ALTER TABLE problem ADD COLUMN bad TEXT; THIS IS NOT SQL;",
    ));
    assert!(migrate(&mut conn, &list).is_err());
    let v: u32 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, 2);
    drop(conn);
    let _ = std::fs::remove_file(&path);
}
