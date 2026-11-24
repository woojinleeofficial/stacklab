//! Engine events (spec 3.3). Every event carries the tick, a state reference and the ruleset
//! version so problem judging, statistics and replays can rely on them alone.

use crate::piece::{Piece, Rot};
use crate::rotation::Spin;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TopOutReason {
    /// The next piece could not spawn.
    BlockOut,
    /// Garbage pushed blocks out of the top of the matrix.
    GarbageOverflow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatchEndReason {
    TopOut(TopOutReason),
    /// 40-line (sprint) goal reached.
    GoalReached,
    /// The opponent topped out (versus).
    OpponentToppedOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventKind {
    PieceSpawn {
        piece: Piece,
    },
    PieceLock {
        piece: Piece,
        rot: Rot,
        x: i8,
        y: i8,
    },
    LineClear {
        lines: u32,
    },
    Spin {
        piece: Piece,
        spin: Spin,
        lines: u32,
    },
    PerfectClear,
    B2bChange {
        from: i32,
        to: i32,
    },
    SurgeCharge {
        amount: u32,
    },
    SurgeRelease {
        amount: u32,
        segments: Vec<u32>,
    },
    OpenerPhaseEnd,
    ClutchClear {
        streak: u32,
    },
    ComboChange {
        from: i32,
        to: i32,
    },
    AttackSent {
        lines: u32,
    },
    AttackCancelled {
        lines: u32,
    },
    GarbageReceived {
        lines: u32,
        hole: u8,
    },
    HoldUsed {
        held: Piece,
        took: Option<Piece>,
    },
    TopOut {
        reason: TopOutReason,
    },
    MatchEnd {
        reason: MatchEndReason,
    },
}

impl EventKind {
    pub fn name(&self) -> &'static str {
        match self {
            EventKind::PieceSpawn { .. } => "PIECE_SPAWN",
            EventKind::PieceLock { .. } => "PIECE_LOCK",
            EventKind::LineClear { .. } => "LINE_CLEAR",
            EventKind::Spin { .. } => "SPIN",
            EventKind::PerfectClear => "PERFECT_CLEAR",
            EventKind::B2bChange { .. } => "B2B_CHANGE",
            EventKind::SurgeCharge { .. } => "SURGE_CHARGE",
            EventKind::SurgeRelease { .. } => "SURGE_RELEASE",
            EventKind::OpenerPhaseEnd => "OPENER_PHASE_END",
            EventKind::ClutchClear { .. } => "CLUTCH_CLEAR",
            EventKind::ComboChange { .. } => "COMBO_CHANGE",
            EventKind::AttackSent { .. } => "ATTACK_SENT",
            EventKind::AttackCancelled { .. } => "ATTACK_CANCELLED",
            EventKind::GarbageReceived { .. } => "GARBAGE_RECEIVED",
            EventKind::HoldUsed { .. } => "HOLD_USED",
            EventKind::TopOut { .. } => "TOP_OUT",
            EventKind::MatchEnd { .. } => "MATCH_END",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// Frame (tick) the event happened in.
    pub tick: u64,
    /// Sub-frame inside the tick.
    pub sub: u32,
    /// Number of pieces locked before this event.
    pub piece_index: u32,
    /// Hash of the full game state right after the event (state reference).
    pub state_hash: u64,
    pub ruleset_version: Arc<str>,
    pub kind: EventKind,
}

/// FNV-1a 64-bit. Stable across builds and platforms (unlike `std::hash`).
#[derive(Debug, Clone, Copy)]
pub struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    pub fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.0 ^= x as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    pub fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
    pub fn i64(&mut self, v: i64) {
        self.bytes(&v.to_le_bytes());
    }
    pub fn finish(&self) -> u64 {
        self.0
    }
}
