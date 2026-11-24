//! Tetromino kinds, rotation states and cell shapes (y-up, relative to the rotation centre).

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum Piece {
    I,
    O,
    T,
    S,
    Z,
    J,
    L,
}

pub const ALL_PIECES: [Piece; 7] = [
    Piece::I,
    Piece::O,
    Piece::T,
    Piece::S,
    Piece::Z,
    Piece::J,
    Piece::L,
];

impl Piece {
    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_char(c: char) -> Option<Piece> {
        Some(match c.to_ascii_uppercase() {
            'I' => Piece::I,
            'O' => Piece::O,
            'T' => Piece::T,
            'S' => Piece::S,
            'Z' => Piece::Z,
            'J' => Piece::J,
            'L' => Piece::L,
            _ => return None,
        })
    }

    pub fn to_char(self) -> char {
        match self {
            Piece::I => 'I',
            Piece::O => 'O',
            Piece::T => 'T',
            Piece::S => 'S',
            Piece::Z => 'Z',
            Piece::J => 'J',
            Piece::L => 'L',
        }
    }

    /// Cells for a rotation state (y-up). Matches the SRS "true rotation" frames the kick
    /// tables are defined against.
    pub fn cells(self, rot: Rot) -> [(i8, i8); 4] {
        CELLS[self.index()][rot.index()]
    }
}

pub fn parse_queue(s: &str) -> Vec<Piece> {
    s.chars().filter_map(Piece::from_char).collect()
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum Rot {
    N = 0,
    E = 1,
    S = 2,
    W = 3,
}

impl Rot {
    pub fn index(self) -> usize {
        self as usize
    }
    pub fn from_index(i: usize) -> Rot {
        match i & 3 {
            0 => Rot::N,
            1 => Rot::E,
            2 => Rot::S,
            _ => Rot::W,
        }
    }
    pub fn cw(self) -> Rot {
        Rot::from_index(self.index() + 1)
    }
    pub fn ccw(self) -> Rot {
        Rot::from_index(self.index() + 3)
    }
    pub fn flip(self) -> Rot {
        Rot::from_index(self.index() + 2)
    }
    /// Name used in ruleset kick keys ("0", "R", "2", "L").
    pub fn key(self) -> char {
        match self {
            Rot::N => '0',
            Rot::E => 'R',
            Rot::S => '2',
            Rot::W => 'L',
        }
    }
}

const fn rot_cw(c: [(i8, i8); 4]) -> [(i8, i8); 4] {
    // (x, y) -> (y, -x) in a y-up frame
    [
        (c[0].1, -c[0].0),
        (c[1].1, -c[1].0),
        (c[2].1, -c[2].0),
        (c[3].1, -c[3].0),
    ]
}

const fn four(c: [(i8, i8); 4]) -> [[(i8, i8); 4]; 4] {
    let r = rot_cw(c);
    let s = rot_cw(r);
    let w = rot_cw(s);
    [c, r, s, w]
}

const T0: [(i8, i8); 4] = [(-1, 0), (0, 0), (1, 0), (0, 1)];
const S0: [(i8, i8); 4] = [(-1, 0), (0, 0), (0, 1), (1, 1)];
const Z0: [(i8, i8); 4] = [(-1, 1), (0, 1), (0, 0), (1, 0)];
const J0: [(i8, i8); 4] = [(-1, 1), (-1, 0), (0, 0), (1, 0)];
const L0: [(i8, i8); 4] = [(1, 1), (-1, 0), (0, 0), (1, 0)];

/// The I and O pieces rotate around a point between cells; encode their four frames directly.
const I_ROT: [[(i8, i8); 4]; 4] = [
    [(-1, 0), (0, 0), (1, 0), (2, 0)],
    [(1, 1), (1, 0), (1, -1), (1, -2)],
    [(-1, -1), (0, -1), (1, -1), (2, -1)],
    [(0, 1), (0, 0), (0, -1), (0, -2)],
];
const O_ROT: [[(i8, i8); 4]; 4] = [[(0, 0), (1, 0), (0, 1), (1, 1)]; 4];

const CELLS: [[[(i8, i8); 4]; 4]; 7] = [
    I_ROT,
    O_ROT,
    four(T0),
    four(S0),
    four(Z0),
    four(J0),
    four(L0),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_points_down_in_state_2() {
        let c = Piece::T.cells(Rot::S);
        assert!(c.contains(&(0, -1)));
        assert!(c.contains(&(-1, 0)) && c.contains(&(1, 0)));
    }

    #[test]
    fn rotations_cycle() {
        for p in ALL_PIECES {
            for r in 0..4 {
                let rot = Rot::from_index(r);
                assert_eq!(rot.cw().ccw(), rot);
                assert_eq!(rot.flip().flip(), rot);
                assert_eq!(p.cells(rot).len(), 4);
            }
        }
    }
}
