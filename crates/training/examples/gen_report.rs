//! STEP 2 report: generation success rate, validation, timing and difficulty per module.
//! cargo run --release -p stacklab-training --example gen_report -- [seeds_per_level]

use stacklab_engine::Ruleset;
use stacklab_training::policy::Policy;
use stacklab_training::problem::Module;
use stacklab_training::service::ProblemService;
use stacklab_training::validator::verify;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

fn main() {
    let n: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);
    let svc = ProblemService::new(Arc::new(Ruleset::tl_s2()), Policy::bundled());
    for m in [Module::A, Module::B] {
        println!("== module {m:?} ==");
        println!("target_D  generated  valid  mean_D  gen_ms(mean/max)  verify_ms(mean)");
        let mut tags: BTreeMap<String, u32> = BTreeMap::new();
        for d in [1100.0, 1300.0, 1500.0, 1700.0, 1900.0] {
            let (mut gen, mut valid, mut sum_d, mut gen_ms, mut max_ms, mut ver_ms) =
                (0, 0, 0.0, 0.0, 0.0f64, 0.0);
            for k in 0..n {
                let t0 = Instant::now();
                let p = svc.generate(m, d, 10_000 + k);
                let ms = t0.elapsed().as_secs_f64() * 1000.0;
                gen_ms += ms;
                max_ms = max_ms.max(ms);
                if let Some(p) = p {
                    gen += 1;
                    let t1 = Instant::now();
                    if verify(&p).valid {
                        valid += 1;
                    }
                    ver_ms += t1.elapsed().as_secs_f64() * 1000.0;
                    sum_d += p.difficulty;
                    for t in &p.subtype_tags {
                        if !t.starts_with("piece:") {
                            *tags.entry(t.clone()).or_default() += 1;
                        }
                    }
                }
            }
            println!(
                "{d:8.0}  {gen:4}/{n:<4}  {valid:5}  {:6.0}  {:7.1}/{:7.1}  {:7.1}",
                sum_d / gen.max(1) as f64,
                gen_ms / n as f64,
                max_ms,
                ver_ms / gen.max(1) as f64
            );
        }
        println!("subtype tags: {tags:?}\n");
    }
}
