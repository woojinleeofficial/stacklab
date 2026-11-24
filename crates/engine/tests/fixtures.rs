//! Spec 14.1 fixtures: fixed inputs -> expected engine events.

mod common;

use common::*;
use stacklab_engine::events::{EventKind, MatchEndReason, TopOutReason};
use stacklab_engine::game::{Game, Handling, InputEvent, Key, Mode};
use stacklab_engine::{Piece, Rot, Spin};

#[test]
fn spawn_position_and_event() {
    let h = Harness::new(init(&[], "TIOSZJL"));
    let a = h.g.active_piece().unwrap();
    assert_eq!((a.piece, a.rot, a.x, a.y), (Piece::T, Rot::N, 4, 20));
    assert!(h.has(|k| matches!(k, EventKind::PieceSpawn { piece: Piece::T })));
    assert_eq!(h.g.next_pieces().len(), 5);
    assert_eq!(
        &h.g.next_pieces()[..4],
        &[Piece::I, Piece::O, Piece::S, Piece::Z]
    );
}

#[test]
fn hard_drop_and_wall_collision() {
    let mut h = Harness::new(init(&[], "TIOSZJL"));
    let mut inputs = Vec::new();
    for i in 0..6 {
        inputs.push(InputEvent {
            time: i * 2,
            key: Key::Left,
            down: true,
        });
        inputs.push(InputEvent {
            time: i * 2 + 1,
            key: Key::Left,
            down: false,
        });
    }
    inputs.push(InputEvent {
        time: 20,
        key: Key::HardDrop,
        down: true,
    });
    inputs.push(InputEvent {
        time: 21,
        key: Key::HardDrop,
        down: false,
    });
    h.run_inputs(&inputs, 30);
    // T stops at the wall (centre x = 1) and lands on the floor
    assert!(h.has(|k| matches!(
        k,
        EventKind::PieceLock {
            piece: Piece::T,
            x: 1,
            y: 0,
            ..
        }
    )));
    assert_eq!(h.g.board().rows[0], 0b111);
    assert_eq!(h.g.board().rows[1], 0b010);
}

#[test]
fn das_arr0_reaches_wall() {
    let rs = rs();
    let handling = Handling {
        das: 100,
        arr: 0,
        sdf: 0,
        dcd: 0,
    };
    let mut g = Game::with_initial(rs, 1, handling, Mode::Free, Some(&init(&[], "OIOSZJL")));
    let press = [InputEvent {
        time: 0,
        key: Key::Right,
        down: true,
    }];
    g.step(&press);
    // after 1 tap: x 5. DAS charges exactly 100 sub-frames (10 frames) after the press
    for _ in 0..9 {
        g.step(&[]);
    }
    assert_eq!(g.active_piece().unwrap().x, 5);
    g.step(&[]);
    assert_eq!(
        g.active_piece().unwrap().x,
        8,
        "O teleports to the right wall"
    );
}

#[test]
fn lock_delay_then_lock() {
    let mut h = Harness::new(init(&[], "OIOSZJL"));
    let inputs = [
        InputEvent {
            time: 0,
            key: Key::SoftDrop,
            down: true,
        },
        InputEvent {
            time: 1,
            key: Key::SoftDrop,
            down: false,
        },
    ];
    h.run_inputs(&inputs, 2);
    assert_eq!(h.g.active_piece().unwrap().y, 0);
    h.frames(28);
    assert_eq!(h.g.stats().pieces, 0, "not locked before 30 frames");
    h.frames(2);
    assert_eq!(h.g.stats().pieces, 1, "locked after lock delay");
}

#[test]
fn t_180_off_the_floor_through_inputs() {
    let mut h = Harness::new(init(&[], "TIOSZJL"));
    let inputs = [
        InputEvent {
            time: 0,
            key: Key::SoftDrop,
            down: true,
        },
        InputEvent {
            time: 1,
            key: Key::SoftDrop,
            down: false,
        },
        InputEvent {
            time: 2,
            key: Key::Flip,
            down: true,
        },
        InputEvent {
            time: 3,
            key: Key::Flip,
            down: false,
        },
    ];
    h.run_inputs(&inputs, 4);
    let a = h.g.active_piece().unwrap();
    assert_eq!((a.rot, a.y), (Rot::S, 1));
}

