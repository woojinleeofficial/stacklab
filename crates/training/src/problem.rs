//! Problem format (spec 5.3): every stored problem carries seed, rulesetVersion,
//! generatorVersion, goal, solutionWitness, difficulty, fingerprint and subtypeTags.

use serde::{Deserialize, Serialize};
use stacklab_engine::events::Fnv;
use stacklab_engine::game::{Handling, InitialState, InputEvent};
use stacklab_engine::{Piece, Rot};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Module {
    /// A. finesse & speed
    A,
    /// B. stacking & hold
    B,
    /// C. spins
    C,
    /// D. perfect clear
    D,
    /// E. openers
    E,
    /// F. attack
    F,
    /// G. defence & recovery
    G,
    /// H. quick judgment
    H,
}

impl Module {
    pub const ALL: [Module; 8] = [
        Module::A,
        Module::B,
        Module::C,
        Module::D,
        Module::E,
        Module::F,
        Module::G,
        Module::H,
    ];

    pub fn policy_key(self) -> &'static str {
        match self {
            Module::A => "A_finesse",
            Module::B => "B_stacking",
            Module::C => "C_spin",
            Module::D => "D_pc",
            Module::E => "E_opener",
            Module::F => "F_attack",
            Module::G => "G_defense",
            Module::H => "H_judgment",
        }
    }

    pub fn letter(self) -> char {
        (b'A' + self as u8) as char
    }
}

/// Target placement identified by its cells (rotations with identical cells are equivalent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetPlacement {
    pub piece: Piece,
    pub rot: Rot,
    pub x: i8,
    pub y: i8,
}

impl TargetPlacement {
    pub fn state(&self) -> stacklab_engine::PieceState {
        stacklab_engine::PieceState {
            piece: self.piece,
            rot: self.rot,
            x: self.x,
            y: self.y,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Goal {
    /// Lock the piece exactly at `target`. Finesse is judged separately: key presses for the
    /// piece (hard drop included) must not exceed `optimal_inputs`.
    Finesse {
        target: TargetPlacement,
        optimal_inputs: u32,
        allow_180: bool,
    },
    /// Place all `pieces` pieces of the fixed queue and finish with at most `max_holes` holes,
    /// a stack no higher than `max_height`, and (optionally) column `well` completely empty.
    Stacking {
        pieces: u32,
        max_holes: u32,
        max_height: u32,
        well: Option<u8>,
    },
    /// Perform a spin judged by the engine (`piece` None = any piece) clearing `lines` lines
    /// within `within_pieces` locks.
    Spin {
        piece: Option<Piece>,
        spin: stacklab_engine::Spin,
        lines: u32,
        within_pieces: u32,
    },
    /// Empty the whole matrix (hidden rows included) within `within_pieces` locks.
    PerfectClear { within_pieces: u32 },
    /// Send at least `min_attack` lines (before cancelling) within `within_pieces` locks.
    Attack { min_attack: u32, within_pieces: u32 },
    /// Survive and clear at least `min_lines` lines within `pieces` locks (garbage terrain).
    Downstack { pieces: u32, min_lines: u32 },
    /// Opener execution (module E): after exactly `pieces` locks the field must equal one of the
    /// accepted branch shapes; with a spin finisher the next lock must perform it.
    OpenerBuild {
        opener: String,
        accepted: Vec<OpenerShape>,
        pieces: u32,
        finisher: Option<(Piece, stacklab_engine::Spin, u32)>,
    },
    /// Quick judgment: lock the current piece at one of the accepted placements. `rated` is
    /// false when the evaluation is not confident enough to grade a skill rating (spec 4, H).
    Placement {
        accepted: Vec<TargetPlacement>,
        rated: bool,
    },
}

impl Goal {
    /// Whether attempts at this goal may update a skill rating.
    pub fn rated(&self) -> bool {
        !matches!(self, Goal::Placement { rated: false, .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenerShape {
    pub branch: String,
    /// Bottom-up rows of the finished shape.
    pub rows: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Witness {
    /// Handling the witness inputs were produced for.
    pub handling: Handling,
    pub inputs: Vec<InputEvent>,
    /// Frames needed to replay the witness to completion.
    pub frames: u64,
    /// The witness as placements (used for hints / "show answer"), with "hold first" flags.
    pub placements: Vec<WitnessStep>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WitnessStep {
    pub target: TargetPlacement,
    pub use_hold: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Problem {
    pub id: String,
    pub module: Module,
    pub subtype_tags: Vec<String>,
    pub seed: u64,
    pub ruleset_version: String,
    pub generator_version: String,
    pub validation_version: String,
    pub initial: InitialState,
    /// Next pieces shown to the player (difficulty factor; judging does not depend on it).
    pub visible_next: u8,
    pub goal: Goal,
    pub witness: Witness,
    /// Difficulty on the Glicko-2 scale (anchor = 1500).
    pub difficulty: f64,
    pub param_bucket: String,
    pub time_limit_secs: u32,
    pub fingerprint: u64,
}

/// Fingerprint over start state + goal (spec 5.3 duplicate check). Stable across builds.
pub fn fingerprint(module: Module, initial: &InitialState, goal: &Goal) -> u64 {
    let mut h = Fnv::default();
    h.bytes(&[module as u8]);
    for r in &initial.rows {
        h.u64(*r as u64);
    }
    h.bytes(b"|q");
    for p in &initial.queue {
        h.bytes(&[*p as u8]);
    }
    h.bytes(&[initial.hold.map_or(9, |p| p as u8)]);
    h.i64(initial.b2b as i64);
    h.i64(initial.combo as i64);
    h.bytes(
        serde_json::to_string(goal)
            .expect("goal serialises")
            .as_bytes(),
    );
    h.finish()
}

impl Problem {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("problem serialises")
    }
    pub fn from_json(s: &str) -> Result<Problem, String> {
        serde_json::from_str(s).map_err(|e| e.to_string())
    }
}
