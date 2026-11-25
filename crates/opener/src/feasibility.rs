//! Feasibility of openers (spec 8.2).
//!
//! * `full_sequence_feasible` — the whole piece order is known (offline verification).
//! * `visible_info_feasible` — a strategy exists using only what a player sees: current piece,
//!   Next window, hold, and bag counting (AND over hidden pieces, OR over choices).
//! * `coverage` — share of first-bag orders (exhaustive 5,040 or sampled) with a 95 % interval.
//! * `decide` — abandon rule: primary if feasible from visible info, else fallback, else free.
//!
//! Memorized branches: every queue piece either goes to its target cell group (reachable at
//! that moment, no line clear) or into hold; the reserved piece (e.g. T for the TSD) must still be
//! playable when the shape is complete.

use crate::library::{Branch, Finisher, Kind, OpenerDefinition};
use stacklab_engine::board::{Board, WIDTH};
use stacklab_engine::rng::Bag7;
use stacklab_engine::{lock, placements, MoveRules, Piece};
use stacklab_solver::pc::{solve as solve_pc, PcConfig};
use std::collections::HashMap;

type VisMemo = HashMap<(Vec<Piece>, usize, Option<Piece>, u32), bool>;
type BeamNode = (Board, usize, Option<Piece>);

pub struct BranchSolver<'a> {
    pub branch: &'a Branch,
    rules: &'a MoveRules,
    reach: HashMap<(u32, usize), bool>,
}

