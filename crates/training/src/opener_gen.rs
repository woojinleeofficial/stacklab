//! Module E: opener problems (spec 6.5 opener mix, 8.4). Roles:
//! * `primary`  — the visible pieces allow the primary: build one of its valid branches
//!   (+ the finisher spin when the branch has one).
//! * `switch`   — the primary is NOT possible from the visible pieces: recognise it and build
//!   the fallback instead (building the primary fails).
//! * `fallback` — the fallback is possible: execute it.

use crate::gen_common::{assemble, initial, GameSim};
use crate::problem::{Goal, Module, OpenerShape, Problem};
use stacklab_engine::board::Board;
use stacklab_engine::rng::{Bag7, Rng};
use stacklab_engine::{placements, MoveRules, Piece, Ruleset};
use stacklab_opener::feasibility::{opener_visible, BranchSolver};
use stacklab_opener::library::{Branch, Finisher, Kind, Library, OpenerDefinition};
use stacklab_opener::loadout::OpenerLoadout;
use stacklab_solver::pc::{solve as solve_pc, PcConfig};
use std::sync::Arc;

pub const GENERATOR_VERSION: &str = "E-opener-1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenerRole {
    Primary,
    Switch,
    Fallback,
}

impl OpenerRole {
    pub fn tag(self) -> &'static str {
        match self {
            OpenerRole::Primary => "primary",
            OpenerRole::Switch => "switch",
            OpenerRole::Fallback => "fallback",
        }
    }
}

pub fn estimate_difficulty(role: OpenerRole, holds: u32, finisher: bool, pieces: u32) -> f64 {
    let base = match role {
        OpenerRole::Primary => 1350.0,
        OpenerRole::Fallback => 1400.0,
        OpenerRole::Switch => 1550.0,
    };
    base + 50.0 * holds as f64 + if finisher { 60.0 } else { 0.0 } + 20.0 * (pieces as f64 - 6.0)
}

fn rows_of(b: &Board) -> Vec<u16> {
    let h = b.height();
    b.rows[..h].to_vec()
}

fn play_branch(
    sim: GameSim,
    branch: &Branch,
    plan: &[(usize, bool)],
    rules: &MoveRules,
) -> Option<GameSim> {
    let mut cur = sim;
    for &(i, hold) in plan {
        let s = branch.pieces[i];
        let pl = placements(cur.game.board(), s.piece, rules)
            .into_iter()
            .find(|p| p.state.cell_key() == s.cell_key())?;
        cur = cur.play(&pl, hold, rules)?;
    }
    if let Some(Finisher::Spin { piece, spin, lines }) = branch.finisher {
        let use_hold = cur.game.active_piece().map(|a| a.piece) != Some(piece);
        let pl = placements(cur.game.board(), piece, rules)
            .into_iter()
            .find(|p| {
                let mut b = cur.game.board().clone();
                p.spin == spin && stacklab_engine::lock(&mut b, p).lines == lines
            })?;
        cur = cur.play(&pl, use_hold, rules)?;
    }
    Some(cur)
}

struct Built {
    goal: Goal,
    sim: GameSim,
    branch: String,
    finisher: bool,
}

