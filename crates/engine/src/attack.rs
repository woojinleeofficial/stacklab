//! Attack computation: clear table, B2B charging + Surge, multiplier combo. Integer only.

use crate::piece::Piece;
use crate::rotation::Spin;
use crate::ruleset::Ruleset;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClearInfo {
    pub piece: Piece,
    pub lines: u32,
    pub spin: Spin,
    pub perfect_clear: bool,
}

/// B2B and combo counters. `-1` means "no chain". `b2b` follows the displayed "B2B xN" value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChainState {
    pub b2b: i32,
    pub combo: i32,
}

impl Default for ChainState {
    fn default() -> Self {
        ChainState { b2b: -1, combo: -1 }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttackResult {
    /// Garbage packets produced by this lock, in send order (main attack, PC bonus, surge parts).
    pub packets: Vec<u32>,
    pub surge_released: u32,
    pub b2b_before: i32,
    pub b2b_after: i32,
    pub combo_after: i32,
}

impl AttackResult {
    pub fn total(&self) -> u32 {
        self.packets.iter().sum()
    }
}

pub fn base_attack(rs: &Ruleset, c: &ClearInfo) -> u32 {
    let a = &rs.attack;
    match (c.spin, c.piece, c.lines) {
        (_, _, 0) => 0,
        (Spin::Full, Piece::T, 1) => a.tspin_single,
        (Spin::Full, Piece::T, 2) => a.tspin_double,
        (Spin::Full, Piece::T, _) => a.tspin_triple,
        (Spin::Full, _, _) | (Spin::Mini, _, _) => match c.lines {
            1 => a.tspin_mini_single,
            2 => a.tspin_mini_double,
            3 => a.mini_triple,
            _ => a.mini_quad,
        },
        (Spin::None, _, 1) => a.single,
        (Spin::None, _, 2) => a.double,
        (Spin::None, _, 3) => a.triple,
        (Spin::None, _, _) => a.quad,
    }
}

pub fn combo_attack(rs: &Ruleset, base: u32, combo: i32) -> u32 {
    if combo <= 0 {
        return base;
    }
    let combo = combo as u32;
    if base > 0 {
        // floor(base * (1 + 0.25 * combo)) == floor(base * (4 + combo) / 4)
        base * (4 + combo * rs.combo.quarter_step) / 4
    } else {
        rs.combo
            .zero_base_thresholds
            .iter()
            .take_while(|&&t| combo >= t)
            .count() as u32
    }
}

/// Attack multiplier (1/1000) after `tick` frames: constant until margin time, then rising
/// linearly per whole second (TL-S2 existence unverified, see ruleset `[margin]`).
pub fn margin_multiplier_milli(rs: &Ruleset, tick: u64) -> u64 {
    let m = &rs.margin;
    if !m.enabled || tick <= m.margin_time_frames as u64 {
        return m.multiplier_start_milli as u64;
    }
    let secs = (tick - m.margin_time_frames as u64) / rs.timing.frames_per_second as u64;
    m.multiplier_start_milli as u64 + secs * m.multiplier_increase_milli_per_second as u64
}

pub fn surge_amount(rs: &Ruleset, b2b: i32) -> u32 {
    let start = rs.b2b.surge_start_b2b as i32;
    if b2b < start {
        0
    } else {
        rs.b2b.surge_start_lines + (b2b - start) as u32 * rs.b2b.surge_lines_per_level
    }
}

/// Splits surge into segments; the earliest segments carry the remainder.
pub fn split_surge(total: u32, segments: u32) -> Vec<u32> {
    let base = total / segments;
    let rem = total % segments;
    (0..segments)
        .map(|i| base + u32::from(i < rem))
        .filter(|&v| v > 0)
        .collect()
}

/// Applies one piece lock to the chain state and returns the produced attack.
pub fn on_lock(rs: &Ruleset, chain: &mut ChainState, c: &ClearInfo) -> AttackResult {
    let mut res = AttackResult {
        b2b_before: chain.b2b,
        ..Default::default()
    };
    if c.lines == 0 {
        chain.combo = -1;
        res.b2b_after = chain.b2b;
        res.combo_after = chain.combo;
        return res;
    }
    chain.combo += 1;
    let difficult = c.lines >= 4 || c.spin != Spin::None;
    let mut surge = 0;
    if difficult {
        chain.b2b += 1;
    } else {
        surge = surge_amount(rs, chain.b2b);
        chain.b2b = -1;
    }
    let mut base = base_attack(rs, c);
    if difficult && chain.b2b >= 1 {
        base += rs.b2b.bonus_lines;
    }
    let main = combo_attack(rs, base, chain.combo);
    if main > 0 {
        res.packets.push(main);
    }
    if c.perfect_clear {
        if rs.attack.pc_separate_attack || res.packets.is_empty() {
            res.packets.push(rs.attack.pc_attack);
        } else {
            res.packets[0] += rs.attack.pc_attack;
        }
        chain.b2b = chain.b2b.max(-1) + rs.attack.pc_b2b_add as i32;
    }
    if surge > 0 {
        res.packets
            .extend(split_surge(surge, rs.b2b.surge_segments));
        res.surge_released = surge;
    }
    res.b2b_after = chain.b2b;
    res.combo_after = chain.combo;
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ci(piece: Piece, lines: u32, spin: Spin) -> ClearInfo {
        ClearInfo {
            piece,
            lines,
            spin,
            perfect_clear: false,
        }
    }

    /// Row "combo 0..20" for the non-B2B clears in the public TETR.IO combo table.
    #[test]
    fn combo_table_matches_public_chart() {
        let rs = Ruleset::tl_s2();
        let single = [
            0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3,
        ];
        let double = [
            1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 6,
        ];
        let quad = [
            4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
        ];
        let tst = [
            6, 7, 9, 10, 12, 13, 15, 16, 18, 19, 21, 22, 24, 25, 27, 28, 30, 31, 33, 34, 36,
        ];
        for c in 0..=20 {
            assert_eq!(combo_attack(&rs, 0, c), single[c as usize], "single c={c}");
            assert_eq!(combo_attack(&rs, 1, c), double[c as usize], "double c={c}");
            assert_eq!(combo_attack(&rs, 4, c), quad[c as usize], "quad c={c}");
            assert_eq!(combo_attack(&rs, 6, c), tst[c as usize], "tst c={c}");
        }
    }

    #[test]
    fn b2b_bonus_and_surge() {
        let rs = Ruleset::tl_s2();
        let mut ch = ChainState::default();
        // 1st quad: no bonus
        let r = on_lock(&rs, &mut ch, &ci(Piece::I, 4, Spin::None));
        assert_eq!(r.packets, vec![4]);
        assert_eq!(ch.b2b, 0);
        // break combo by a non-clearing lock
        on_lock(&rs, &mut ch, &ci(Piece::O, 0, Spin::None));
        // 2nd quad: B2B x1 -> +1
        let r = on_lock(&rs, &mut ch, &ci(Piece::I, 4, Spin::None));
        assert_eq!(r.packets, vec![5]);
        for _ in 0..7 {
            on_lock(&rs, &mut ch, &ci(Piece::O, 0, Spin::None));
            on_lock(&rs, &mut ch, &ci(Piece::T, 2, Spin::Full));
        }
        assert_eq!(ch.b2b, 8);
        on_lock(&rs, &mut ch, &ci(Piece::O, 0, Spin::None));
        // a plain double breaks B2B x8 -> surge 8 split 3/3/2 after the double's own 1 line
        let r = on_lock(&rs, &mut ch, &ci(Piece::L, 2, Spin::None));
        assert_eq!(r.surge_released, 8);
        assert_eq!(r.packets, vec![1, 3, 3, 2]);
        assert_eq!(ch.b2b, -1);
    }

    #[test]
    fn margin_multiplier_ramps_after_margin_time() {
        let mut rs = Ruleset::tl_s2();
        assert_eq!(margin_multiplier_milli(&rs, 0), 1000);
        assert_eq!(margin_multiplier_milli(&rs, 10_800), 1000);
        assert_eq!(margin_multiplier_milli(&rs, 10_800 + 60 * 125), 2000);
        rs.margin.enabled = false;
        assert_eq!(margin_multiplier_milli(&rs, 10_800 + 60 * 125), 1000);
    }

    #[test]
    fn surge_split_remainder_first() {
        assert_eq!(split_surge(4, 3), vec![2, 1, 1]);
        assert_eq!(split_surge(5, 3), vec![2, 2, 1]);
        assert_eq!(split_surge(6, 3), vec![2, 2, 2]);
    }
}
