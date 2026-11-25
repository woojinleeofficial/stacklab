//! UI-09 records: per-module attempt statistics (counted attempts only) and 40-line records
//! with verifiable replays.

use stacklab_engine::game::{Handling, InputEvent, Key, Mode};
use stacklab_engine::replay::{Recorder, ReplayLog};
use stacklab_engine::Ruleset;
use stacklab_store::Store;
use stacklab_training::policy::Policy;
use stacklab_training::problem::Module;
use stacklab_training::service::ProblemService;
use stacklab_training::session::AttemptSession;
use std::sync::Arc;

#[test]
fn module_stats_count_only_deciding_attempts() {
    let s = Store::open_in_memory().unwrap();
    let svc = ProblemService::new(Arc::new(Ruleset::tl_s2()), Policy::bundled());
    let p = svc.anchor(Module::A, 0).unwrap();
    s.ensure_problem(&p).unwrap();
    s.ensure_problem(&p).unwrap(); // idempotent, keeps the row attempts refer to
    let mut r = AttemptSession::new(&p, p.witness.handling)
        .unwrap()
        .result();
    r.success = true;
    r.valid_for_mmr = true;
    r.finesse_success = Some(false);
    s.save_attempt(None, Some(1), true, &p, &r, "[]").unwrap();
    s.save_attempt(None, Some(1), false, &p, &r, "[]").unwrap(); // a retry: not counted
    let st = s.module_stats().unwrap();
    assert_eq!(st, vec![("A".to_string(), 1, 1, 1, 0, 1)]);
}

#[test]
fn sprint_records_sort_and_replay() {
    let s = Store::open_in_memory().unwrap();
    let rs = Arc::new(Ruleset::tl_s2());
    let mut rec = Recorder::new(rs, 4, Handling::default(), Mode::Sprint { lines: 40 }, None);
    for f in 0..120u64 {
        let ev = if f % 20 == 0 {
            vec![
                InputEvent {
                    time: f * 10,
                    key: Key::HardDrop,
                    down: true,
                },
                InputEvent {
                    time: f * 10 + 1,
                    key: Key::HardDrop,
                    down: false,
                },
            ]
        } else {
            vec![]
        };
        rec.step(&ev);
    }
    let log = rec.finish();
    s.save_sprint(None, 6, 6, &log.to_json(), "TL-S2").unwrap();
    s.save_sprint(Some(3000), 100, 250, &log.to_json(), "TL-S2")
        .unwrap();
    s.save_sprint(Some(2500), 100, 240, &log.to_json(), "TL-S2")
        .unwrap();
    let rows = s.sprint_records(10).unwrap();
    let order: Vec<Option<u64>> = rows.iter().map(|r| r.finish_ticks).collect();
    assert_eq!(
        order,
        vec![Some(2500), Some(3000), None],
        "fastest first, unfinished last"
    );
    assert!(ReplayLog::from_json(&rows[0].replay)
        .unwrap()
        .verify()
        .unwrap());
}
