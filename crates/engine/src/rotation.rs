//! SRS+ rotation with kick tables taken from the ruleset, plus All-Mini+ spin detection.

use crate::board::{Board, PieceState};
use crate::piece::{Piece, Rot};
use crate::ruleset::Ruleset;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RotDir {
    Cw,
    Ccw,
    Flip,
}

impl RotDir {
    pub fn apply(self, r: Rot) -> Rot {
        match self {
            RotDir::Cw => r.cw(),
            RotDir::Ccw => r.ccw(),
            RotDir::Flip => r.flip(),
        }
    }
}

/// Kick offsets indexed by [from][to].
#[derive(Debug, Clone)]
pub struct KickTables {
    jlstz: [[Vec<(i8, i8)>; 4]; 4],
    i: [[Vec<(i8, i8)>; 4]; 4],
}

impl KickTables {
    pub fn from_ruleset(rs: &Ruleset) -> KickTables {
        let mut jlstz: [[Vec<(i8, i8)>; 4]; 4] = Default::default();
        let mut i: [[Vec<(i8, i8)>; 4]; 4] = Default::default();
        for from in 0..4 {
            for to in 0..4 {
                if from == to {
                    continue;
                }
                let (f, t) = (Rot::from_index(from), Rot::from_index(to));
                let key: String = [f.key(), t.key()].iter().collect();
                let conv = |v: &Vec<[i8; 2]>| v.iter().map(|k| (k[0], k[1])).collect::<Vec<_>>();
                if (from + 2) % 4 == to {
                    let t180 = conv(&rs.rotation.r180[&key]);
                    jlstz[from][to] = t180.clone();
                    i[from][to] = if rs.rotation.i_uses_r180 {
                        t180
                    } else {
                        vec![(0, 0)]
                    };
                } else {
                    jlstz[from][to] = conv(&rs.rotation.jlstz[&key]);
                    i[from][to] = conv(&rs.rotation.i[&key]);
                }
            }
        }
        KickTables { jlstz, i }
    }

    pub fn kicks(&self, piece: Piece, from: Rot, to: Rot) -> &[(i8, i8)] {
        const NONE: &[(i8, i8)] = &[(0, 0)];
        match piece {
            Piece::O => NONE,
            Piece::I => &self.i[from.index()][to.index()],
            _ => &self.jlstz[from.index()][to.index()],
        }
    }

    /// Returns the rotated state and the index of the kick that succeeded.
    pub fn try_rotate(
        &self,
        board: &Board,
        s: &PieceState,
        dir: RotDir,
    ) -> Option<(PieceState, u8)> {
        let to = dir.apply(s.rot);
        for (idx, &(dx, dy)) in self.kicks(s.piece, s.rot, to).iter().enumerate() {
            let n = PieceState {
                piece: s.piece,
                rot: to,
                x: s.x + dx,
                y: s.y + dy,
            };
            if board.fits(&n) {
                return Some((n, idx as u8));
            }
        }
        None
    }

    pub fn kick_count(&self, piece: Piece, from: Rot, to: Rot) -> usize {
        self.kicks(piece, from, to).len()
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum Spin {
    None,
    Mini,
    Full,
}

/// Spin rules derived from the ruleset `[spin]` table.
#[derive(Debug, Clone)]
pub struct SpinRules {
    all_mini: bool,
    t_corner_rule: bool,
    t_last_kick_upgrades: bool,
    immobile_dirs: Vec<(i8, i8)>,
}

impl SpinRules {
    pub fn from_ruleset(rs: &Ruleset) -> SpinRules {
        let immobile_dirs = rs
            .spin
            .immobile_dirs
            .iter()
            .filter_map(|d| match d.as_str() {
                "left" => Some((-1, 0)),
                "right" => Some((1, 0)),
                "up" => Some((0, 1)),
                "down" => Some((0, -1)),
                _ => None,
            })
            .collect();
        SpinRules {
            all_mini: rs.spin.mode == "all-mini+" || rs.spin.mode == "all-mini",
            t_corner_rule: rs.spin.t_corner_rule,
            t_last_kick_upgrades: rs.spin.t_last_kick_upgrades_to_full,
            immobile_dirs,
        }
    }

    pub fn immobile(&self, board: &Board, s: &PieceState) -> bool {
        self.immobile_dirs
            .iter()
            .all(|&(dx, dy)| !board.fits(&s.shifted(dx, dy)))
    }

    /// Classify the lock of `s`. `last_rotation` is `Some((kick_index, kick_count))` when the
    /// last successful action before locking was a rotation.
    pub fn detect(
        &self,
        board: &Board,
        s: &PieceState,
        last_rotation: Option<(u8, usize)>,
    ) -> Spin {
        let Some((kick_idx, kick_count)) = last_rotation else {
            return Spin::None;
        };
        if s.piece == Piece::T && self.t_corner_rule {
            let corners = [(-1, 1), (1, 1), (1, -1), (-1, -1)];
            let filled: Vec<bool> = corners
                .iter()
                .map(|&(dx, dy)| board.occupied(s.x + dx, s.y + dy))
                .collect();
            let count = filled.iter().filter(|&&f| f).count();
            if count >= 3 {
                // front corners: the two on the side the T points to
                let front = match s.rot {
                    Rot::N => [0, 1],
                    Rot::E => [1, 2],
                    Rot::S => [2, 3],
                    Rot::W => [3, 0],
                };
                let both_front = filled[front[0]] && filled[front[1]];
                let last_kick = kick_count >= 5 && kick_idx as usize == 4;
                return if both_front || (self.t_last_kick_upgrades && last_kick) {
                    Spin::Full
                } else {
                    Spin::Mini
                };
            }
        }
        if self.all_mini && self.immobile(board, s) {
            return Spin::Mini;
        }
        Spin::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SRS+ property from the public chart: I kicks are mirror images left/right.
    #[test]
    fn srs_plus_i_kicks_are_mirror_symmetric() {
        let k = KickTables::from_ruleset(&Ruleset::tl_s2());
        let pairs = [
            (Rot::N, Rot::E, Rot::N, Rot::W),
            (Rot::E, Rot::N, Rot::W, Rot::N),
            (Rot::E, Rot::S, Rot::W, Rot::S),
            (Rot::S, Rot::E, Rot::S, Rot::W),
        ];
        for (a, b, c, d) in pairs {
            let left: Vec<_> = k
                .kicks(Piece::I, a, b)
                .iter()
                .map(|&(x, y)| (-x, y))
                .collect();
            assert_eq!(
                left,
                k.kicks(Piece::I, c, d),
                "{a:?}->{b:?} vs {c:?}->{d:?}"
            );
        }
    }

    #[test]
    fn t_180_off_the_floor_kick() {
        let k = KickTables::from_ruleset(&Ruleset::tl_s2());
        // T resting on the floor pointing up: plain 180 would push the stem into the floor
        let b = Board::default();
        let s = PieceState {
            piece: Piece::T,
            rot: Rot::N,
            x: 4,
            y: 0,
        };
        let (n, idx) = k.try_rotate(&b, &s, RotDir::Flip).expect("180 succeeds");
        assert_eq!((n.rot, n.x, n.y, idx), (Rot::S, 4, 1, 1));
    }
}
