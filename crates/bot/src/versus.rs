//! Simplified 1v1 match loop for STEP 0 strength checks.
//!
//! Uses the real attack table, B2B charging/Surge, combo, garbage travel time, cancelling,
//! opener phase, garbage cap and change-on-attack holes from the ruleset. Timing is per piece
//! (each bot places one piece every 60000/pps_milli frames) instead of per input.

use crate::{Bot, BotConfig};
use stacklab_engine::attack::{on_lock, ChainState, ClearInfo};
use stacklab_engine::board::{Board, WIDTH};
use stacklab_engine::rng::{Bag7, Rng};
use stacklab_engine::{lock, Piece, Ruleset};
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy)]
struct Incoming {
    lines: u32,
    arrive_frame: u64,
}

struct Player {
    bot: Bot,
    board: Board,
    bag: Bag7,
    queue: VecDeque<Piece>,
    hold: Option<Piece>,
    chain: ChainState,
    incoming: VecDeque<Incoming>,
    next_frame: u64,
    frames_per_piece: u64,
    pieces: u32,
    sent: u32,
    garbage_rng: Rng,
    hole: usize,
    alive: bool,
}

impl Player {
    fn new(cfg: BotConfig, rs: &Ruleset, seed: u64, start_offset: u64) -> Player {
        let mut bag = Bag7::new(seed, &rs.queue.bag_base_order);
        let queue = (0..=rs.queue.next_count)
            .map(|_| bag.next_piece())
            .collect();
        let fpp = (rs.timing.frames_per_second as u64 * 1000) / cfg.pps_milli.max(1) as u64;
        Player {
            bot: Bot::new(cfg, rs, seed ^ 0x5bd1e995),
            board: Board::default(),
            bag,
            queue,
            hold: None,
            chain: ChainState::default(),
            incoming: VecDeque::new(),
            next_frame: fpp + start_offset,
            frames_per_piece: fpp,
            pieces: 0,
            sent: 0,
            garbage_rng: Rng::new(seed.wrapping_mul(31).wrapping_add(7)),
            hole: (seed % WIDTH as u64) as usize,
            alive: true,
        }
    }

    fn pending(&self) -> u32 {
        self.incoming.iter().map(|g| g.lines).sum()
    }
}

#[derive(Debug, Clone)]
pub struct MatchResult {
    pub winner: usize,
    pub frames: u64,
    pub pieces: [u32; 2],
    pub sent: [u32; 2],
    pub capped: bool,
}

/// Plays one round until a player tops out.
pub fn play_round(
    a: BotConfig,
    b: BotConfig,
    rs: &Ruleset,
    seed: u64,
    max_frames: u64,
) -> MatchResult {
    let mut p = [Player::new(a, rs, seed, 0), Player::new(b, rs, seed, 1)];
    let next_count = rs.queue.next_count as usize;
    loop {
        let i = if p[0].next_frame <= p[1].next_frame {
            0
        } else {
            1
        };
        let now = p[i].next_frame;
        if now > max_frames {
            // safety cap for the spike only; real rounds end by top-out (margin time)
            let w = if p[0].board.height() <= p[1].board.height() {
                0
            } else {
                1
            };
            return MatchResult {
                winner: w,
                frames: now,
                pieces: [p[0].pieces, p[1].pieces],
                sent: [p[0].sent, p[1].sent],
                capped: true,
            };
        }
        let me = &mut p[i];
        let current = me.queue[0];
        let next: Vec<Piece> = me.queue.iter().skip(1).take(next_count).copied().collect();
        let pending = me.pending();
        let Some(d) = me
            .bot
            .decide(&me.board, current, me.hold, &next, &me.chain, pending)
        else {
            me.alive = false;
            return MatchResult {
                winner: 1 - i,
                frames: now,
                pieces: [p[0].pieces, p[1].pieces],
                sent: [p[0].sent, p[1].sent],
                capped: false,
            };
        };
        // consume queue
        me.queue.pop_front();
        if d.used_hold {
            if me.hold.is_none() {
                me.queue.pop_front();
            }
            me.hold = Some(current);
        }
        while me.queue.len() < next_count + 1 {
            let np = me.bag.next_piece();
            me.queue.push_back(np);
        }
        let res = lock(&mut me.board, &d.placement);
        let atk = on_lock(
            rs,
            &mut me.chain,
            &ClearInfo {
                piece: d.placement.state.piece,
                lines: res.lines,
                spin: d.placement.spin,
                perfect_clear: res.perfect_clear,
            },
        );
        me.pieces += 1;
        let mult = stacklab_engine::attack::margin_multiplier_milli(rs, now);
        let mut out: u32 = atk
            .packets
            .iter()
            .map(|&l| (l as u64 * mult / 1000) as u32)
            .sum();
        // cancel incoming (opener phase doubles cancelling while sent < pending)
        let opener = rs.opener_phase.enabled && me.pieces <= rs.opener_phase.pieces;
        let factor = if opener && me.sent < pending {
            rs.opener_phase.cancel_multiplier
        } else {
            1
        };
        let mut cancel_power = out * factor;
        while cancel_power > 0 {
            let Some(front) = me.incoming.front_mut() else {
                break;
            };
            let c = front.lines.min(cancel_power);
            front.lines -= c;
            cancel_power -= c;
            if front.lines == 0 {
                me.incoming.pop_front();
            }
        }
        out = cancel_power.div_ceil(factor);
        me.sent += out;
        // garbage enters on a non-clearing lock, capped per piece
        if res.lines == 0 {
            let mut budget = rs.garbage.cap_lines_per_piece;
            while budget > 0 {
                let Some(front) = me.incoming.front_mut() else {
                    break;
                };
                if front.arrive_frame > now {
                    break;
                }
                let n = front.lines.min(budget);
                if me
                    .garbage_rng
                    .chance_pm(rs.garbage.messiness_between_attacks_pm)
                {
                    let mut h = me.garbage_rng.below(WIDTH as u64) as usize;
                    if rs.garbage.hole_change_must_differ && h == me.hole {
                        h = (h + 1 + me.garbage_rng.below(WIDTH as u64 - 1) as usize) % WIDTH;
                    }
                    me.hole = h;
                }
                if !me.board.push_garbage(n as usize, me.hole) {
                    me.alive = false;
                }
                front.lines -= n;
                budget -= n;
                if front.lines == 0 {
                    me.incoming.pop_front();
                }
            }
        }
        me.next_frame += me.frames_per_piece;
        let alive = me.alive;
        if out > 0 {
            let travel = rs.garbage.travel_frames as u64;
            p[1 - i].incoming.push_back(Incoming {
                lines: out,
                arrive_frame: now + travel,
            });
        }
        if !alive {
            return MatchResult {
                winner: 1 - i,
                frames: now,
                pieces: [p[0].pieces, p[1].pieces],
                sent: [p[0].sent, p[1].sent],
                capped: false,
            };
        }
    }
}
