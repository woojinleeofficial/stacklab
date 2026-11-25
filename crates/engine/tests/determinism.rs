//! Spec 14.1: 1,000+ seed replays reproduce the final state exactly, random-input fuzzing keeps
//! invariants, and replays saved by an earlier build still reproduce (golden files).

mod common;

use common::rs;
use stacklab_engine::board::FULL_ROW;
use stacklab_engine::game::{Game, Handling, InitialState, InputEvent, Key, Mode};
use stacklab_engine::replay::{Recorder, ReplayLog, VersusReplay, REPLAY_FORMAT};
use stacklab_engine::rng::Rng;
use stacklab_engine::versus::Versus;
use std::path::PathBuf;

const KEYS: [Key; 8] = [
    Key::Left,
    Key::Right,
    Key::SoftDrop,
    Key::HardDrop,
    Key::Cw,
    Key::Ccw,
    Key::Flip,
    Key::Hold,
];

/// Random but plausible inputs for one frame: taps and holds at random sub-frames.
fn random_frame_inputs(rng: &mut Rng, tick: u64, held: &mut [bool; 8]) -> Vec<InputEvent> {
    let mut v = Vec::new();
    let base = tick * 10;
    for _ in 0..rng.below(3) {
        let k = rng.below(8) as usize;
        let t = base + rng.below(10);
        // hard drop more often so games progress
        let k = if rng.chance_pm(250) { 3 } else { k };
        held[k] = !held[k];
        v.push(InputEvent {
            time: t,
            key: KEYS[k],
            down: held[k],
        });
    }
    v.sort_by_key(|e| e.time);
    v
}

fn random_handling(rng: &mut Rng) -> Handling {
    Handling {
        das: 10 + rng.below(190) as u32,
        arr: rng.below(50) as u32,
        sdf: rng.below(41) as u32,
        dcd: rng.below(30) as u32,
    }
}

fn check_invariants(g: &Game) {
    for r in g.board().rows {
        assert_eq!(r & !FULL_ROW, 0, "no cells outside the matrix");
        assert_ne!(r, FULL_ROW, "full rows are always cleared");
    }
    for (y, r) in g.board().rows.iter().enumerate() {
        for x in 0..10 {
            assert_eq!(
                r >> x & 1 == 1,
                g.colors().at(x, y) != 0,
                "colour layer tracks cell ({x},{y})"
            );
        }
    }
    if let Some(p) = g.active_piece() {
        assert!(g.board().fits(&p), "active piece never overlaps the stack");
        assert!(!g.is_over());
    }
}

fn record_single(seed: u64, frames: u64) -> ReplayLog {
    let mut rng = Rng::new(seed * 7 + 3);
    let handling = random_handling(&mut rng);
    let mut rec = Recorder::new(rs(), seed, handling, Mode::Free, None);
    let mut held = [false; 8];
    for f in 0..frames {
        let inp = random_frame_inputs(&mut rng, f, &mut held);
        rec.step(&inp);
        check_invariants(&rec.game);
    }
    rec.finish()
}

#[test]
fn thousand_seed_replays_match() {
    let mut finished = 0;
    for seed in 1..=1000u64 {
        let log = record_single(seed, 300);
        assert!(log.verify().unwrap(), "seed {seed} replay diverged");
        if log.run().unwrap().stats().pieces > 0 {
            finished += 1;
        }
    }
    assert!(
        finished > 900,
        "fuzz inputs should actually place pieces ({finished})"
    );
}

#[test]
fn json_roundtrip_replays_match() {
    for seed in 1..=50u64 {
        let log = record_single(seed, 600);
        let back = ReplayLog::from_json(&log.to_json()).unwrap();
        assert_eq!(back, log);
        assert!(back.verify().unwrap());
    }
}

#[test]
fn snapshot_restore_is_exact() {
    let mut rng = Rng::new(99);
    let mut g = Game::new(rs(), 5, Handling::default(), Mode::Free);
    let mut held = [false; 8];
    for f in 0..200 {
        let inp = random_frame_inputs(&mut rng, f, &mut held);
        g.step(&inp);
    }
    let snap = g.snapshot();
    let mut a = g.clone();
    let mut held_a = held;
    let mut rng_a = rng.clone();
    for f in 200..400 {
        let inp = random_frame_inputs(&mut rng_a, f, &mut held_a);
        a.step(&inp);
    }
    g.restore(&snap);
    for f in 200..400 {
        let inp = random_frame_inputs(&mut rng, f, &mut held);
        g.step(&inp);
    }
    assert_eq!(a.state_hash(), g.state_hash());
}

