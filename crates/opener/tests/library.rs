use stacklab_engine::piece::parse_queue;
use stacklab_engine::{MoveRules, Ruleset};
use stacklab_opener::feasibility::{
    all_first_bags, branch_valid, coverage, decide, opener_full_sequence, opener_visible,
    BranchSolver, Decision,
};
use stacklab_opener::library::{Kind, Library};

fn rules() -> MoveRules {
    MoveRules::from_ruleset(&Ruleset::tl_s2())
}

#[test]
fn bundled_library_is_valid_and_engine_verified() {
    let lib = Library::bundled();
    let rules = rules();
    for id in ["tki", "pco", "pc-first", "free-transition"] {
        let def = lib.get(id).unwrap_or_else(|| panic!("{id}"));
        assert_eq!(def.ruleset, "TL-S2");
        for b in &def.branches {
            branch_valid(b, &rules).unwrap();
        }
    }
    let tki = lib.get("tki").unwrap();
    assert_eq!(tki.branches.len(), 8, "4 branches + mirrors");
    assert!(tki
        .branches
        .iter()
        .all(|b| b.pieces.len() == 6 && b.reserve.is_some()));
    let pco = lib.get("pco").unwrap();
    assert_eq!(pco.branch("with-i").unwrap().pieces.len(), 7);
    assert_eq!(pco.branch("no-i").unwrap().pieces.len(), 6);
    assert_eq!(lib.get("free-transition").unwrap().kind, Kind::Template);
}

#[test]
fn tki_example_orders() {
    let lib = Library::bundled();
    let rules = rules();
    let tki = lib.get("tki").unwrap();
    assert!(opener_full_sequence(tki, &parse_queue("ILOZSJTILOZSJT"), &rules).is_some());
    let base = tki.branch("j-right-vertical").unwrap();
    assert!(
        BranchSolver::new(base, &rules).finisher_ok(),
        "TSD available"
    );
}

#[test]
fn hundred_percent_claim_needs_exhaustive_scope() {
    let lib = Library::bundled();
    let rules = rules();
    let tki = lib.get("tki").unwrap();
    let sampled = coverage(tki, &rules, 251, 6);
    assert!(!sampled.is_guaranteed(), "a sample can never claim 100 %");
    assert!(sampled.scope.contains("표본"));
    assert!(sampled.visible <= sampled.full && sampled.full <= sampled.total);
    assert!(sampled.ci95.0 <= sampled.rate() && sampled.rate() <= sampled.ci95.1);
}

#[test]
fn abandon_rule_primary_then_fallback_then_free() {
    let lib = Library::bundled();
    let rules = rules();
    let tki = lib.get("tki").unwrap();
    let free = lib.get("free-transition").unwrap();
    let bags = all_first_bags();
    let ok = bags
        .iter()
        .find(|b| opener_visible(tki, &b[..6], &rules).is_some())
        .expect("some TKI bag");
    let bad = bags
        .iter()
        .find(|b| opener_visible(tki, &b[..6], &rules).is_none())
        .expect("some non-TKI bag");
    assert!(matches!(
        decide(tki, free, &ok[..6], &rules),
        Decision::Primary(_)
    ));
    assert_eq!(
        decide(tki, free, &bad[..6], &rules),
        Decision::Fallback("free-transition".into())
    );
}

#[test]
fn precomputed_coverage_matches_library_versions() {
    let lib = Library::bundled();
    let table = stacklab_opener::library::coverage_table();
    for def in &lib.openers {
        let c = table
            .get(&def.id)
            .unwrap_or_else(|| panic!("coverage for {}", def.id));
        assert_eq!(
            c.version, def.version,
            "{}: regenerate data/openers/coverage.toml",
            def.id
        );
        assert!(c.visible <= c.full && c.full <= c.total);
        assert!(c.ci_low <= c.rate() && c.rate() <= c.ci_high + 1e-9);
        if def.kind == Kind::Template {
            assert!(
                !c.exhaustive && !c.is_guaranteed(),
                "sampled scopes never claim 100 %"
            );
        }
    }
}

#[test]
fn neither_possible_means_free_stacking() {
    let lib = Library::bundled();
    let rules = rules();
    let tki = lib.get("tki").unwrap();
    let pco = lib.get("pco").unwrap();
    let both_bad = all_first_bags()
        .into_iter()
        .step_by(3)
        .find(|b| {
            opener_visible(tki, &b[..6], &rules).is_none()
                && opener_visible(pco, &b[..6], &rules).is_none()
        })
        .expect("some bag allows neither");
    assert_eq!(
        decide(tki, pco, &both_bad[..6], &rules),
        Decision::FreeStacking
    );
}

#[test]
fn per_bag_bits_roundtrip_and_match_counts() {
    use stacklab_opener::library::{decode_bits, encode_bits};
    let v: Vec<bool> = (0..5040).map(|i| i % 7 == 0 || i % 11 == 3).collect();
    assert_eq!(decode_bits(&encode_bits(&v), v.len()), v);
    let table = stacklab_opener::library::coverage_table();
    for id in ["tki", "pco"] {
        let c = &table[id];
        let bits = c.per_bag().expect("exhaustive openers store per-bag bits");
        assert_eq!(bits.iter().filter(|b| **b).count() as u32, c.visible);
    }
}