#[test]
fn tsd_spin_and_attack() {
    let mut h = Harness::new(init(&["##........", "#...######", "##.#######"], "TIOSZJL"));
    h.place_at(Piece::T, Rot::S, 2, 1, Spin::Full);
    assert!(h.has(|k| matches!(
        k,
        EventKind::Spin {
            piece: Piece::T,
            spin: Spin::Full,
            lines: 2
        }
    )));
    assert!(h.has(|k| matches!(k, EventKind::LineClear { lines: 2 })));
    assert_eq!(h.sent(), vec![4]);
}

#[test]
fn all_mini_plus_s_spin() {
    // S has to rotate into an immobile pocket: counted as a mini spin
    let mut h = Harness::new(init(&["##..######", "#..#######", "##..######"], "SIOTZJL"));
    let target = h.find(Piece::S, |p| p.spin == Spin::Mini);
    h.play(&target, false);
    assert!(h.has(|k| matches!(
        k,
        EventKind::Spin {
            piece: Piece::S,
            spin: Spin::Mini,
            ..
        }
    )));
}

#[test]
fn quad_perfect_clear_sends_9() {
    // user-confirmed: PC adds 5 to the clear's own attack (Quad PC = 4 + 5)
    let rows = ["#########.", "#########.", "#########.", "#########."];
    let mut h = Harness::new(init(&rows, "IOTSZJL"));
    h.place_at(Piece::I, Rot::E, 8, 2, Spin::None);
    assert!(h.has(|k| matches!(k, EventKind::PerfectClear)));
    assert_eq!(h.sent().iter().sum::<u32>(), 9);
    assert!(h.g.board().is_empty());
}

#[test]
fn tsd_perfect_clear_sends_9() {
    let rows = ["#...######", "##.#######"];
    let mut h = Harness::new(init(&rows, "TIOSZJL"));
    // T pointing down dropped into the slot: 3 corners (floor counts) -> TSD + PC
    let p = h.find(Piece::T, |p| {
        p.state.rot == Rot::S && p.state.x == 2 && p.state.y == 1
    });
    h.play(&p, false);
    assert!(h.has(|k| matches!(k, EventKind::PerfectClear)));
    let spin_full = h.has(|k| {
        matches!(
            k,
            EventKind::Spin {
                spin: Spin::Full,
                lines: 2,
                ..
            }
        )
    });
    let total: u32 = h.sent().iter().sum();
    if spin_full {
        assert_eq!(total, 9, "TSD PC = 4 + 5");
    } else {
        assert_eq!(total, 1 + 5, "plain double PC = 1 + 5");
    }
}

#[test]
fn b2b_charge_and_surge_release() {
    let rows = [
        "#########.",
        "#########.",
        "#########.",
        "#########.",
        "#########.",
        "#########.",
    ];
    let mut i = init(&rows, "IIOTSZJL");
    i.b2b = 7;
    let mut h = Harness::new(i);
    h.place_at(Piece::I, Rot::E, 8, 2, Spin::None);
    // quad at B2B x8: 4 + 1 bonus
    assert_eq!(h.sent(), vec![5]);
    assert!(h.has(|k| matches!(k, EventKind::B2bChange { from: 7, to: 8 })));
    assert!(h.has(|k| matches!(k, EventKind::SurgeCharge { amount: 8 })));
    // remaining two rows: a vertical I clears 2 lines (double) and breaks B2B -> surge 8 = 3/3/2
    h.events.clear();
    h.place_at(Piece::I, Rot::E, 8, 2, Spin::None);
    assert!(h.has(|k| matches!(k, EventKind::SurgeRelease { amount: 8, .. })));
    assert_eq!(
        h.sent(),
        vec![1, 3, 3, 2],
        "double (combo 1) 1, then surge 8 split 3/3/2"
    );
}