fn record_versus(seed: u64, frames: u64) -> VersusReplay {
    let mut rng = Rng::new(seed * 13 + 1);
    let handling = [common::fast_handling(), common::fast_handling()];
    let seeds = [seed, seed + 100_000];
    // pre-filled rows with one hole each so random play clears lines and sends garbage
    let mk = |rng: &mut Rng| {
        let mut rows = Vec::new();
        for _ in 0..4 {
            rows.push(FULL_ROW & !(1 << rng.below(10)));
        }
        Some(InitialState {
            rows,
            continue_with_bag: true,
            b2b: -1,
            combo: -1,
            ..Default::default()
        })
    };
    let initial = [mk(&mut rng), mk(&mut rng)];
    let mut v = Versus::with_initial(
        rs(),
        seeds,
        handling,
        [initial[0].as_ref(), initial[1].as_ref()],
    );
    let mut players = [
        common::PlacementPlayer::new(seed * 3),
        common::PlacementPlayer::new(seed * 3 + 1),
    ];
    let mut inputs = [Vec::new(), Vec::new()];
    let mut n = 0;
    for _ in 0..frames {
        let a = players[0].frame_inputs(&v.players[0]);
        let b = players[1].frame_inputs(&v.players[1]);
        v.step([&a, &b]);
        check_invariants(&v.players[0]);
        check_invariants(&v.players[1]);
        inputs[0].extend(a);
        inputs[1].extend(b);
        n += 1;
        if v.winner().is_some() {
            break;
        }
    }
    VersusReplay {
        format: REPLAY_FORMAT,
        ruleset_version: "TL-S2".into(),
        seeds,
        handling,
        initial,
        frames: n,
        inputs,
        final_hash: v.state_hash(),
    }
}

#[test]
fn versus_replays_match_and_garbage_flows() {
    let mut garbage_seen = false;
    let (mut lines, mut attack, mut frames) = (0, 0, 0);
    for seed in 1..=100u64 {
        let r = record_versus(seed, 6000);
        let v = r.run().unwrap();
        assert_eq!(v.state_hash(), r.final_hash, "versus seed {seed} diverged");
        garbage_seen |= v
            .players
            .iter()
            .any(|p| p.stats().received > 0 || p.pending_garbage() > 0);
        lines += v.players.iter().map(|p| p.stats().lines).sum::<u32>();
        attack += v.players.iter().map(|p| p.stats().attack).sum::<u32>();
        frames += r.frames;
    }
    assert!(
        frames / 100 > 300,
        "games should last (avg frames {})",
        frames / 100
    );
    assert!(attack > 50, "attack {attack}");
    assert!(
        garbage_seen,
        "random play should send some garbage (lines {lines}, attack {attack}, frames {frames})"
    );
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/replays")
}

/// Replays written by an earlier build must reproduce (same rulesetVersion).
/// Regenerate only on purpose: `STACKLAB_BLESS=1 cargo test --release golden`.
#[test]
fn golden_replays_from_previous_builds() {
    let dir = golden_dir();
    if std::env::var("STACKLAB_BLESS").is_ok() {
        std::fs::create_dir_all(&dir).unwrap();
        for seed in [11u64, 22, 33] {
            let log = record_single(seed, 1800);
            std::fs::write(dir.join(format!("single_seed{seed}.json")), log.to_json()).unwrap();
        }
        let v = record_versus(44, 3000);
        std::fs::write(
            dir.join("versus_seed44.json"),
            serde_json::to_string(&v).unwrap(),
        )
        .unwrap();
    }
    let mut count = 0;
    for entry in std::fs::read_dir(&dir).expect("golden replay directory exists") {
        let path = entry.unwrap().path();
        let text = std::fs::read_to_string(&path).unwrap();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if name.starts_with("versus") {
            let r: VersusReplay = serde_json::from_str(&text).unwrap();
            assert_eq!(r.run().unwrap().state_hash(), r.final_hash, "{name}");
        } else {
            let r = ReplayLog::from_json(&text).unwrap();
            assert!(r.verify().unwrap(), "{name}");
        }
        count += 1;
    }
    assert!(count >= 4);
}
