//! Deterministic integer PRNG (Park–Miller minimal standard) and 7-bag randomizer.

use crate::piece::Piece;

const M: u64 = 2_147_483_647;
const A: u64 = 16_807;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Rng {
        let mut s = seed % M;
        if s == 0 {
            s = 2_147_483_646;
        }
        Rng { state: s }
    }

    pub fn next_u31(&mut self) -> u64 {
        self.state = self.state * A % M;
        self.state
    }

    /// Uniform integer in `0..n` computed as floor((next-1) * n / (M-1)) without floats.
    pub fn below(&mut self, n: u64) -> u64 {
        (self.next_u31() - 1) * n / (M - 1)
    }

    /// True with probability `per_mille / 1000`.
    pub fn chance_pm(&mut self, per_mille: u32) -> bool {
        self.below(1000) < per_mille as u64
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Bag7 {
    rng: Rng,
    base: Vec<Piece>,
    buf: Vec<Piece>,
}

impl Bag7 {
    pub fn new(seed: u64, base_order: &str) -> Bag7 {
        Bag7 {
            rng: Rng::new(seed),
            base: crate::piece::parse_queue(base_order),
            buf: Vec::new(),
        }
    }

    fn refill(&mut self) {
        let mut bag = self.base.clone();
        for i in (1..bag.len()).rev() {
            let j = self.rng.below(i as u64 + 1) as usize;
            bag.swap(i, j);
        }
        // buf is consumed from the end
        bag.reverse();
        self.buf = bag;
    }

    pub fn next_piece(&mut self) -> Piece {
        if self.buf.is_empty() {
            self.refill();
        }
        self.buf.pop().expect("bag refilled")
    }

    /// Pieces remaining in the current bag (public information a player can count).
    pub fn remaining_in_bag(&self) -> &[Piece] {
        &self.buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bags_are_permutations_and_deterministic() {
        let mut a = Bag7::new(12345, "ZLOSIJT");
        let mut b = Bag7::new(12345, "ZLOSIJT");
        for _ in 0..50 {
            let mut seen = [false; 7];
            for _ in 0..7 {
                let p = a.next_piece();
                assert_eq!(p, b.next_piece());
                assert!(!seen[p.index()]);
                seen[p.index()] = true;
            }
        }
    }
}