#[test]
fn combo_multiplier_in_game() {
    let rows = ["#########.", "#########."];
    let mut i = init(&rows, "IOTSZJL");
    i.combo = 3; // the next clear is combo 4
    let mut h = Harness::new(i);
    h.place_at(Piece::I, Rot::E, 8, 2, Spin::None);
    // double (1) at combo 4 -> floor(1 * 2.0) = 2
    assert!(h.has(|k| matches!(k, EventKind::ComboChange { from: 3, to: 4 })));
    assert_eq!(h.sent(), vec![2]);
}

#[test]
fn opener_phase_doubles_cancel() {
    let rows = ["##.#######", "##.#######", "###.######"];
    let mut h = Harness::new(init(&rows, "IOTSZJL"));
    h.g.receive_garbage(4);
    // vertical I in column 2: clears 2 lines -> double = 1 line, cancels 2 during opener phase
    h.place_at(Piece::I, Rot::E, 1, 3, Spin::None);
    assert!(h.has(|k| matches!(k, EventKind::AttackCancelled { lines: 2 })));
    assert_eq!(h.g.pending_garbage(), 2);
    assert!(h.sent().is_empty());
}

#[test]
fn garbage_waits_travel_time_and_respects_cap() {
    let mut h = Harness::new(init(&[], "OOOOOOOOOOOOOO"));
    h.g.receive_garbage(10);
    // drop one O immediately: garbage not active yet (20 frames travel)
    h.place_at(Piece::O, Rot::N, 0, 0, Spin::None);
    assert_eq!(h.g.stats().received, 0);
    h.frames(21);
    h.place_at(Piece::O, Rot::N, 2, 0, Spin::None);
    assert_eq!(h.g.stats().received, 8, "cap 8 per piece");
    assert_eq!(h.g.pending_garbage(), 2);
    assert!(h.has(|k| matches!(k, EventKind::GarbageReceived { lines: 8, .. })));
    // every garbage row has exactly one hole
    for y in 0..8 {
        assert_eq!(h.g.board().rows[y].count_ones(), 9);
    }
}

#[test]
fn block_out_tops_out() {
    let rows = vec!["###.######"; 20];
    let mut h = Harness::new(init(&rows, "OOTSZJL"));
    // O lands on the stack at the spawn rows; next O cannot spawn
    let inputs = [
        InputEvent {
            time: 0,
            key: Key::HardDrop,
            down: true,
        },
        InputEvent {
            time: 1,
            key: Key::HardDrop,
            down: false,
        },
    ];
    h.run_inputs(&inputs, 3);
    h.frames(1);
    assert!(h.has(|k| matches!(
        k,
        EventKind::TopOut {
            reason: TopOutReason::BlockOut
        }
    )));
    assert!(h.g.is_over());
}

#[test]
fn clutch_clear_survives_block_out() {
    // tower in columns 3..5 up to row 19; row 20 is full except the O spawn columns 4,5
    let mut rows: Vec<String> = vec!["####..####".to_string()];
    for _ in 0..20 {
        rows.push("...###....".to_string());
    }
    let refs: Vec<&str> = rows.iter().map(|s| s.as_str()).collect();
    let mut h = Harness::new(init(&refs, "OTSZJL"));
    let inputs = [
        InputEvent {
            time: 0,
            key: Key::HardDrop,
            down: true,
        },
        InputEvent {
            time: 1,
            key: Key::HardDrop,
            down: false,
        },
    ];
    h.run_inputs(&inputs, 3);
    assert!(h.has(|k| matches!(k, EventKind::LineClear { lines: 1 })));
    assert!(h.has(|k| matches!(k, EventKind::ClutchClear { streak: 1 })));
    assert!(!h.g.is_over());
}

