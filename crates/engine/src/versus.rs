//! Two games in lock-step with attack routing (no passthrough: attacks sent in the same frame
//! are delivered after both players stepped, then cancel against each other's queues).

use crate::events::MatchEndReason;
use crate::game::{Game, Handling, InitialState, InputEvent, Mode};
use crate::ruleset::Ruleset;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Versus {
    pub players: [Game; 2],
    winner: Option<usize>,
}

impl Versus {
    pub fn new(rs: Arc<Ruleset>, seeds: [u64; 2], handling: [Handling; 2]) -> Versus {
        Self::with_initial(rs, seeds, handling, [None, None])
    }

    pub fn with_initial(
        rs: Arc<Ruleset>,
        seeds: [u64; 2],
        handling: [Handling; 2],
        initial: [Option<&InitialState>; 2],
    ) -> Versus {
        Versus {
            players: [
                Game::with_initial(rs.clone(), seeds[0], handling[0], Mode::Free, initial[0]),
                Game::with_initial(rs, seeds[1], handling[1], Mode::Free, initial[1]),
            ],
            winner: None,
        }
    }

    pub fn winner(&self) -> Option<usize> {
        self.winner
    }

    pub fn step(&mut self, inputs: [&[InputEvent]; 2]) {
        if self.winner.is_some() {
            return;
        }
        self.players[0].step(inputs[0]);
        self.players[1].step(inputs[1]);
        let out0 = self.players[0].take_outgoing();
        let out1 = self.players[1].take_outgoing();
        for l in out0 {
            self.players[1].receive_garbage(l);
        }
        for l in out1 {
            self.players[0].receive_garbage(l);
        }
        let t0 = self.players[0].topped_out_at();
        let t1 = self.players[1].topped_out_at();
        let loser = match (t0, t1) {
            (None, None) => return,
            (Some(_), None) => 0,
            (None, Some(_)) => 1,
            // both in the same frame: earlier sub-frame loses; then less attack; then player 1
            (Some(a), Some(b)) if a != b => usize::from(a > b),
            _ => {
                let (a0, a1) = (
                    self.players[0].stats().attack,
                    self.players[1].stats().attack,
                );
                if a0 < a1 {
                    0
                } else {
                    1
                }
            }
        };
        let w = 1 - loser;
        self.winner = Some(w);
        self.players[w].finish_external(MatchEndReason::OpponentToppedOut);
    }

    pub fn state_hash(&self) -> u64 {
        let mut h = crate::events::Fnv::default();
        h.u64(self.players[0].state_hash());
        h.u64(self.players[1].state_hash());
        h.u64(self.winner.map_or(9, |w| w as u64));
        h.finish()
    }
}
