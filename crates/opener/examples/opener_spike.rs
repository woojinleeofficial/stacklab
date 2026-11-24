//! STEP 0 spike: first-bag coverage of a TSD setup shape, full-sequence vs visible-info.
//!
//! cargo run --release -p stacklab-opener --example opener_spike -- [sample_step]

use stacklab_engine::{MoveRules, Piece, Ruleset};
use stacklab_opener::{all_bags, full_sequence_feasible, sample_tsd_shape, VisibleCheck};
use std::time::Instant;

fn main() {
    let step: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    let rules = MoveRules::from_ruleset(&Ruleset::tl_s2());
    let shape = sample_tsd_shape();
    println!(
        "shape {}:{:?}finisher TSD ok = {}",
        shape.id,
        shape.target,
        shape.finisher_ok(&rules, 2)
    );
    let bags: Vec<Vec<Piece>> = all_bags().into_iter().step_by(step).collect();
    println!(
        "sample: every {step}th first-bag permutation -> {} bags",
        bags.len()
    );
    for hold in [false, true] {
        let t0 = Instant::now();
        let n = bags
            .iter()
            .filter(|b| full_sequence_feasible(&shape, b, hold, &rules))
            .count();
        println!(
            "fullSequenceFeasible hold={hold}: {n}/{} = {:.2}%  ({:.1}s)",
            bags.len(),
            100.0 * n as f64 / bags.len() as f64,
            t0.elapsed().as_secs_f64()
        );
    }
    for visible in [6usize, 3, 2] {
        let t0 = Instant::now();
        let mut ok = 0;
        let mut nodes = 0;
        for b in &bags {
            let mut vc = VisibleCheck::new(&shape, &rules, visible, true);
            if vc.feasible(&b[..visible.min(7)]) {
                ok += 1;
            }
            nodes += vc.nodes;
        }
        println!(
            "visibleInfoFeasible visible={visible} hold=true: {ok}/{} = {:.2}%  nodes={nodes} ({:.1}s)",
            bags.len(),
            100.0 * ok as f64 / bags.len() as f64,
            t0.elapsed().as_secs_f64()
        );
    }
}
