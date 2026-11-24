//! STEP 0 spike: do the strength knobs produce a monotonic ladder?
//! Plays adjacent presets against each other and prints win rates.
//!
//! cargo run --release -p stacklab-bot --example bot_strength_spike -- [games_per_pair]

use stacklab_bot::versus::play_round;
use stacklab_bot::BotConfig;
use stacklab_engine::Ruleset;
use std::time::Instant;

fn main() {
    let games: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    let rs = Ruleset::tl_s2();
    let max_frames = 60 * 60 * 6; // 6 minutes safety cap (spike only)
    println!("preset  pps  depth beam mistake attackW");
    for l in 0..5 {
        let c = BotConfig::preset(l);
        println!(
            "{:6} {:4.1} {:5} {:4} {:6.1}% {:7}",
            c.name,
            c.pps_milli as f64 / 1000.0,
            c.depth,
            c.beam,
            c.mistake_pm as f64 / 10.0,
            c.attack_weight
        );
    }
    for l in 0..4u32 {
        let t0 = Instant::now();
        let (mut wins_hi, mut capped, mut frames) = (0, 0, 0u64);
        for g in 0..games {
            let (lo, hi) = (BotConfig::preset(l), BotConfig::preset(l + 1));
            // alternate sides
            let r = if g % 2 == 0 {
                let r = play_round(lo, hi, &rs, 1000 + g, max_frames);
                (r.winner == 1, r)
            } else {
                let r = play_round(hi, lo, &rs, 1000 + g, max_frames);
                (r.winner == 0, r)
            };
            if r.0 {
                wins_hi += 1;
            }
            if r.1.capped {
                capped += 1;
            }
            frames += r.1.frames;
        }
        let p = wins_hi as f64 / games as f64;
        // logistic rating gap implied by the score (Elo scale, for a rough ladder picture)
        let pc = p.clamp(0.5 / games as f64, 1.0 - 0.5 / games as f64);
        let gap = -400.0 * (1.0 / pc - 1.0).log10();
        println!(
            "L{} vs L{}: stronger wins {}/{} ({:.0}%)  implied gap {:+.0}  avg round {:.0}s  capped {}  ({:.1}s)",
            l + 1, l, wins_hi, games, 100.0 * p, gap,
            frames as f64 / games as f64 / 60.0, capped, t0.elapsed().as_secs_f64()
        );
    }
}
