//! STEP 4 (spec 14.3 opener items): opener problems by role, loadout invariants, profiles
//! independent of the loadout, abandon rule to free stacking.

use stacklab_engine::rng::Rng;
use stacklab_engine::{MoveRules, Ruleset};
use stacklab_opener::library::Library;
use stacklab_opener::loadout::{LoadoutError, OpenerLoadout, Role};
use stacklab_training::adaptive::{plan_cycle, Cycle, SkillProfile, SlotResult};
use stacklab_training::opener_gen::{generate, OpenerRole};
use stacklab_training::policy::Policy;
use stacklab_training::problem::{Goal, Module};
use stacklab_training::session::{AttemptResult, AttemptSession, Status};
use stacklab_training::validator::verify;
use std::sync::Arc;

fn setup() -> (Arc<Ruleset>, MoveRules, Library) {
    let rs = Arc::new(Ruleset::tl_s2());
    let rules = MoveRules::from_ruleset(&rs);
    (rs, rules, Library::bundled())
}

#[test]
fn loadout_invariants() {
    let (_, _, lib) = setup();
    assert_eq!(
        OpenerLoadout::new(&lib, "TL-S2", "tki", "tki").unwrap_err(),
        LoadoutError::SameOpener
    );
    assert!(matches!(
        OpenerLoadout::new(&lib, "TL-S2", "free-transition", "tki"),
        Err(LoadoutError::PrimaryMustBeMemorized(_))
    ));
    assert!(matches!(
        OpenerLoadout::new(&lib, "TL-S2", "tki", "nope"),
        Err(LoadoutError::Unknown(_))
    ));
    assert!(matches!(
        OpenerLoadout::new(&lib, "TL-S3", "tki", "pco"),
        Err(LoadoutError::Unknown(_))
    ));
    let mut lo = OpenerLoadout::new(&lib, "TL-S2", "tki", "free-transition").unwrap();
    assert!(matches!(
        lo.change(&lib, "TL-S2", "pco", "tki", 7),
        Err(LoadoutError::VersionConflict { .. })
    ));
    lo.change(&lib, "TL-S2", "pco", "tki", 1).unwrap();
    assert_eq!(
        (lo.primary.as_str(), lo.fallback.as_str(), lo.version),
        ("pco", "tki", 2)
    );
    assert_eq!(lo.role_of("tki"), Role::Fallback);
}

#[test]
fn opener_problems_for_every_role_are_valid() {
    let (rs, rules, lib) = setup();
    for (p, f) in [
        ("tki", "free-transition"),
        ("pco", "tki"),
        ("tki", "pc-first"),
    ] {
        let lo = OpenerLoadout::new(&lib, "TL-S2", p, f).unwrap();
        for role in [
            OpenerRole::Primary,
            OpenerRole::Switch,
            OpenerRole::Fallback,
        ] {
            let prob = (0..6u64)
                .find_map(|k| generate(&rs, &rules, &lib, &lo, role, 1500.0, 100 + k, 60))
                .unwrap_or_else(|| panic!("{p}/{f} {role:?}"));
            assert!(
                verify(&prob).valid,
                "{} {:?}",
                prob.id,
                verify(&prob).reason
            );
            let want = if role == OpenerRole::Primary { p } else { f };
            assert!(
                prob.subtype_tags.contains(&format!("opener:{want}")),
                "{:?}",
                prob.subtype_tags
            );
            assert!(prob.subtype_tags.contains(&format!("role:{}", role.tag())));
        }
    }
}

#[test]
fn switch_problem_rejects_building_the_primary() {
    let (rs, rules, lib) = setup();
    let lo = OpenerLoadout::new(&lib, "TL-S2", "tki", "pco").unwrap();
    let prob = (0..10u64)
        .find_map(|k| {
            generate(
                &rs,
                &rules,
                &lib,
                &lo,
                OpenerRole::Switch,
                1500.0,
                7 + k,
                60,
            )
        })
        .unwrap();
    let Goal::OpenerBuild {
        opener, accepted, ..
    } = &prob.goal
    else {
        panic!("memorized fallback -> build goal")
    };
    assert_eq!(opener, "pco");
    assert!(accepted
        .iter()
        .all(|a| !a.branch.starts_with("j-") && !a.branch.starts_with("early")));
    // replaying the witness solves it
    let mut s = AttemptSession::new(&prob, prob.witness.handling).unwrap();
    let mut inputs = prob.witness.inputs.clone();
    inputs.sort_by_key(|e| e.time);
    let mut f = 0u64;
    while *s.status() == Status::Running && f < 3000 {
        let frame: Vec<_> = inputs
            .iter()
            .filter(|e| e.time / 10 == f)
            .copied()
            .collect();
        s.step(&frame);
        f += 1;
    }
    assert_eq!(*s.status(), Status::Success);
}

fn attempt(success: bool) -> AttemptResult {
    AttemptResult {
        success,
        finesse_success: None,
        used_hint: false,
        used_rewind: false,
        used_answer: false,
        valid_for_mmr: true,
        elapsed_frames: 600,
        inputs_per_piece: vec![],
        reason: None,
    }
}

#[test]
fn opener_ratings_survive_loadout_changes() {
    let (_, _, lib) = setup();
    let policy = Policy::bundled();
    let mut profile = SkillProfile::new(&policy);
    profile.loadout = Some(OpenerLoadout::new(&lib, "TL-S2", "tki", "free-transition").unwrap());
    let mut cycle = Cycle::new(vec![Module::E; 12]);
    for i in 0..12 {
        let tags = vec![
            "opener:tki".to_string(),
            "role:primary".into(),
            "branch:j-on-z".into(),
        ];
        cycle.record(SlotResult {
            module: Module::E,
            bucket: "E:tki:primary:h0".into(),
            estimate: 1400.0,
            tags,
            attempt: attempt(i % 3 != 0),
            time_share: 0.5,
        });
    }
    cycle.close(&mut profile, &policy);
    let tki = profile.openers["tki"];
    assert!(tki.rd < 350.0);
    assert_eq!(profile.branch_stats["tki/j-on-z"], (12, 8));
    // change the loadout: the TKI record must stay exactly as it was
    let before = profile.openers.clone();
    let stats = profile.branch_stats.clone();
    let lo = profile.loadout.as_mut().unwrap();
    let v = lo.version;
    lo.change(&lib, "TL-S2", "pco", "pc-first", v).unwrap();
    assert_eq!(profile.openers, before);
    assert_eq!(profile.branch_stats, stats);
    // E appears in cycle plans only with a loadout
    let mut rng = Rng::new(5);
    assert!((0..200).any(|_| plan_cycle(&profile, &policy, &mut rng).contains(&Module::E)));
    profile.loadout = None;
    assert!((0..200).all(|_| !plan_cycle(&profile, &policy, &mut rng).contains(&Module::E)));
}