#[test]
fn hold_once_per_piece() {
    let mut h = Harness::new(init(&[], "TIOSZJL"));
    let inputs = [
        InputEvent {
            time: 0,
            key: Key::Hold,
            down: true,
        },
        InputEvent {
            time: 1,
            key: Key::Hold,
            down: false,
        },
        InputEvent {
            time: 2,
            key: Key::Hold,
            down: true,
        },
        InputEvent {
            time: 3,
            key: Key::Hold,
            down: false,
        },
    ];
    h.run_inputs(&inputs, 4);
    assert_eq!(h.g.hold_piece(), Some(Piece::T));
    assert_eq!(
        h.g.active_piece().unwrap().piece,
        Piece::I,
        "second hold ignored"
    );
    let holds = h
        .events
        .iter()
        .filter(|e| matches!(e.kind, EventKind::HoldUsed { .. }))
        .count();
    assert_eq!(holds, 1);
}

#[test]
fn sprint_mode_finishes_with_goal() {
    let rows = ["#########.", "#########."];
    let mut h = Harness::with(init(&rows, "IOTSZJL"), Mode::Sprint { lines: 2 });
    h.place_at(Piece::I, Rot::E, 8, 2, Spin::None);
    assert!(h.g.is_over());
    assert!(h.has(|k| matches!(
        k,
        EventKind::MatchEnd {
            reason: MatchEndReason::GoalReached
        }
    )));
    assert!(h.g.stats().finish_tick.is_some());
}

#[test]
fn events_carry_ruleset_version() {
    let h = Harness::new(init(&[], "TIOSZJL"));
    assert!(h.events.iter().all(|e| &*e.ruleset_version == "TL-S2"));
}

#[test]
fn opener_phase_end_after_14_pieces() {
    let mut h = Harness::new(init(&[], "OOOOOOOOOOOOOOOO"));
    for i in 0..14 {
        let x = (i % 5) as i8 * 2;
        let p = h.find(Piece::O, |p| p.state.x == x);
        h.play(&p, false);
    }
    let ends: Vec<u32> = h
        .events
        .iter()
        .filter(|e| matches!(e.kind, EventKind::OpenerPhaseEnd))
        .map(|e| e.piece_index)
        .collect();
    assert_eq!(ends, vec![14]);
}

#[test]
fn arr_repeats_every_arr_subframes() {
    let rs = rs();
    let handling = Handling {
        das: 50,
        arr: 20,
        sdf: 0,
        dcd: 0,
    };
    let mut g = Game::with_initial(rs, 1, handling, Mode::Free, Some(&init(&[], "TIOSZJL")));
    g.step(&[InputEvent {
        time: 0,
        key: Key::Left,
        down: true,
    }]);
    // tap: x 3. DAS (50 sub) charges during frame 5 -> x 2, then every 20 sub-frames
    for _ in 0..4 {
        g.step(&[]);
    }
    assert_eq!(g.active_piece().unwrap().x, 3);
    g.step(&[]);
    assert_eq!(g.active_piece().unwrap().x, 2);
    g.step(&[]);
    g.step(&[]);
    assert_eq!(g.active_piece().unwrap().x, 1);
}

#[test]
fn dcd_pauses_auto_shift_after_rotation() {
    let rs = rs();
    let handling = Handling {
        das: 30,
        arr: 0,
        sdf: 0,
        dcd: 50,
    };
    let mut g = Game::with_initial(rs, 1, handling, Mode::Free, Some(&init(&[], "TIOSZJL")));
    // spawn also cuts DAS for 50 sub-frames: holding Right does nothing for 5 frames + DAS
    g.step(&[InputEvent {
        time: 0,
        key: Key::Right,
        down: true,
    }]);
    assert_eq!(g.active_piece().unwrap().x, 5, "initial tap moves once");
    for _ in 0..7 {
        g.step(&[]);
    }
    assert_eq!(
        g.active_piece().unwrap().x,
        5,
        "DCD (5F) + DAS (3F) not yet elapsed"
    );
    g.step(&[]);
    assert_eq!(g.active_piece().unwrap().x, 8, "then ARR 0 teleports");
}
