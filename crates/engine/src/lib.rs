//! StackLab engine core.
//!
//! Rules data, board, SRS+ rotation, All-Mini+ spins, attack (combo, B2B charging, Surge, PC),
//! deterministic 7-bag, reachable placements and witness paths, the fixed-tick [`game::Game`]
//! (handling, gravity, lock delay, hold, garbage, clutch, top-out, 40-line mode), engine events,
//! replays and the two-player [`versus::Versus`]. No UI or OS dependencies, integers only.

pub mod attack;
pub mod board;
pub mod events;
pub mod game;
pub mod movegen;
pub mod piece;
pub mod replay;
pub mod rng;
pub mod rotation;
pub mod ruleset;
pub mod versus;

pub use board::{Board, PieceState};
pub use movegen::{lock, placements, MoveRules, Placement};
pub use piece::{Piece, Rot};
pub use rotation::Spin;
pub use ruleset::Ruleset;
