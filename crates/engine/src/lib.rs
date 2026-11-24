//! StackLab engine core.
//!
//! STEP 0 scope: rules data, board, SRS+ rotation, All-Mini+ spin detection, attack
//! calculation, deterministic 7-bag, reachable-placement generation. The fixed-tick game loop,
//! garbage queue, replay and event log are added in STEP 1 on top of these modules.

pub mod attack;
pub mod board;
pub mod movegen;
pub mod piece;
pub mod rng;
pub mod rotation;
pub mod ruleset;

pub use board::{Board, PieceState};
pub use movegen::{lock, placements, MoveRules, Placement};
pub use piece::{Piece, Rot};
pub use rotation::Spin;
pub use ruleset::Ruleset;
