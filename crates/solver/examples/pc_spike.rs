//! STEP 0 spike: 4-line perfect clear from an empty board (10 pieces, hold on) over random
//! 7-bag queues. Prints success rate, node counts and timing.
//!
//! cargo run --release -p stacklab-solver --example pc_spike -- [samples] [queue_len]

use stacklab_engine::rng::Bag7;
use stacklab_engine::{Board, MoveRules, Piece, Ruleset};
use stacklab_solver::pc::{solve, PcConfig};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let samples: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(100);
    let qlen: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(11);
    let rs = Ruleset::tl_s2();
    let rules = MoveRules::from_ruleset(&rs);
    let cfg = PcConfig {
        height: 4,
        hold: true,
        max_nodes: 2_000_000,
    };
    let (mut found, mut exhausted, mut total_us, mut max_us) = (0, 0, 0u128, 0u128);
    let mut times = Vec::new();
    for seed in 1..=samples {
        let mut bag = Bag7::new(seed * 7919, &rs.queue.bag_base_order);
        let q: Vec<Piece> = (0..qlen).map(|_| bag.next_piece()).collect();
        let out = solve(&Board::default(), &q, None, &rules, &cfg);
        let qs: String = q.iter().map(|p| p.to_char()).collect();
        if out.solution.is_some() {
            found += 1;
        }
        if out.exhausted {
            exhausted += 1;
        }
        total_us += out.micros;
        max_us = max_us.max(out.micros);
        times.push(out.micros);
        if seed <= 5 {
            println!(
                "queue {qs}: {} nodes={} {:.1} ms",
                if out.solution.is_some() {
                    "PC"
                } else {
                    "no PC"
                },
                out.nodes,
                out.micros as f64 / 1000.0
            );
        }
    }
    times.sort_unstable();
    let p50 = times[times.len() / 2];
    let p95 = times[times.len() * 95 / 100];
    println!(
        "samples={samples} queue_len={qlen} found={found} ({:.1}%) budget_hit={exhausted} \
         mean={:.1}ms p50={:.1}ms p95={:.1}ms max={:.1}ms",
        100.0 * found as f64 / samples as f64,
        total_us as f64 / samples as f64 / 1000.0,
        p50 as f64 / 1000.0,
        p95 as f64 / 1000.0,
        max_us as f64 / 1000.0
    );
}
