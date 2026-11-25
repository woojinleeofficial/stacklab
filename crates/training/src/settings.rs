//! User input settings (spec 3 조작·인터페이스, UI-09): key bindings with duplicate check and
//! handling in TETR.IO units and ranges. Internally handling is stored in engine sub-frames
//! (1 frame = 10 sub-frames, docs/DECISIONS.md D-007), shown to the user in frames (F).

use serde::{Deserialize, Serialize};
use stacklab_engine::game::{Handling, Key};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    Game(Key),
    Restart,
    Pause,
    Hint,
}

impl Action {
    pub const ALL: [Action; 11] = [
        Action::Game(Key::Left),
        Action::Game(Key::Right),
        Action::Game(Key::SoftDrop),
        Action::Game(Key::HardDrop),
        Action::Game(Key::Cw),
        Action::Game(Key::Ccw),
        Action::Game(Key::Flip),
        Action::Game(Key::Hold),
        Action::Restart,
        Action::Pause,
        Action::Hint,
    ];

    /// i18n key suffix.
    pub fn id(self) -> &'static str {
        match self {
            Action::Game(Key::Left) => "left",
            Action::Game(Key::Right) => "right",
            Action::Game(Key::SoftDrop) => "soft_drop",
            Action::Game(Key::HardDrop) => "hard_drop",
            Action::Game(Key::Cw) => "cw",
            Action::Game(Key::Ccw) => "ccw",
            Action::Game(Key::Flip) => "flip",
            Action::Game(Key::Hold) => "hold",
            Action::Restart => "restart",
            Action::Pause => "pause",
            Action::Hint => "hint",
        }
    }
}

/// TETR.IO handling ranges in sub-frames: DAS 1–20 F, ARR 0–5 F, DCD 0–20 F (0.1 F steps),
/// SDF 5–40 X or 41 = infinite (stored as 0 in the engine).
pub const DAS_RANGE: (u32, u32) = (10, 200);
pub const ARR_RANGE: (u32, u32) = (0, 50);
pub const DCD_RANGE: (u32, u32) = (0, 200);
pub const SDF_RANGE: (u32, u32) = (5, 41);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub handling: Handling,
    /// (action, key name as egui spells it, e.g. "ArrowLeft", "Space", "Z"). One action may
    /// have several keys; one key may serve only one action.
    pub bindings: Vec<(Action, String)>,
    #[serde(default)]
    pub perf_overlay: bool,
}

