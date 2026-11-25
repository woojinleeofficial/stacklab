//! Opener library (spec 8.3): definitions, branches and templates loaded from
//! `data/openers/*.toml`. Memorized branches are shapes with a piece letter per cell; each
//! letter group becomes one target placement. Mirrors are generated (x flipped, S<->Z, J<->L).

use serde::Deserialize;
use stacklab_engine::board::{Board, PieceState, WIDTH};
use stacklab_engine::{Piece, Rot, Spin};
use std::collections::BTreeMap;

pub const BUNDLED: [(&str, &str); 4] = [
    ("tki", include_str!("../../../data/openers/tki.toml")),
    ("pco", include_str!("../../../data/openers/pco.toml")),
    (
        "pc-first",
        include_str!("../../../data/openers/pc-first.toml"),
    ),
    (
        "free-transition",
        include_str!("../../../data/openers/free-transition.toml"),
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Memorized,
    Template,
}

#[derive(Debug, Clone, Deserialize)]
struct RawBranch {
    id: String,
    rows: Vec<String>,
    reserve: Option<char>,
    finisher: Option<RawFinisher>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawFinisher {
    piece: Option<char>,
    spin: Option<String>,
    lines: Option<u32>,
    pc: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct Template {
    pub pc: Option<u32>,
    pub pieces: Option<u32>,
    pub stack_pieces: Option<u32>,
    pub max_height: Option<u32>,
    pub max_holes: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawDef {
    id: String,
    name: String,
    kind: Kind,
    ruleset: String,
    version: u32,
    source: String,
    mirror: bool,
    requirements: BTreeMap<String, u32>,
    traits: BTreeMap<String, u32>,
    #[serde(default)]
    branch: Vec<RawBranch>,
    template: Option<Template>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finisher {
    /// The reserved piece must spin like this.
    Spin {
        piece: Piece,
        spin: Spin,
        lines: u32,
    },
    /// A perfect clear of this many lines follows with the next pieces.
    PerfectClear { lines: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    pub id: String,
    /// Target placements, one per piece of the setup.
    pub pieces: Vec<PieceState>,
    pub target: Board,
    pub reserve: Option<Piece>,
    pub finisher: Option<Finisher>,
    pub mirrored: bool,
}

#[derive(Debug, Clone)]
pub struct OpenerDefinition {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub ruleset: String,
    pub version: u32,
    pub source: String,
    pub requirements: BTreeMap<String, u32>,
    pub traits: BTreeMap<String, u32>,
    pub branches: Vec<Branch>,
    pub template: Option<Template>,
}

#[derive(Debug)]
pub struct LibraryError(pub String);

impl std::fmt::Display for LibraryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "opener library: {}", self.0)
    }
}

impl std::error::Error for LibraryError {}

fn mirror_piece(p: Piece) -> Piece {
    match p {
        Piece::S => Piece::Z,
        Piece::Z => Piece::S,
        Piece::J => Piece::L,
        Piece::L => Piece::J,
        other => other,
    }
}

/// Finds the piece state whose cells are exactly `cells`.
fn state_for(piece: Piece, cells: &[(i8, i8)]) -> Option<PieceState> {
    let mut want = cells.to_vec();
    want.sort_unstable();
    for r in 0..4 {
        for x in -2..WIDTH as i8 + 2 {
            for y in -2..24 {
                let s = PieceState {
                    piece,
                    rot: Rot::from_index(r),
                    x,
                    y,
                };
                if s.cell_key().to_vec() == want {
                    return Some(s);
                }
            }
        }
    }
    None
}

/// Splits same-letter cells into 4-cell connected groups (handles two pieces of one type).
fn groups(cells: &[(i8, i8)]) -> Vec<Vec<(i8, i8)>> {
    let mut left: Vec<(i8, i8)> = cells.to_vec();
    let mut out = Vec::new();
    while let Some(start) = left.pop() {
        let mut g = vec![start];
        let mut i = 0;
        while i < g.len() {
            let (x, y) = g[i];
            let mut k = 0;
            while k < left.len() {
                let (a, b) = left[k];
                if (a - x).abs() + (b - y).abs() == 1 {
                    g.push(left.swap_remove(k));
                } else {
                    k += 1;
                }
            }
            i += 1;
        }
        out.push(g);
    }
    out
}

fn parse_rows(rows: &[String], mirror: bool) -> Result<Vec<PieceState>, String> {
    let h = rows.len();
    let mut by_piece: BTreeMap<Piece, Vec<(i8, i8)>> = BTreeMap::new();
    for (i, row) in rows.iter().enumerate() {
        if row.chars().count() != WIDTH {
            return Err(format!("row {i} is not {WIDTH} wide"));
        }
        let y = (h - 1 - i) as i8;
        for (x, c) in row.chars().enumerate() {
            if c == '.' {
                continue;
            }
            let p = Piece::from_char(c).ok_or_else(|| format!("bad cell '{c}'"))?;
            let (p, x) = if mirror {
                (mirror_piece(p), WIDTH as i8 - 1 - x as i8)
            } else {
                (p, x as i8)
            };
            by_piece.entry(p).or_default().push((x, y));
        }
    }
    let mut out = Vec::new();
    for (p, cells) in by_piece {
        for g in groups(&cells) {
            if g.len() != 4 {
                return Err(format!(
                    "{} cells of {} do not form tetrominoes",
                    g.len(),
                    p.to_char()
                ));
            }
            out.push(state_for(p, &g).ok_or_else(|| {
                format!(
                    "{} cells {:?} are not a {} shape",
                    p.to_char(),
                    g,
                    p.to_char()
                )
            })?);
        }
    }
    Ok(out)
}

fn build_branch(raw: &RawBranch, mirror: bool) -> Result<Branch, String> {
    let pieces = parse_rows(&raw.rows, mirror)?;
    let mut target = Board::default();
    for s in &pieces {
        target.place(s);
    }
    let fix = |p: Piece| if mirror { mirror_piece(p) } else { p };
    let finisher = match &raw.finisher {
        None => None,
        Some(f) if f.pc.is_some() => Some(Finisher::PerfectClear {
            lines: f.pc.unwrap_or(4),
        }),
        Some(f) => {
            let piece = f.piece.and_then(Piece::from_char).ok_or("finisher piece")?;
            let spin = match f.spin.as_deref() {
                Some("Full") => Spin::Full,
                Some("Mini") => Spin::Mini,
                _ => return Err("finisher spin must be Full or Mini".into()),
            };
            Some(Finisher::Spin {
                piece: fix(piece),
                spin,
                lines: f.lines.unwrap_or(1),
            })
        }
    };
    Ok(Branch {
        id: if mirror {
            format!("{}-mirror", raw.id)
        } else {
            raw.id.clone()
        },
        pieces,
        target,
        reserve: raw.reserve.and_then(Piece::from_char).map(fix),
        finisher,
        mirrored: mirror,
    })
}

impl OpenerDefinition {
    pub fn from_toml(text: &str) -> Result<OpenerDefinition, LibraryError> {
        let raw: RawDef = toml::from_str(text).map_err(|e| LibraryError(e.to_string()))?;
        let mut branches = Vec::new();
        for b in &raw.branch {
            branches.push(
                build_branch(b, false)
                    .map_err(|e| LibraryError(format!("{}/{}: {e}", raw.id, b.id)))?,
            );
            if raw.mirror {
                branches.push(
                    build_branch(b, true)
                        .map_err(|e| LibraryError(format!("{}/{} mirror: {e}", raw.id, b.id)))?,
                );
            }
        }
        if raw.kind == Kind::Memorized && branches.is_empty() {
            return Err(LibraryError(format!(
                "{}: memorized opener without branches",
                raw.id
            )));
        }
        if raw.kind == Kind::Template && raw.template.is_none() {
            return Err(LibraryError(format!("{}: template without rule", raw.id)));
        }
        Ok(OpenerDefinition {
            id: raw.id,
            name: raw.name,
            kind: raw.kind,
            ruleset: raw.ruleset,
            version: raw.version,
            source: raw.source,
            requirements: raw.requirements,
            traits: raw.traits,
            branches,
            template: raw.template,
        })
    }

    pub fn branch(&self, id: &str) -> Option<&Branch> {
        self.branches.iter().find(|b| b.id == id)
    }
}

#[derive(Debug, Clone)]
pub struct Library {
    pub openers: Vec<OpenerDefinition>,
}

impl Library {
    pub fn bundled() -> Library {
        Library {
            openers: BUNDLED
                .iter()
                .map(|(_, t)| OpenerDefinition::from_toml(t).expect("bundled opener data is valid"))
                .collect(),
        }
    }

    pub fn get(&self, id: &str) -> Option<&OpenerDefinition> {
        self.openers.iter().find(|o| o.id == id)
    }

    /// Ids valid for a ruleset.
    pub fn ids(&self, ruleset: &str) -> Vec<String> {
        self.openers
            .iter()
            .filter(|o| o.ruleset == ruleset)
            .map(|o| o.id.clone())
            .collect()
    }
}

/// Precomputed coverage (data/openers/coverage.toml), keyed by opener id.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CoverageRecord {
    pub version: u32,
    /// Hex bitset of visible-info feasibility per first bag (exhaustive scopes only).
    #[serde(default)]
    pub bits: String,
    pub scope: String,
    pub exhaustive: bool,
    pub total: u32,
    pub full: u32,
    pub visible: u32,
    pub ci_low: f64,
    pub ci_high: f64,
}

impl CoverageRecord {
    /// Spec 8.2: "100 %" only for an exhaustive scope where every case is feasible.
    pub fn is_guaranteed(&self) -> bool {
        self.exhaustive && self.visible == self.total
    }
    pub fn rate(&self) -> f64 {
        self.visible as f64 / self.total.max(1) as f64
    }
}

pub const COVERAGE_TOML: &str = include_str!("../../../data/openers/coverage.toml");

pub fn coverage_table() -> BTreeMap<String, CoverageRecord> {
    toml::from_str(COVERAGE_TOML).expect("coverage.toml parses")
}

/// Hex encoding of a per-bag bitset (bit i = bag i in `all_first_bags()` order).
pub fn encode_bits(v: &[bool]) -> String {
    v.chunks(4)
        .map(|c| {
            let n = c
                .iter()
                .enumerate()
                .fold(0u8, |a, (i, &b)| a | (u8::from(b) << i));
            char::from_digit(n as u32, 16).expect("nibble")
        })
        .collect()
}

pub fn decode_bits(s: &str, len: usize) -> Vec<bool> {
    let mut out = Vec::with_capacity(len);
    for ch in s.chars() {
        let n = ch.to_digit(16).unwrap_or(0);
        for i in 0..4 {
            if out.len() < len {
                out.push(n >> i & 1 == 1);
            }
        }
    }
    out
}

impl CoverageRecord {
    /// Per-bag feasibility when the scope was exhaustive.
    pub fn per_bag(&self) -> Option<Vec<bool>> {
        (self.exhaustive && !self.bits.is_empty())
            .then(|| decode_bits(&self.bits, self.total as usize))
    }
}