impl<'a> BranchSolver<'a> {
    pub fn new(branch: &'a Branch, rules: &'a MoveRules) -> BranchSolver<'a> {
        BranchSolver {
            branch,
            rules,
            reach: HashMap::new(),
        }
    }

    fn full_mask(&self) -> u32 {
        (1u32 << self.branch.pieces.len()) - 1
    }

    fn board(&self, mask: u32) -> Board {
        let mut b = Board::default();
        for (i, s) in self.branch.pieces.iter().enumerate() {
            if mask >> i & 1 == 1 {
                b.place(s);
            }
        }
        b
    }

    /// Can branch piece `i` be placed (reachable, no line clear) when `mask` is already built?
    pub fn placeable(&mut self, mask: u32, i: usize) -> bool {
        if let Some(&v) = self.reach.get(&(mask, i)) {
            return v;
        }
        let b = self.board(mask);
        let s = self.branch.pieces[i];
        let v = placements(&b, s.piece, self.rules).into_iter().any(|pl| {
            if pl.state.cell_key() != s.cell_key() {
                return false;
            }
            let mut bb = b.clone();
            lock(&mut bb, &pl).lines == 0
        });
        self.reach.insert((mask, i), v);
        v
    }

    /// Branch pieces of type `p` placeable now.
    fn targets(&mut self, mask: u32, p: Piece) -> Vec<usize> {
        (0..self.branch.pieces.len())
            .filter(|&i| mask >> i & 1 == 0 && self.branch.pieces[i].piece == p)
            .filter(|&i| self.placeable(mask, i))
            .collect()
    }

    fn reserve_ok(&self, queue: &[Piece], idx: usize, hold: Option<Piece>) -> bool {
        match self.branch.reserve {
            None => true,
            Some(r) => hold == Some(r) || queue.get(idx) == Some(&r),
        }
    }

    /// Order solver with the whole queue known. Returns the decisions (piece index or hold).
    pub fn full_sequence(&mut self, queue: &[Piece], hold_enabled: bool) -> bool {
        let mut memo = HashMap::new();
        self.full_rec(queue, 0, None, 0, hold_enabled, &mut memo)
    }

    fn full_rec(
        &mut self,
        q: &[Piece],
        idx: usize,
        hold: Option<Piece>,
        mask: u32,
        he: bool,
        memo: &mut HashMap<(usize, Option<Piece>, u32), bool>,
    ) -> bool {
        if mask == self.full_mask() {
            return self.reserve_ok(q, idx, hold);
        }
        if let Some(&v) = memo.get(&(idx, hold, mask)) {
            return v;
        }
        let mut ok = false;
        for (piece, nidx, nhold, _) in stacklab_solver::piece_options(q, idx, hold, he) {
            for i in self.targets(mask, piece) {
                if self.full_rec(q, nidx, nhold, mask | 1 << i, he, memo) {
                    ok = true;
                    break;
                }
            }
            if ok {
                break;
            }
        }
        memo.insert((idx, hold, mask), ok);
        ok
    }

    /// One building order for a fully known queue: (branch piece index, used hold) per placement.
    pub fn plan(&mut self, queue: &[Piece], hold_enabled: bool) -> Option<Vec<(usize, bool)>> {
        let mut memo = HashMap::new();
        if !self.full_rec(queue, 0, None, 0, hold_enabled, &mut memo) {
            return None;
        }
        // walk the memoised success states
        let (mut idx, mut hold, mut mask) = (0usize, None, 0u32);
        let mut out = Vec::new();
        while mask != self.full_mask() {
            let mut moved = false;
            for (piece, nidx, nhold, used) in
                stacklab_solver::piece_options(queue, idx, hold, hold_enabled)
            {
                for i in self.targets(mask, piece) {
                    if self.full_rec(queue, nidx, nhold, mask | 1 << i, hold_enabled, &mut memo) {
                        out.push((i, used));
                        idx = nidx;
                        hold = nhold;
                        mask |= 1 << i;
                        moved = true;
                        break;
                    }
                }
                if moved {
                    break;
                }
            }
            if !moved {
                return None;
            }
        }
        Some(out)
    }

    /// AND-OR search over hidden pieces with `visible` pieces shown (current + Next).
    pub fn visible_info(&mut self, revealed: &[Piece], visible: usize, hold_enabled: bool) -> bool {
        let mut memo = HashMap::new();
        self.vis_rec(
            revealed.to_vec(),
            0,
            None,
            0,
            visible,
            hold_enabled,
            &mut memo,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn vis_rec(
        &mut self,
        revealed: Vec<Piece>,
        idx: usize,
        hold: Option<Piece>,
        mask: u32,
        visible: usize,
        he: bool,
        memo: &mut VisMemo,
    ) -> bool {
        if revealed.len() < idx + visible {
            return bag_candidates(&revealed).into_iter().all(|p| {
                let mut r = revealed.clone();
                r.push(p);
                self.vis_rec(r, idx, hold, mask, visible, he, memo)
            });
        }
        if mask == self.full_mask() {
            return self.reserve_ok(&revealed, idx, hold);
        }
        let key = (revealed.clone(), idx, hold, mask);
        if let Some(&v) = memo.get(&key) {
            return v;
        }
        let mut ok = false;
        'outer: for (piece, nidx, nhold, _) in
            stacklab_solver::piece_options(&revealed, idx, hold, he)
        {
            for i in self.targets(mask, piece) {
                if self.vis_rec(
                    revealed.clone(),
                    nidx,
                    nhold,
                    mask | 1 << i,
                    visible,
                    he,
                    memo,
                ) {
                    ok = true;
                    break 'outer;
                }
            }
        }
        memo.insert(key, ok);
        ok
    }

    /// The finished shape allows the declared finisher (spin with the reserved piece).
    pub fn finisher_ok(&self) -> bool {
        match self.branch.finisher {
            Some(Finisher::Spin { piece, spin, lines }) => {
                let b = self.board(self.full_mask());
                placements(&b, piece, self.rules).iter().any(|pl| {
                    let mut bb = b.clone();
                    pl.spin == spin && lock(&mut bb, pl).lines == lines
                })
            }
            Some(Finisher::PerfectClear { lines }) => {
                // the empty cells under `lines` must be fillable by tetrominoes at all
                let b = self.board(self.full_mask());
                let empty: u32 = (0..lines as usize)
                    .map(|y| WIDTH as u32 - b.rows[y].count_ones())
                    .sum();
                empty.is_multiple_of(4) && b.height() as u32 <= lines
            }
            None => true,
        }
    }
}

/// Pieces that can come next given everything revealed (7-bag counting).
pub fn bag_candidates(revealed: &[Piece]) -> Vec<Piece> {
    let start = revealed.len() / 7 * 7;
    let cur = &revealed[start..];
    stacklab_engine::piece::ALL_PIECES
        .iter()
        .copied()
        .filter(|p| !cur.contains(p))
        .collect()
}

/// Template "pc-first": a 4-line PC from an empty field with these pieces (full order known).
pub fn pc_template_feasible(queue: &[Piece], rules: &MoveRules, lines: u32) -> bool {
    solve_pc(
        &Board::default(),
        queue,
        None,
        rules,
        &PcConfig {
            height: lines as usize,
            hold: true,
            max_nodes: 300_000,
        },
    )
    .solution
    .is_some()
}

/// Template "free-transition": after `pieces` placements (hold allowed), no holes and height
/// within the limit. Beam search on the board.
pub fn stack_template_feasible(
    queue: &[Piece],
    rules: &MoveRules,
    pieces: u32,
    max_height: u32,
    max_holes: u32,
) -> bool {
    let holes = |b: &Board| -> u32 {
        (0..WIDTH)
            .map(|x| {
                let h = b.column_height(x);
                b.rows[..h].iter().filter(|r| *r >> x & 1 == 0).count() as u32
            })
            .sum()
    };
    let score = |b: &Board| -> i32 {
        let hs: Vec<i32> = (0..WIDTH).map(|x| b.column_height(x) as i32).collect();
        let bump: i32 = hs.windows(2).map(|w| (w[0] - w[1]).abs()).sum();
        -(holes(b) as i32) * 100 - bump * 4 - hs.iter().max().copied().unwrap_or(0) * 8
    };
    let mut frontier: Vec<BeamNode> = vec![(Board::default(), 0, None)];
    for _ in 0..pieces {
        let mut next: Vec<(i32, BeamNode)> = Vec::new();
        for (b, idx, hold) in &frontier {
            for (p, nidx, nhold, _) in stacklab_solver::piece_options(queue, *idx, *hold, true) {
                for pl in placements(b, p, rules) {
                    let mut nb = b.clone();
                    lock(&mut nb, &pl);
                    next.push((score(&nb), (nb, nidx, nhold)));
                }
            }
        }
        next.sort_by_key(|n| std::cmp::Reverse(n.0));
        next.truncate(24);
        frontier = next.into_iter().map(|n| n.1).collect();
        if frontier.is_empty() {
            return false;
        }
    }
    frontier
        .iter()
        .any(|(b, _, _)| holes(b) <= max_holes && b.height() as u32 <= max_height)
}

/// Feasibility of a whole opener (any branch) for a queue with full knowledge.
pub fn opener_full_sequence(
    def: &OpenerDefinition,
    queue: &[Piece],
    rules: &MoveRules,
) -> Option<String> {
    match def.kind {
        Kind::Memorized => def
            .branches
            .iter()
            .find(|b| BranchSolver::new(b, rules).full_sequence(queue, true))
            .map(|b| b.id.clone()),
        Kind::Template => {
            let t = def.template.expect("template rule");
            let ok = if let Some(lines) = t.pc {
                pc_template_feasible(
                    &queue[..queue.len().min(t.pieces.unwrap_or(10) as usize + 1)],
                    rules,
                    lines,
                )
            } else {
                stack_template_feasible(
                    queue,
                    rules,
                    t.stack_pieces.unwrap_or(7),
                    t.max_height.unwrap_or(6),
                    t.max_holes.unwrap_or(0),
                )
            };
            ok.then(|| def.id.clone())
        }
    }
}

/// Visible-info feasibility at the start of a game (`revealed` = current + Next).
/// Memorized: per-branch AND-OR (a strategy committing to one branch). Templates: treated as
/// feasible when the full-sequence check passes on the revealed pieces plus bag counting
/// (templates adapt piece by piece, so the strict AND-OR proof is not required).
pub fn opener_visible(
    def: &OpenerDefinition,
    revealed: &[Piece],
    rules: &MoveRules,
) -> Option<String> {
    match def.kind {
        Kind::Memorized => def
            .branches
            .iter()
            .find(|b| {
                BranchSolver::new(b, rules).visible_info(revealed, revealed.len().max(1), true)
            })
            .map(|b| b.id.clone()),
        Kind::Template => {
            let mut q = revealed.to_vec();
            while q.len() < 7 {
                let c = bag_candidates(&q);
                q.push(c[0]);
            }
            let t = def.template.expect("template rule");
            if t.pc.is_some() {
                // the PC needs pieces of the next (hidden) bag: estimate with seeded samples of
                // every continuation class; all samples must be solvable (reported as an estimate)
                let ok = (0..PC_VISIBLE_SAMPLES).all(|k| {
                    let mut bag = Bag7::new(0x5eed_0000 + k, "ZLOSIJT");
                    let mut full = q.clone();
                    full.extend((0..7).map(|_| bag.next_piece()));
                    opener_full_sequence(def, &full, rules).is_some()
                });
                return ok.then(|| def.id.clone());
            }
            opener_full_sequence(def, &q, rules)
        }
    }
}

/// Hidden next-bag samples used to estimate visible-info feasibility of the PC template.
pub const PC_VISIBLE_SAMPLES: u64 = 4;

#[derive(Debug, Clone, PartialEq)]
pub struct Coverage {
    /// Human-readable verification scope (shown next to any "100 %" claim).
    pub scope: String,
    pub exhaustive: bool,
    pub total: u32,
    pub full: u32,
    pub visible: u32,
    /// 95 % Wilson interval of the visible-info rate.
    pub ci95: (f64, f64),
    /// Visible-info result per evaluated bag, in `all_first_bags()` order (stepped).
    pub per_bag: Vec<bool>,
}

impl Coverage {
    /// Spec 8.2: "100 %" only when every case in the stated scope was verified.
    pub fn is_guaranteed(&self) -> bool {
        self.exhaustive && self.visible == self.total
    }
    pub fn rate(&self) -> f64 {
        self.visible as f64 / self.total.max(1) as f64
    }
}

pub fn wilson(k: u32, n: u32) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    let z = 1.96;
    let p = k as f64 / n as f64;
    let n = n as f64;
    let d = 1.0 + z * z / n;
    let c = p + z * z / (2.0 * n);
    let r = z * ((p * (1.0 - p) + z * z / (4.0 * n)) / n).sqrt();
    (((c - r) / d).max(0.0), ((c + r) / d).min(1.0))
}