impl Default for Settings {
    fn default() -> Self {
        let b = |a: Action, k: &str| (a, k.to_string());
        Settings {
            handling: Handling::default(),
            bindings: vec![
                b(Action::Game(Key::Left), "ArrowLeft"),
                b(Action::Game(Key::Right), "ArrowRight"),
                b(Action::Game(Key::SoftDrop), "ArrowDown"),
                b(Action::Game(Key::HardDrop), "Space"),
                b(Action::Game(Key::Cw), "ArrowUp"),
                b(Action::Game(Key::Cw), "X"),
                b(Action::Game(Key::Ccw), "Z"),
                b(Action::Game(Key::Flip), "A"),
                b(Action::Game(Key::Hold), "C"),
                b(Action::Restart, "R"),
                b(Action::Pause, "Escape"),
                b(Action::Hint, "H"),
            ],
            perf_overlay: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsError {
    /// The same key is bound to two different actions.
    DuplicateKey {
        key: String,
        actions: (Action, Action),
    },
    /// A game action has no key.
    Unbound(Action),
    OutOfRange(&'static str),
}

impl Settings {
    pub fn validate(&self) -> Result<(), SettingsError> {
        for (i, (a, k)) in self.bindings.iter().enumerate() {
            if let Some((b, _)) = self.bindings[..i].iter().find(|(b, kk)| kk == k && b != a) {
                return Err(SettingsError::DuplicateKey {
                    key: k.clone(),
                    actions: (*b, *a),
                });
            }
        }
        for a in Action::ALL {
            if matches!(a, Action::Game(_)) && !self.bindings.iter().any(|(b, _)| *b == a) {
                return Err(SettingsError::Unbound(a));
            }
        }
        let h = &self.handling;
        let inr = |v: u32, r: (u32, u32)| (r.0..=r.1).contains(&v);
        if !inr(h.das, DAS_RANGE) {
            return Err(SettingsError::OutOfRange("das"));
        }
        if !inr(h.arr, ARR_RANGE) {
            return Err(SettingsError::OutOfRange("arr"));
        }
        if !inr(h.dcd, DCD_RANGE) {
            return Err(SettingsError::OutOfRange("dcd"));
        }
        if h.sdf != 0 && !inr(h.sdf, SDF_RANGE) {
            return Err(SettingsError::OutOfRange("sdf"));
        }
        Ok(())
    }

    /// Binds `key` to `action`, replacing that action's binding at `slot` (0 = first key).
    /// The result is validated; on a duplicate nothing changes.
    pub fn bind(&mut self, action: Action, slot: usize, key: &str) -> Result<(), SettingsError> {
        let mut next = self.clone();
        let idx: Vec<usize> = next
            .bindings
            .iter()
            .enumerate()
            .filter(|(_, (a, _))| *a == action)
            .map(|(i, _)| i)
            .collect();
        match idx.get(slot) {
            Some(&i) => next.bindings[i].1 = key.to_string(),
            None => next.bindings.push((action, key.to_string())),
        }
        next.validate()?;
        *self = next;
        Ok(())
    }

    pub fn keys_for(&self, action: Action) -> Vec<&str> {
        self.bindings
            .iter()
            .filter(|(a, _)| *a == action)
            .map(|(_, k)| k.as_str())
            .collect()
    }

    pub fn action_for(&self, key: &str) -> Option<Action> {
        self.bindings
            .iter()
            .find(|(_, k)| k == key)
            .map(|(a, _)| *a)
    }

    /// SDF as the user sees it: 5..=40 or 41 (= infinite).
    pub fn sdf_display(&self) -> u32 {
        if self.handling.sdf == 0 {
            41
        } else {
            self.handling.sdf
        }
    }

    pub fn set_sdf_display(&mut self, v: u32) {
        self.handling.sdf = if v >= 41 { 0 } else { v.max(5) };
    }
}

/// Frames (0.1 F precision) <-> sub-frames.
pub fn frames(sub: u32) -> f64 {
    sub as f64 / 10.0
}

pub fn sub_frames(f: f64) -> u32 {
    (f * 10.0).round().max(0.0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_tetrio_values() {
        let s = Settings::default();
        assert_eq!(s.validate(), Ok(()));
        assert_eq!(
            (
                frames(s.handling.das),
                frames(s.handling.arr),
                frames(s.handling.dcd)
            ),
            (10.0, 2.0, 1.0)
        );
        assert_eq!(s.sdf_display(), 6);
    }

    #[test]
    fn duplicate_keys_are_rejected_and_nothing_changes() {
        let mut s = Settings::default();
        let before = s.clone();
        let e = s.bind(Action::Game(Key::Hold), 0, "Z").unwrap_err();
        assert!(matches!(e, SettingsError::DuplicateKey { ref key, .. } if key == "Z"));
        assert_eq!(s, before);
        // a second key for the same action is fine
        s.bind(Action::Game(Key::Hold), 1, "Shift").unwrap();
        assert_eq!(s.keys_for(Action::Game(Key::Hold)), vec!["C", "Shift"]);
        assert_eq!(s.action_for("Shift"), Some(Action::Game(Key::Hold)));
    }

    #[test]
    fn ranges_follow_tetrio() {
        let mut s = Settings::default();
        s.handling.das = 5;
        assert_eq!(s.validate(), Err(SettingsError::OutOfRange("das")));
        s.handling.das = 80;
        s.handling.arr = 0;
        s.set_sdf_display(41);
        assert_eq!(s.handling.sdf, 0, "41 = infinite soft drop");
        assert_eq!(s.validate(), Ok(()));
        assert_eq!(sub_frames(1.7), 17);
    }

    #[test]
    fn every_game_action_needs_a_key() {
        let mut s = Settings::default();
        s.bindings.retain(|(a, _)| *a != Action::Game(Key::Flip));
        assert_eq!(
            s.validate(),
            Err(SettingsError::Unbound(Action::Game(Key::Flip)))
        );
    }
}