fn build_for(
    def: &OpenerDefinition,
    rs: &Arc<Ruleset>,
    seed: u64,
    queue: &[Piece],
    rules: &MoveRules,
) -> Option<Built> {
    let init = initial(&Board::default(), queue.to_vec());
    let sim = GameSim::new(rs.clone(), seed, &init);
    match def.kind {
        Kind::Memorized => {
            // all branches buildable for this queue are accepted (multi-answer)
            let mut feasible: Vec<(&Branch, Vec<(usize, bool)>)> = Vec::new();
            for b in &def.branches {
                if let Some(plan) = BranchSolver::new(b, rules).plan(queue, true) {
                    feasible.push((b, plan));
                }
            }
            let (wb, plan) = feasible.first()?.clone();
            let n = wb.pieces.len();
            let accepted: Vec<OpenerShape> = feasible
                .iter()
                .filter(|(b, _)| b.pieces.len() == n && b.finisher == wb.finisher)
                .map(|(b, _)| OpenerShape {
                    branch: b.id.clone(),
                    rows: rows_of(&b.target),
                })
                .collect();
            let sim = play_branch(sim, wb, &plan, rules)?;
            let finisher = match wb.finisher {
                Some(Finisher::Spin { piece, spin, lines }) => Some((piece, spin, lines)),
                _ => None,
            };
            Some(Built {
                goal: Goal::OpenerBuild {
                    opener: def.id.clone(),
                    accepted,
                    pieces: n as u32,
                    finisher,
                },
                sim,
                branch: wb.id.clone(),
                finisher: finisher.is_some(),
            })
        }
        Kind::Template => {
            let t = def.template?;
            if let Some(lines) = t.pc {
                let n = t.pieces.unwrap_or(10);
                let sol = solve_pc(
                    &Board::default(),
                    &queue[..queue.len().min(n as usize + 1)],
                    None,
                    rules,
                    &PcConfig {
                        height: lines as usize,
                        hold: true,
                        max_nodes: 300_000,
                    },
                )
                .solution?;
                let mut cur = sim;
                for s in &sol {
                    cur = cur.play(&s.placement, s.used_hold, rules)?;
                }
                Some(Built {
                    goal: Goal::PerfectClear {
                        within_pieces: sol.len() as u32,
                    },
                    sim: cur,
                    branch: def.id.clone(),
                    finisher: false,
                })
            } else {
                let pieces = t.stack_pieces.unwrap_or(7);
                let goal = Goal::Stacking {
                    pieces,
                    max_holes: t.max_holes.unwrap_or(0),
                    max_height: t.max_height.unwrap_or(6),
                    well: None,
                };
                let steps = crate::stacking::solve(&Board::default(), queue, &goal, rules, 48)?;
                let mut cur = sim;
                for (pl, hold) in &steps {
                    cur = cur.play(pl, *hold, rules)?;
                }
                Some(Built {
                    goal,
                    sim: cur,
                    branch: def.id.clone(),
                    finisher: false,
                })
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn generate(
    rs: &Arc<Ruleset>,
    rules: &MoveRules,
    lib: &Library,
    loadout: &OpenerLoadout,
    role: OpenerRole,
    target_d: f64,
    seed: u64,
    time_limit_secs: u32,
) -> Option<Problem> {
    let prim = lib.get(&loadout.primary)?;
    let fb = lib.get(&loadout.fallback)?;
    let mut rng = Rng::new(seed);
    for attempt in 0..80u64 {
        let mut bag = Bag7::new(
            seed.wrapping_mul(31).wrapping_add(attempt),
            &rs.queue.bag_base_order,
        );
        let queue: Vec<Piece> = (0..14).map(|_| bag.next_piece()).collect();
        let visible = &queue[..6];
        let target = match role {
            OpenerRole::Primary => opener_visible(prim, visible, rules).map(|_| prim),
            OpenerRole::Switch => (opener_visible(prim, visible, rules).is_none()
                && opener_visible(fb, visible, rules).is_some())
            .then_some(fb),
            OpenerRole::Fallback => opener_visible(fb, visible, rules).map(|_| fb),
        };
        let Some(def) = target else { continue };
        let Some(built) = build_for(def, rs, seed, &queue, rules) else {
            continue;
        };
        let holds = built.sim.steps.iter().filter(|s| s.use_hold).count() as u32;
        let pieces = built.sim.steps.len() as u32;
        let d = estimate_difficulty(role, holds, built.finisher, pieces);
        // prefer instances near the requested difficulty, but never fail just for that
        if (d - target_d).abs() > 150.0 && attempt < 40 && rng.chance_pm(700) {
            continue;
        }
        let tags = vec![
            format!("opener:{}", def.id),
            format!("role:{}", role.tag()),
            format!("branch:{}", built.branch),
        ];
        let init = initial(&Board::default(), queue.clone());
        return Some(assemble(
            Module::E,
            tags,
            seed,
            rs,
            GENERATOR_VERSION,
            init,
            5,
            built.goal,
            built.sim.witness(),
            d,
            format!("E:{}:{}:h{}", def.id, role.tag(), holds.min(3)),
            time_limit_secs,
        ));
    }
    None
}
