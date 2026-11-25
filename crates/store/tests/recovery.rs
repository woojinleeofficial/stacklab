//! Spec 11 / 14.4: forced exit mid-problem or mid-write resumes from the last checkpoint; the
//! unsubmitted problem is not counted.

use stacklab_engine::Ruleset;
use stacklab_store::{Checkpoint, Store};
use stacklab_training::flow::{ProblemRequest, TrainingState};
use stacklab_training::policy::Policy;
use stacklab_training::service::ProblemService;
use stacklab_training::session::AttemptSession;
use std::sync::Arc;

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("stacklab-recovery-{}-{name}", std::process::id()));
    for ext in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{ext}", d.display()));
    }
    d
}

fn serve(svc: &mut ProblemService, st: &mut TrainingState) -> stacklab_training::Problem {
    match st.next_request().unwrap() {
        ProblemRequest::Anchor { module, k } => svc.anchor(module, k).unwrap(),
        ProblemRequest::Slot { module, seed } => {
            let mut r = stacklab_engine::rng::Rng::new(seed);
            svc.next_for(&st.profile, module, &mut r).unwrap().0
        }
    }
}

#[test]
fn forced_exit_resumes_from_the_last_checkpoint() {
    let path = tmp("ckpt.db");
    let policy = Policy::bundled();
    let mut svc = ProblemService::new(Arc::new(Ruleset::tl_s2()), policy.clone());
    let mut st = TrainingState::new(&policy, 77);
    // two decided problems, each followed by a checkpoint
    for _ in 0..2 {
        let p = serve(&mut svc, &mut st);
        let s = AttemptSession::new(&p, p.witness.handling).unwrap();
        let mut r = s.result();
        r.success = false;
        st.record_problem(&p, r, &policy);
    }
    let current = serve(&mut svc, &mut st);
    let saved_json = serde_json::to_string(&st).unwrap();
    {
        let mut store = Store::open(&path).unwrap();
        store
            .write_checkpoint(&Checkpoint {
                training: saved_json.clone(),
                current_problem: Some(serde_json::to_string(&current).unwrap()),
                open_round: None,
            })
            .unwrap();
    }
    // a child process dies (abort) in the middle of the next checkpoint write
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_child", "--nocapture", "--test-threads=1"])
        .env("STACKLAB_CRASH_DB", &path)
        .status()
        .unwrap();
    assert!(
        !status.success() && status.code() != Some(101),
        "the child must abort mid-write, not fail: {status:?}"
    );
    let store = Store::open(&path).unwrap();
    let c = store
        .read_checkpoint()
        .unwrap()
        .expect("checkpoint survives");
    assert_eq!(
        c.training, saved_json,
        "the last committed checkpoint, not the torn write"
    );
    let resumed: TrainingState = serde_json::from_str(&c.training).unwrap();
    assert_eq!(
        resumed.cycle.results.len(),
        2,
        "the unsubmitted problem is not counted"
    );
    let p: stacklab_training::Problem =
        serde_json::from_str(c.current_problem.as_deref().unwrap()).unwrap();
    assert_eq!(p.id, current.id, "the same problem is shown again");
    assert_eq!(serde_json::to_string(&resumed).unwrap(), saved_json);
}

/// Helper run in a child process by the test above: opens a transaction, writes half a
/// checkpoint and aborts the process without committing.
#[test]
fn crash_child() {
    let Some(path) = std::env::var_os("STACKLAB_CRASH_DB") else {
        return;
    };
    let store = Store::open(std::path::Path::new(&path)).unwrap();
    store
        .conn()
        .execute_batch(
            "BEGIN; UPDATE app_state SET value = '{broken' WHERE key = 'training';
             DELETE FROM app_state WHERE key = 'current_problem';",
        )
        .unwrap();
    std::process::abort();
}

#[test]
fn open_round_marker_survives_and_clears() {
    let path = tmp("round.db");
    let mut store = Store::open(&path).unwrap();
    let st = TrainingState::new(&Policy::bundled(), 5);
    let training = serde_json::to_string(&st).unwrap();
    store
        .write_checkpoint(&Checkpoint {
            training: training.clone(),
            current_problem: None,
            open_round: Some("set1-eval".into()),
        })
        .unwrap();
    drop(store);
    let mut store = Store::open(&path).unwrap();
    let c = store.read_checkpoint().unwrap().unwrap();
    assert_eq!(c.open_round.as_deref(), Some("set1-eval"));
    store
        .write_checkpoint(&Checkpoint {
            training,
            current_problem: None,
            open_round: None,
        })
        .unwrap();
    assert_eq!(store.read_checkpoint().unwrap().unwrap().open_round, None);
}
