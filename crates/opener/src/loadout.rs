//! Opener loadout (primary + fallback) and history (spec 8.1, 9.5, 10.3). The loadout is a
//! separate record: changing it never touches any opener's proficiency rating or statistics.

use crate::library::{Kind, Library};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenerLoadout {
    pub primary: String,
    pub fallback: String,
    pub version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadoutError {
    SameOpener,
    Unknown(String),
    PrimaryMustBeMemorized(String),
    VersionConflict { expected: u32, actual: u32 },
}

impl std::fmt::Display for LoadoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadoutError::SameOpener => write!(f, "주력과 예외는 서로 달라야 합니다"),
            LoadoutError::Unknown(id) => write!(f, "현재 룰셋에 없는 오프너: {id}"),
            LoadoutError::PrimaryMustBeMemorized(id) => {
                write!(f, "주력은 암기형 오프너만 가능합니다: {id}")
            }
            LoadoutError::VersionConflict { expected, actual } => {
                write!(f, "설정 버전 충돌 (기대 {expected}, 실제 {actual})")
            }
        }
    }
}

impl std::error::Error for LoadoutError {}

/// Validates a primary/fallback pair against the library and ruleset (spec 10.3 invariants).
pub fn validate(
    lib: &Library,
    ruleset: &str,
    primary: &str,
    fallback: &str,
) -> Result<(), LoadoutError> {
    if primary == fallback {
        return Err(LoadoutError::SameOpener);
    }
    let ids = lib.ids(ruleset);
    for id in [primary, fallback] {
        if !ids.iter().any(|x| x == id) {
            return Err(LoadoutError::Unknown(id.to_string()));
        }
    }
    if lib.get(primary).map(|d| d.kind) != Some(Kind::Memorized) {
        return Err(LoadoutError::PrimaryMustBeMemorized(primary.to_string()));
    }
    Ok(())
}

impl OpenerLoadout {
    pub fn new(
        lib: &Library,
        ruleset: &str,
        primary: &str,
        fallback: &str,
    ) -> Result<OpenerLoadout, LoadoutError> {
        validate(lib, ruleset, primary, fallback)?;
        Ok(OpenerLoadout {
            primary: primary.into(),
            fallback: fallback.into(),
            version: 1,
        })
    }

    /// Changes the loadout if `expected_version` matches (optimistic concurrency, spec 9.4/11).
    pub fn change(
        &mut self,
        lib: &Library,
        ruleset: &str,
        primary: &str,
        fallback: &str,
        expected_version: u32,
    ) -> Result<(), LoadoutError> {
        if expected_version != self.version {
            return Err(LoadoutError::VersionConflict {
                expected: expected_version,
                actual: self.version,
            });
        }
        validate(lib, ruleset, primary, fallback)?;
        self.primary = primary.into();
        self.fallback = fallback.into();
        self.version += 1;
        Ok(())
    }

    pub fn role_of(&self, opener: &str) -> Role {
        if opener == self.primary {
            Role::Primary
        } else if opener == self.fallback {
            Role::Fallback
        } else {
            Role::None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    Primary,
    Fallback,
    None,
}

/// History entry: the role an opener had at the time of an attempt/round (never rewritten).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenerHistoryEntry {
    pub opener: String,
    pub role_at_time: Role,
    pub reference: String,
}
