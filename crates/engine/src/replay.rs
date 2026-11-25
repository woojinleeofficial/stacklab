//! Replay logs: `initial state + rulesetVersion + seed + input log = same result` (spec 3.2).

use crate::game::{Game, Handling, InitialState, InputEvent, Mode};
use crate::ruleset::{Ruleset, TL_S2_TOML};
use crate::versus::Versus;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const REPLAY_FORMAT: u32 = 1;

/// Rulesets known to this build, by version. Old versions stay here forever (spec 3.1).
pub fn ruleset_by_version(version: &str) -> Option<Arc<Ruleset>> {
    match version {
        "TL-S2" => Some(Arc::new(Ruleset::from_toml(TL_S2_TOML).ok()?)),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayLog {
    pub format: u32,
    pub ruleset_version: String,
    pub seed: u64,
    pub handling: Handling,
    pub mode: Mode,
    pub initial: Option<InitialState>,
    pub frames: u64,
    pub inputs: Vec<InputEvent>,
    pub final_hash: u64,
}

#[derive(Debug)]
pub enum ReplayError {
    UnknownRuleset(String),
    Format(String),
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplayError::UnknownRuleset(v) => write!(f, "unknown ruleset version {v}"),
            ReplayError::Format(e) => write!(f, "replay format error: {e}"),
        }
    }
}

impl std::error::Error for ReplayError {}

/// Feeds inputs frame by frame. Inputs must be sorted by time.
pub(crate) fn frame_slices(inputs: &[InputEvent], spf: u64, frames: u64) -> Vec<(usize, usize)> {
    let mut out = Vec::with_capacity(frames as usize);
    let mut i = 0;
    for f in 0..frames {
        let end_t = (f + 1) * spf;
        let start = i;
        while i < inputs.len() && inputs[i].time < end_t {
            i += 1;
        }
        out.push((start, i));
    }
    out
}

impl ReplayLog {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("replay serialises")
    }

    pub fn from_json(s: &str) -> Result<ReplayLog, ReplayError> {
        serde_json::from_str(s).map_err(|e| ReplayError::Format(e.to_string()))
    }

    /// Re-runs the log and returns the resulting game (`replay(log)` of the logical API).
    pub fn run(&self) -> Result<Game, ReplayError> {
        let rs = ruleset_by_version(&self.ruleset_version)
            .ok_or_else(|| ReplayError::UnknownRuleset(self.ruleset_version.clone()))?;
        let spf = rs.timing.subframes_per_frame as u64;
        let mut g = Game::with_initial(
            rs,
            self.seed,
            self.handling,
            self.mode.clone(),
            self.initial.as_ref(),
        );
        let mut sorted = self.inputs.clone();
        sorted.sort_by_key(|e| e.time);
        for (a, b) in frame_slices(&sorted, spf, self.frames) {
            g.step(&sorted[a..b]);
        }
        Ok(g)
    }

    pub fn verify(&self) -> Result<bool, ReplayError> {
        Ok(self.run()?.state_hash() == self.final_hash)
    }
}

/// Wraps a game and records everything needed to replay it.
#[derive(Debug, Clone)]
pub struct Recorder {
    pub game: Game,
    log: ReplayLog,
}

impl Recorder {
    pub fn new(
        rs: Arc<Ruleset>,
        seed: u64,
        handling: Handling,
        mode: Mode,
        initial: Option<InitialState>,
    ) -> Recorder {
        let log = ReplayLog {
            format: REPLAY_FORMAT,
            ruleset_version: rs.meta.version.clone(),
            seed,
            handling,
            mode: mode.clone(),
            initial: initial.clone(),
            frames: 0,
            inputs: Vec::new(),
            final_hash: 0,
        };
        Recorder {
            game: Game::with_initial(rs, seed, handling, mode, initial.as_ref()),
            log,
        }
    }

    pub fn step(&mut self, inputs: &[InputEvent]) {
        let spf = self.game.ruleset().timing.subframes_per_frame as u64;
        let end = (self.game.tick() + 1) * spf;
        let start = self.game.tick() * spf;
        // keep only what the engine will actually consume, clamped into this frame
        for e in inputs.iter().filter(|e| e.time < end) {
            let mut e = *e;
            e.time = e.time.max(start);
            self.log.inputs.push(e);
        }
        self.game.step(inputs);
        self.log.frames += 1;
    }

    pub fn finish(&self) -> ReplayLog {
        let mut log = self.log.clone();
        log.final_hash = self.game.state_hash();
        log
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersusReplay {
    pub format: u32,
    pub ruleset_version: String,
    pub seeds: [u64; 2],
    pub handling: [Handling; 2],
    #[serde(default)]
    pub initial: [Option<InitialState>; 2],
    pub frames: u64,
    pub inputs: [Vec<InputEvent>; 2],
    pub final_hash: u64,
}

impl VersusReplay {
    /// An empty log for a match that starts now (record frames with [`Self::step`]).
    pub fn start(
        v: &Versus,
        seeds: [u64; 2],
        handling: [Handling; 2],
        initial: [Option<InitialState>; 2],
    ) -> VersusReplay {
        VersusReplay {
            format: REPLAY_FORMAT,
            ruleset_version: v.players[0].ruleset().meta.version.clone(),
            seeds,
            handling,
            initial,
            frames: 0,
            inputs: [Vec::new(), Vec::new()],
            final_hash: v.state_hash(),
        }
    }

    /// Steps `v` one frame and records the inputs the engine consumes (same clamping as
    /// [`Recorder::step`]).
    pub fn step(&mut self, v: &mut Versus, inputs: [&[InputEvent]; 2]) {
        let spf = v.players[0].ruleset().timing.subframes_per_frame as u64;
        for (i, ins) in inputs.iter().enumerate() {
            let start = v.players[i].tick() * spf;
            let end = start + spf;
            for e in ins.iter().filter(|e| e.time < end) {
                let mut e = *e;
                e.time = e.time.max(start);
                self.inputs[i].push(e);
            }
        }
        v.step(inputs);
        self.frames += 1;
        self.final_hash = v.state_hash();
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("replay serialises")
    }

    pub fn from_json(s: &str) -> Result<VersusReplay, serde_json::Error> {
        serde_json::from_str(s)
    }

    /// Re-simulates and compares the final state hash.
    pub fn verify(&self) -> Result<bool, ReplayError> {
        Ok(self.run()?.state_hash() == self.final_hash)
    }

    pub fn run(&self) -> Result<Versus, ReplayError> {
        let rs = ruleset_by_version(&self.ruleset_version)
            .ok_or_else(|| ReplayError::UnknownRuleset(self.ruleset_version.clone()))?;
        let spf = rs.timing.subframes_per_frame as u64;
        let mut v = Versus::with_initial(
            rs,
            self.seeds,
            self.handling,
            [self.initial[0].as_ref(), self.initial[1].as_ref()],
        );
        let mut a = self.inputs[0].clone();
        let mut b = self.inputs[1].clone();
        a.sort_by_key(|e| e.time);
        b.sort_by_key(|e| e.time);
        let sa = frame_slices(&a, spf, self.frames);
        let sb = frame_slices(&b, spf, self.frames);
        for f in 0..self.frames as usize {
            v.step([&a[sa[f].0..sa[f].1], &b[sb[f].0..sb[f].1]]);
        }
        Ok(v)
    }
}