/// Every permutation of one bag (5,040).
pub fn all_first_bags() -> Vec<Vec<Piece>> {
    fn perm(rest: &mut Vec<Piece>, cur: &mut Vec<Piece>, out: &mut Vec<Vec<Piece>>) {
        if rest.is_empty() {
            out.push(cur.clone());
            return;
        }
        for i in 0..rest.len() {
            let p = rest.remove(i);
            cur.push(p);
            perm(rest, cur, out);
            cur.pop();
            rest.insert(i, p);
        }
    }
    let mut out = Vec::new();
    perm(
        &mut stacklab_engine::piece::ALL_PIECES.to_vec(),
        &mut Vec::new(),
        &mut out,
    );
    out
}

/// Coverage over first bags. `step` = 1 is exhaustive (5,040); larger steps sample.
/// The second bag is a fixed seeded bag (only matters when holds push past bag 1).
pub fn coverage(
    def: &OpenerDefinition,
    rules: &MoveRules,
    step: usize,
    visible: usize,
) -> Coverage {
    let bags: Vec<Vec<Piece>> = all_first_bags().into_iter().step_by(step.max(1)).collect();
    let mut second = Bag7::new(0x0bea_c0de, "ZLOSIJT");
    let tail: Vec<Piece> = (0..7).map(|_| second.next_piece()).collect();
    let (mut full, mut vis) = (0, 0);
    let mut per_bag = Vec::with_capacity(bags.len());
    for b in &bags {
        let mut q = b.clone();
        q.extend(tail.iter().copied());
        let mut ok = false;
        if opener_full_sequence(def, &q, rules).is_some() {
            full += 1;
            if opener_visible(def, &b[..visible.min(7)], rules).is_some() {
                vis += 1;
                ok = true;
            }
        }
        per_bag.push(ok);
    }
    let total = bags.len() as u32;
    let estimate = def.template.is_some_and(|t| t.pc.is_some());
    Coverage {
        scope: format!(
            "{} 첫 Bag {}가지{}, Hold 사용, 공개 정보 = 현재+Next {}{}",
            if step <= 1 { "전수:" } else { "표본:" },
            total,
            if step <= 1 {
                ""
            } else {
                " (5,040가지 중 일정 간격)"
            },
            visible.saturating_sub(1),
            if estimate {
                " (가시정보는 다음 Bag 표본 추정)"
            } else {
                ""
            }
        ),
        exhaustive: step <= 1,
        per_bag,
        total,
        full,
        visible: vis,
        ci95: wilson(vis, total),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Primary(String),
    Fallback(String),
    FreeStacking,
}

/// Spec 8.1 execution order with the abandon rule: primary when it is feasible from the visible
/// information, otherwise the fallback, otherwise free stacking.
pub fn decide(
    primary: &OpenerDefinition,
    fallback: &OpenerDefinition,
    revealed: &[Piece],
    rules: &MoveRules,
) -> Decision {
    if let Some(b) = opener_visible(primary, revealed, rules) {
        return Decision::Primary(b);
    }
    if let Some(b) = opener_visible(fallback, revealed, rules) {
        return Decision::Fallback(b);
    }
    Decision::FreeStacking
}

/// The finisher of a branch is available (used by tests and the library report).
pub fn branch_valid(b: &Branch, rules: &MoveRules) -> Result<(), String> {
    let s = BranchSolver::new(b, rules);
    if !s.finisher_ok() {
        return Err(format!(
            "{}: finisher not available on the finished shape",
            b.id
        ));
    }
    // there must be at least one order that builds the shape
    let mut solver = BranchSolver::new(b, rules);
    let ok = all_first_bags().iter().step_by(7).any(|bag| {
        let mut q = bag.clone();
        q.extend_from_slice(bag);
        solver.full_sequence(&q, true)
    });
    if !ok {
        return Err(format!("{}: no piece order builds the shape", b.id));
    }
    Ok(())
}
