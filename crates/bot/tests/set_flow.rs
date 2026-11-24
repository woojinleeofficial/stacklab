//! A whole set's match part with a bot playing the user's side on the real engine:
//! mini 1, mini 2, BO3 evaluation -> match rating update (spec 7.1, 7.2, 7.4).

use stacklab_bot::league::play_round;
use stacklab_bot::{calibration, BotConfig};
use stacklab_engine::Ruleset;
use stacklab_training::policy::Policy;
use stacklab_training::set::{
    select_opponent, MatchProfile, MiniStatus, Observations, Phase, RoundRecord, SetState,
};
use std::sync::Arc;

#[test]
fn mini_mini_bo3_with_engine_rounds() {
    let rs = Arc::new(Ruleset::tl_s2());
    let policy = Policy::bundled();
    let cal = calibration();
    let bots: Vec<(String, f64, f64)> = cal
        .bot
        .iter()
        .map(|b| (b.id.clone(), b.rating, b.rd))
        .collect();
    let mut m = MatchProfile::new(&policy);
    let user = BotConfig::preset("L3");
    let mut set = SetState::new_first();
    set.diagnosis_done().unwrap();
    let mut seed = 40;
    let mut play = |m: &MatchProfile| {
        let (id, p, _) = select_opponent(m, &bots, (0.4, 0.6)).unwrap();
        assert!((0.0..=1.0).contains(&p));
        seed += 1;
        let o = play_round(&user, &BotConfig::preset(&id), &rs, seed);
        let b = cal.bot.iter().find(|b| b.id == id).unwrap();
        (o.winner == 0, id, b.rating, b.rd)
    };
    for k in 1..=2u8 {
        set.cycle_done().unwrap();
        assert_eq!(set.phase, Phase::Mini(k));
        let (won, _, _, _) = play(&m);
        set.mini_result(won).unwrap();
        assert!(matches!(
            set.minis[k as usize - 1],
            MiniStatus::Won | MiniStatus::Lost
        ));
    }
    let before = m.rating;
    set.cycle_done().unwrap();
    assert_eq!(set.phase, Phase::Evaluation);
    let mut rounds = 0;
    loop {
        let (won, id, r, rd) = play(&m);
        rounds += 1;
        let done = set
            .eval_round(RoundRecord {
                won,
                void: false,
                bot: id,
                bot_rating: r,
                bot_rd: rd,
                calibration_version: cal.calibration_version,
                observations: Observations::default(),
            })
            .unwrap();
        if done {
            break;
        }
    }
    assert!((2..=3).contains(&rounds), "BO3 ends after 2 or 3 rounds");
    assert!(set.evaluation_completed());
    m.rate_evaluation(&set.eval_rounds, &policy);
    assert_ne!(
        m.rating, before,
        "completed evaluation updates the match rating"
    );
    assert!(m.rating.rd < before.rd);
}
