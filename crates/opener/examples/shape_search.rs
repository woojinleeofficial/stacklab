//! STEP 0 helper: enumerate left-side TSD setup shapes (6 pieces, T reserved) and rank them by
//! sampled first-bag coverage. Used to pick the spike shape; not part of the product.

use stacklab_engine::board::Board;
use stacklab_engine::piece::ALL_PIECES;
use stacklab_engine::{MoveRules, Piece, Ruleset};
use stacklab_opener::{all_bags, full_sequence_feasible, OpenerShape};

/// Order-free exact cover with one of each piece in `pieces` (no reachability).
fn tileable(rem: &Board, pieces: &mut Vec<Piece>) -> bool {
    // first empty-target cell in scan order
    let mut first = None;
    'f: for y in 0..4i8 {
        for x in 0..10i8 {
            if rem.rows[y as usize] >> x & 1 == 1 {
                first = Some((x, y));
                break 'f;
            }
        }
    }
    let Some((fx, fy)) = first else { return true };
    for i in 0..pieces.len() {
        let p = pieces[i];
        for r in 0..4 {
            let rot = stacklab_engine::Rot::from_index(r);
            for &(cx, cy) in p.cells(rot).iter() {
                let (ox, oy) = (fx - cx, fy - cy);
                let cells: Vec<(i8, i8)> = p
                    .cells(rot)
                    .iter()
                    .map(|&(a, b)| (a + ox, b + oy))
                    .collect();
                if cells.iter().all(|&(x, y)| {
                    (0..10).contains(&x)
                        && (0..4).contains(&y)
                        && rem.rows[y as usize] >> x & 1 == 1
                }) {
                    let mut nb = rem.clone();
                    for &(x, y) in &cells {
                        nb.rows[y as usize] &= !(1 << x);
                    }
                    pieces.remove(i);
                    let ok = tileable(&nb, pieces);
                    pieces.insert(i, p);
                    if ok {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn main() {
    let rules = MoveRules::from_ruleset(&Ruleset::tl_s2());
    let non_t: Vec<Piece> = ALL_PIECES
        .iter()
        .copied()
        .filter(|&p| p != Piece::T)
        .collect();
    let y2_cols: Vec<i8> = vec![0, 1, 3, 4, 5, 6, 7, 8, 9];
    let y3_cols: Vec<i8> = vec![0, 1, 4, 5, 6, 7, 8, 9];
    let bags = all_bags();
    let sample: Vec<&Vec<Piece>> = bags.iter().step_by(53).collect();
    let mut results = Vec::new();
    for m2 in 0u32..(1 << y2_cols.len()) {
        for m3 in 0u32..(1 << y3_cols.len()) {
            if m2.count_ones() + m3.count_ones() != 8 {
                continue;
            }
            let mut b = Board::from_ascii(&["#...######", "##.#######"]);
            for (i, &x) in y2_cols.iter().enumerate() {
                if m2 >> i & 1 == 1 {
                    b.rows[2] |= 1 << x;
                }
            }
            let mut supported = true;
            for (i, &x) in y3_cols.iter().enumerate() {
                if m3 >> i & 1 == 1 {
                    b.rows[3] |= 1 << x;
                    supported &= b.rows[2] >> x & 1 == 1;
                }
            }
            if !supported {
                continue;
            }
            let shape = OpenerShape {
                id: String::new(),
                target: b.clone(),
                reserve: Some(Piece::T),
            };
            if !shape.finisher_ok(&rules, 2) {
                continue;
            }
            if !tileable(&b, &mut non_t.clone()) {
                continue;
            }
            let cov = sample
                .iter()
                .filter(|q| full_sequence_feasible(&shape, q, true, &rules))
                .count();
            results.push((cov, b));
        }
    }
    results.sort_by_key(|r| std::cmp::Reverse(r.0));
    println!("candidates={} sample={}", results.len(), sample.len());
    for (cov, b) in results.iter().take(5) {
        println!("coverage {cov}/{}:{:?}", sample.len(), b);
    }
}
