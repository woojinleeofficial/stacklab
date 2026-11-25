//! The off-thread bot (spec 14.4 "AI 탐색이 UI를 막지 않음") plays exactly like the synchronous
//! one: same inputs at the same sub-frames, same final state.

use stacklab_bot::player::{bot_handling, AsyncBotPlayer, BotPlayer};
use stacklab_bot::BotConfig;
use stacklab_engine::versus::Versus;
use stacklab_engine::Ruleset;
use std::sync::Arc;

fn run(async_side: bool, seed: u64, preset: &str) -> (u64, u64, Option<usize>) {
    let rs = Arc::new(Ruleset::tl_s2());
    let mut v = Versus::new(
        rs.clone(),
        [seed, seed ^ 0x77],
        [bot_handling(), bot_handling()],
    );
    let mut a = BotPlayer::new(BotConfig::preset("L4"), &rs, seed * 3);
    let mut b_sync = BotPlayer::new(BotConfig::preset(preset), &rs, seed * 5);
    let mut b_async = AsyncBotPlayer::new(BotConfig::preset(preset), &rs, seed * 5);
    let mut frames = 0;
    let mut stalls = 0u64;
    while v.winner().is_none() && frames < 60 * 90 {
        let ib = if async_side {
            loop {
                match b_async.frame_inputs(&v.players[1]) {
                    Some(x) => break x,
                    None => {
                        stalls += 1;
                        b_async.wait();
                    }
                }
            }
        } else {
            b_sync.frame_inputs(&v.players[1])
        };
        let ia = a.frame_inputs(&v.players[0]);
        v.step([&ia, &ib]);
        let _ = v.players[0].events();
        let _ = v.players[1].events();
        frames += 1;
    }
    let _ = stalls;
    (v.state_hash(), frames, v.winner())
}

#[test]
fn async_bot_matches_the_synchronous_bot() {
    for (seed, preset) in [(1, "L2"), (2, "L6"), (3, "L0")] {
        assert_eq!(
            run(false, seed, preset),
            run(true, seed, preset),
            "seed {seed} {preset}"
        );
    }
}
