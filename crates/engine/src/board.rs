//! Playfield as bit rows. Row 0 is the bottom row; bit x set = cell occupied.

use crate::piece::{Piece, Rot};

pub const WIDTH: usize = 10;
pub const MAX_ROWS: usize = 40;
pub const FULL_ROW: u16 = (1 << WIDTH) - 1;

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Board {
    pub rows: [u16; MAX_ROWS],
}

impl Default for Board {
    fn default() -> Self {
        Board {
            rows: [0; MAX_ROWS],
        }
    }
}

impl std::fmt::Debug for Board {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f)?;
        for y in (0..self.height().max(1)).rev() {
            let mut line = String::with_capacity(WIDTH);
            for x in 0..WIDTH {
                line.push(if self.occupied(x as i8, y as i8) {
                    '#'
                } else {
                    '.'
                });
            }
            writeln!(f, "{line}")?;
        }
        Ok(())
    }
}

/// A piece at a position. `x`,`y` is the rotation centre in board coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PieceState {
    pub piece: Piece,
    pub rot: Rot,
    pub x: i8,
    pub y: i8,
}

impl PieceState {
    pub fn cells(&self) -> [(i8, i8); 4] {
        let mut out = self.piece.cells(self.rot);
        for c in out.iter_mut() {
            c.0 += self.x;
            c.1 += self.y;
        }
        out
    }

    pub fn shifted(&self, dx: i8, dy: i8) -> PieceState {
        PieceState {
            x: self.x + dx,
            y: self.y + dy,
            ..*self
        }
    }

    /// Canonical cell set as a sorted array: different rotations with identical cells compare equal.
    pub fn cell_key(&self) -> [(i8, i8); 4] {
        let mut c = self.cells();
        c.sort_unstable();
        c
    }
}

impl Board {
    pub fn from_ascii(rows_top_to_bottom: &[&str]) -> Board {
        let mut b = Board::default();
        let n = rows_top_to_bottom.len();
        for (i, line) in rows_top_to_bottom.iter().enumerate() {
            let y = n - 1 - i;
            for (x, ch) in line.chars().take(WIDTH).enumerate() {
                if ch != '.' && ch != '_' && ch != ' ' {
                    b.rows[y] |= 1 << x;
                }
            }
        }
        b
    }

    #[inline]
    pub fn occupied(&self, x: i8, y: i8) -> bool {
        if x < 0 || x >= WIDTH as i8 || y < 0 {
            return true;
        }
        if y as usize >= MAX_ROWS {
            return false;
        }
        self.rows[y as usize] >> x & 1 == 1
    }

    pub fn fits(&self, s: &PieceState) -> bool {
        s.cells().iter().all(|&(x, y)| !self.occupied(x, y))
    }

    pub fn grounded(&self, s: &PieceState) -> bool {
        !self.fits(&s.shifted(0, -1))
    }

    pub fn drop_y(&self, s: &PieceState) -> PieceState {
        let mut cur = *s;
        loop {
            let n = cur.shifted(0, -1);
            if self.fits(&n) {
                cur = n;
            } else {
                return cur;
            }
        }
    }

    /// Writes the piece cells. Cells at or above MAX_ROWS are dropped (caller handles top-out).
    pub fn place(&mut self, s: &PieceState) {
        for (x, y) in s.cells() {
            if (0..MAX_ROWS as i8).contains(&y) {
                self.rows[y as usize] |= 1 << x;
            }
        }
    }

    /// Clears full rows, returns the number cleared.
    pub fn clear_lines(&mut self) -> u32 {
        let mut w = 0;
        let mut cleared = 0;
        for r in 0..MAX_ROWS {
            let row = self.rows[r];
            if row == FULL_ROW {
                cleared += 1;
            } else {
                self.rows[w] = row;
                w += 1;
            }
        }
        self.rows[w..].fill(0);
        cleared
    }

    pub fn is_empty(&self) -> bool {
        self.rows.iter().all(|&r| r == 0)
    }

    pub fn height(&self) -> usize {
        (0..MAX_ROWS)
            .rev()
            .find(|&y| self.rows[y] != 0)
            .map_or(0, |y| y + 1)
    }

    pub fn column_height(&self, x: usize) -> usize {
        (0..MAX_ROWS)
            .rev()
            .find(|&y| self.rows[y] >> x & 1 == 1)
            .map_or(0, |y| y + 1)
    }

    pub fn cell_count(&self) -> u32 {
        self.rows.iter().map(|r| r.count_ones()).sum()
    }

    /// Inserts `n` garbage rows at the bottom with a hole at `hole_x`. Returns false when cells
    /// are pushed out of the top of the matrix (top-out by garbage).
    pub fn push_garbage(&mut self, n: usize, hole_x: usize) -> bool {
        if n == 0 {
            return true;
        }
        let overflow = self.rows[MAX_ROWS - n..].iter().any(|&r| r != 0);
        self.rows.copy_within(0..MAX_ROWS - n, n);
        let g = FULL_ROW & !(1 << hole_x);
        self.rows[..n].fill(g);
        !overflow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_and_shift() {
        let mut b = Board::from_ascii(&["#.........", "##########", "#########."]);
        assert_eq!(b.clear_lines(), 1);
        assert_eq!(b.rows[0], 0b01_1111_1111);
        assert_eq!(b.rows[1], 1);
        assert_eq!(b.height(), 2);
    }

    #[test]
    fn walls_are_solid() {
        let b = Board::default();
        assert!(b.occupied(-1, 0));
        assert!(b.occupied(10, 0));
        assert!(b.occupied(0, -1));
        assert!(!b.occupied(0, 39));
    }

    #[test]
    fn garbage_push() {
        let mut b = Board::default();
        assert!(b.push_garbage(2, 3));
        assert_eq!(b.rows[0], FULL_ROW & !(1 << 3));
        assert_eq!(b.rows[1], FULL_ROW & !(1 << 3));
    }
}
