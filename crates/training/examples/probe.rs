//! Dev probe: generation rate / validity / time for one module.
//! cargo run --release -p stacklab-training --example probe -- C 1500 50
use stacklab_engine::Ruleset;
use stacklab_training::policy::Policy;
use stacklab_training::problem::Module;
use stacklab_training::service::ProblemService;
use stacklab_training::validator::verify;
use std::sync::Arc;
use std::time::Instant;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let m = match a.get(1).map(|s| s.as_str()).unwrap_or("C") {
        "A" => Module::A,
        "B" => Module::B,
        "C" => Module::C,
        "D" => Module::D,
        "E" => Module::E,
        "F" => Module::F,
        "G" => Module::G,
        _ => Module::H,
    };
    let d: f64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(1500.0);
    let n: u64 = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(30);
    let svc = ProblemService::new(Arc::new(Ruleset::tl_s2()), Policy::bundled());
    let (mut ok, mut valid, mut ms, mut mx, mut sd) = (0, 0, 0.0, 0.0f64, 0.0);
    let mut tags = std::collections::BTreeMap::new();
    for k in 0..n {
        let t = Instant::now();
        let p = svc.generate(m, d, 777 + k);
        let e = t.elapsed().as_secs_f64() * 1000.0;
        ms += e;
        mx = mx.max(e);
        if let Some(p) = p {
            ok += 1;
            let v = verify(&p);
            if v.valid {
                valid += 1
            } else {
                eprintln!("invalid {}: {:?}", p.id, v.reason)
            }
            sd += p.difficulty;
            for t in p.subtype_tags {
                *tags.entry(t).or_insert(0) += 1;
            }
        }
    }
    println!(
        "{m:?} d={d} gen {ok}/{n} valid {valid} meanD {:.0} ms mean {:.1} max {:.1}\n{tags:?}",
        sd / ok.max(1) as f64,
        ms / n as f64,
        mx
    );
}
