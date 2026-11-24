//! Ruleset data loaded from `rulesets/*.toml`. No rule number lives in code.

use serde::Deserialize;
use std::collections::BTreeMap;

pub type Kick = [i8; 2];

#[derive(Debug, Clone, Deserialize)]
pub struct Ruleset {
    pub meta: Meta,
    pub timing: Timing,
    pub board: BoardCfg,
    pub queue: QueueCfg,
    pub gravity: GravityCfg,
    pub rotation: RotationCfg,
    pub spin: SpinCfg,
    pub attack: AttackCfg,
    pub combo: ComboCfg,
    pub b2b: B2bCfg,
    pub garbage: GarbageCfg,
    pub opener_phase: OpenerPhaseCfg,
    pub margin: MarginCfg,
    pub topout: TopoutCfg,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Meta {
    pub version: String,
    pub name: String,
    pub schema: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Timing {
    pub frames_per_second: u32,
    pub subframes_per_frame: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BoardCfg {
    pub width: u8,
    pub height: u8,
    pub visible_height: u8,
    pub spawn_x: i8,
    pub spawn_y: i8,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QueueCfg {
    pub randomizer: String,
    pub next_count: u8,
    pub hold: bool,
    pub bag_base_order: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GravityCfg {
    pub gravity_micro: u32,
    pub gravity_increase_micro: u32,
    pub lock_delay_frames: u32,
    pub lock_reset_limit: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RotationCfg {
    pub system: String,
    pub jlstz: BTreeMap<String, Vec<Kick>>,
    pub i: BTreeMap<String, Vec<Kick>>,
    pub r180: BTreeMap<String, Vec<Kick>>,
    pub i_uses_r180: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SpinCfg {
    pub mode: String,
    pub t_corner_rule: bool,
    pub t_last_kick_upgrades_to_full: bool,
    pub immobile_dirs: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AttackCfg {
    pub single: u32,
    pub double: u32,
    pub triple: u32,
    pub quad: u32,
    pub tspin_mini_single: u32,
    pub tspin_mini_double: u32,
    pub tspin_single: u32,
    pub tspin_double: u32,
    pub tspin_triple: u32,
    pub mini_triple: u32,
    pub mini_quad: u32,
    pub pc_attack: u32,
    pub pc_b2b_add: u32,
    pub pc_separate_attack: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ComboCfg {
    pub system: String,
    pub quarter_step: u32,
    pub zero_base_thresholds: Vec<u32>,
    pub rounding: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct B2bCfg {
    pub mode: String,
    pub bonus_lines: u32,
    pub surge_start_b2b: u32,
    pub surge_start_lines: u32,
    pub surge_lines_per_level: u32,
    pub surge_segments: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GarbageCfg {
    pub travel_frames: u32,
    pub passthrough: bool,
    pub cap_lines_per_piece: u32,
    pub hole_mode: String,
    pub messiness_within_attack_pm: u32,
    pub messiness_between_attacks_pm: u32,
    pub hole_change_must_differ: bool,
    pub insert_only_on_non_clearing_lock: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenerPhaseCfg {
    pub enabled: bool,
    pub pieces: u32,
    pub cancel_multiplier: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MarginCfg {
    pub enabled: bool,
    pub margin_time_frames: u32,
    pub multiplier_start_milli: u32,
    pub multiplier_increase_milli_per_second: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TopoutCfg {
    pub block_out: bool,
    pub lock_out: bool,
    pub clutch_clear: bool,
    pub garbage_overflow_rows: u8,
}

/// Bundled TL-S2 ruleset text (the file in `rulesets/` is the source of truth).
pub const TL_S2_TOML: &str = include_str!("../../../rulesets/TL-S2.toml");

#[derive(Debug)]
pub enum RulesetError {
    Parse(String),
    Invalid(String),
}

impl std::fmt::Display for RulesetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RulesetError::Parse(s) => write!(f, "ruleset parse error: {s}"),
            RulesetError::Invalid(s) => write!(f, "invalid ruleset: {s}"),
        }
    }
}

impl std::error::Error for RulesetError {}

impl Ruleset {
    pub fn from_toml(text: &str) -> Result<Self, RulesetError> {
        let rs: Ruleset = toml::from_str(text).map_err(|e| RulesetError::Parse(e.to_string()))?;
        rs.validate()?;
        Ok(rs)
    }

    pub fn tl_s2() -> Self {
        Self::from_toml(TL_S2_TOML).expect("bundled TL-S2 ruleset must be valid")
    }

    fn validate(&self) -> Result<(), RulesetError> {
        if self.board.width as usize != crate::board::WIDTH {
            return Err(RulesetError::Invalid(
                "only 10-wide boards are supported".into(),
            ));
        }
        if self.board.height as usize > crate::board::MAX_ROWS {
            return Err(RulesetError::Invalid("board height too large".into()));
        }
        for key in ["0R", "R0", "R2", "2R", "2L", "L2", "L0", "0L"] {
            for (name, t) in [("jlstz", &self.rotation.jlstz), ("i", &self.rotation.i)] {
                match t.get(key) {
                    Some(v) if !v.is_empty() => {}
                    _ => {
                        return Err(RulesetError::Invalid(format!(
                            "missing kick table {name}.{key}"
                        )))
                    }
                }
            }
        }
        for key in ["02", "20", "RL", "LR"] {
            if self.rotation.r180.get(key).is_none_or(|v| v.is_empty()) {
                return Err(RulesetError::Invalid(format!("missing r180.{key}")));
            }
        }
        if self.b2b.surge_segments == 0 {
            return Err(RulesetError::Invalid("surge_segments must be > 0".into()));
        }
        Ok(())
    }
}
