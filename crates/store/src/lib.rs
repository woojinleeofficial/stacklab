//! Local profile store (spec 10.1): SQLite with versioned migrations, problems, attempts,
//! and full JSON backup / restore.

use rusqlite::types::{Value, ValueRef};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Map, Value as Json};
use stacklab_training::problem::{Module, Problem};
use stacklab_training::session::AttemptResult;
use std::path::Path;

pub const SCHEMA_VERSION: u32 = 2;

/// (version, SQL). Each runs in its own transaction and must end at that `user_version`.
pub const MIGRATIONS: &[(u32, &str)] = &[
    (1, include_str!("../migrations/0001_init.sql")),
    (2, include_str!("../migrations/0002_kv.sql")),
];

pub const BACKUP_FORMAT: u32 = 1;

#[derive(Debug)]
pub enum StoreError {
    Sql(rusqlite::Error),
    Data(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Sql(e) => write!(f, "database error: {e}"),
            StoreError::Data(e) => write!(f, "data error: {e}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Sql(e)
    }
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// UTC timestamp `YYYY-MM-DDTHH:MM:SSZ` without external crates.
pub fn now_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    format_utc(secs)
}

pub fn format_utc(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

pub struct Store {
    conn: Connection,
}

/// Applies every migration newer than the database's `user_version`, each atomically.
pub fn migrate(conn: &mut Connection, migrations: &[(u32, &str)]) -> Result<u32> {
    let mut current: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for &(v, sql) in migrations {
        if v <= current {
            continue;
        }
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.execute_batch(&format!("PRAGMA user_version = {v};"))?;
        tx.commit()?;
        current = v;
    }
    Ok(current)
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Store> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Store> {
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        migrate(&mut conn, MIGRATIONS)?;
        let s = Store { conn };
        s.ensure_ruleset("TL-S2", stacklab_engine::ruleset::TL_S2_TOML)?;
        Ok(s)
    }

    pub fn schema_version(&self) -> Result<u32> {
        Ok(self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn ensure_ruleset(&self, version: &str, toml: &str) -> Result<()> {
        let h = {
            let mut f = stacklab_engine::events::Fnv::default();
            f.bytes(toml.as_bytes());
            format!("fnv64:{:016x}", f.finish())
        };
        self.conn.execute(
            "INSERT OR IGNORE INTO ruleset(version, toml, sha256, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![version, toml, h, now_utc()],
        )?;
        Ok(())
    }

    // ----- problems --------------------------------------------------------------------

    pub fn save_problem(&self, p: &Problem, cached: bool) -> Result<()> {
        let j = |v: &dyn erased::Ser| v.json();
        self.conn.execute(
            "INSERT OR REPLACE INTO problem(id, module, subtype_tags, seed, ruleset_version, generator_version,
               validation_version, initial_state, goal, witness, visible_next, time_limit_secs, difficulty,
               param_bucket, fingerprint, cached, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
            params![
                p.id,
                p.module.letter().to_string(),
                j(&p.subtype_tags),
                p.seed as i64,
                p.ruleset_version,
                p.generator_version,
                p.validation_version,
                j(&p.initial),
                j(&p.goal),
                j(&p.witness),
                p.visible_next as i64,
                p.time_limit_secs as i64,
                p.difficulty,
                p.param_bucket,
                format!("{:016x}", p.fingerprint),
                cached as i64,
                now_utc(),
            ],
        )?;
        Ok(())
    }

    fn row_to_problem(r: &rusqlite::Row) -> rusqlite::Result<std::result::Result<Problem, String>> {
        let module: String = r.get("module")?;
        let text = |c: &str| -> rusqlite::Result<String> {
            Ok(match r.get_ref(c)? {
                ValueRef::Text(t) | ValueRef::Blob(t) => String::from_utf8_lossy(t).into_owned(),
                _ => String::new(),
            })
        };
        let fp: String = r.get("fingerprint")?;
        let build = || -> std::result::Result<Problem, String> {
            Ok(Problem {
                id: r.get("id").map_err(|e| e.to_string())?,
                module: match module.as_str() {
                    "A" => Module::A,
                    "B" => Module::B,
                    "C" => Module::C,
                    "D" => Module::D,
                    "E" => Module::E,
                    "F" => Module::F,
                    "G" => Module::G,
                    "H" => Module::H,
                    m => return Err(format!("bad module {m}")),
                },
                subtype_tags: serde_json::from_str(
                    &text("subtype_tags").map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?,
                seed: r.get::<_, i64>("seed").map_err(|e| e.to_string())? as u64,
                ruleset_version: r.get("ruleset_version").map_err(|e| e.to_string())?,
                generator_version: r.get("generator_version").map_err(|e| e.to_string())?,
                validation_version: r.get("validation_version").map_err(|e| e.to_string())?,
                initial: serde_json::from_str(&text("initial_state").map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?,
                visible_next: r.get::<_, i64>("visible_next").map_err(|e| e.to_string())? as u8,
                goal: serde_json::from_str(&text("goal").map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?,
                witness: serde_json::from_str(&text("witness").map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?,
                difficulty: r.get("difficulty").map_err(|e| e.to_string())?,
                param_bucket: r.get("param_bucket").map_err(|e| e.to_string())?,
                time_limit_secs: r
                    .get::<_, i64>("time_limit_secs")
                    .map_err(|e| e.to_string())? as u32,
                fingerprint: u64::from_str_radix(&fp, 16).map_err(|e| e.to_string())?,
            })
        };
        Ok(build())
    }

    pub fn load_problem(&self, id: &str) -> Result<Option<Problem>> {
        let r = self
            .conn
            .query_row(
                "SELECT * FROM problem WHERE id = ?1",
                [id],
                Self::row_to_problem,
            )
            .optional()?;
        r.map(|x| x.map_err(StoreError::Data)).transpose()
    }

    /// Pre-validated fallback cache rows for a module (spec 5.3).
    pub fn cached_problems(&self, m: Module) -> Result<Vec<Problem>> {
        let mut st = self.conn.prepare(
            "SELECT * FROM problem WHERE module = ?1 AND cached = 1 AND invalidated = 0 ORDER BY difficulty",
        )?;
        let rows = st.query_map([m.letter().to_string()], Self::row_to_problem)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?.map_err(StoreError::Data)?);
        }
        Ok(out)
    }

    // ----- attempts --------------------------------------------------------------------

    /// Records an attempt. `counted` marks the attempt that decided its cycle slot (retries of the
    /// same slot are stored with counted = false and never add problems, spec 7.1).
    pub fn save_attempt(
        &self,
        cycle_id: Option<&str>,
        slot: Option<u32>,
        counted: bool,
        problem: &Problem,
        r: &AttemptResult,
        event_log_json: &str,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO attempt(cycle_id, problem_id, slot, success, finesse_success, use_hint, use_rewind,
               use_answer, valid_for_mmr, counted, elapsed_ticks, event_log, ruleset_version, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![
                cycle_id,
                problem.id,
                slot.map(|s| s as i64),
                r.success as i64,
                r.finesse_success.map(|f| f as i64),
                r.used_hint as i64,
                r.used_rewind as i64,
                r.used_answer as i64,
                r.valid_for_mmr as i64,
                counted as i64,
                r.elapsed_frames as i64,
                event_log_json,
                problem.ruleset_version,
                now_utc(),
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    // ----- app state (key/value JSON) -------------------------------------------------

    pub fn set_state(&self, key: &str, value: &str) -> Result<()> {
        let now = now_utc();
        self.conn.execute(
            "INSERT INTO app_state(key, value, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![key, value, now],
        )?;
        Ok(())
    }

    pub fn get_state(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM app_state WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    // ----- backup / restore ------------------------------------------------------------

    fn tables(&self) -> Result<Vec<String>> {
        let mut st = self
            .conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY rowid")?;
        let v = st
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    /// Full JSON backup of every table (spec 10.1 / 14.4).
    pub fn backup_json(&self) -> Result<String> {
        let mut tables = Map::new();
        for t in self.tables()? {
            let mut st = self.conn.prepare(&format!("SELECT * FROM \"{t}\""))?;
            let cols: Vec<String> = st.column_names().map(|s| s.to_string()).collect();
            let rows = st.query_map([], |r| {
                let mut o = Map::new();
                for (i, c) in cols.iter().enumerate() {
                    let v = match r.get_ref(i)? {
                        ValueRef::Null => Json::Null,
                        ValueRef::Integer(n) => json!(n),
                        ValueRef::Real(f) => json!(f),
                        ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
                        ValueRef::Blob(b) => json!({ "$blob": hex(b) }),
                    };
                    o.insert(c.clone(), v);
                }
                Ok(Json::Object(o))
            })?;
            let rows: Vec<Json> = rows.collect::<rusqlite::Result<_>>()?;
            tables.insert(t, Json::Array(rows));
        }
        Ok(json!({
            "format": BACKUP_FORMAT,
            "schema_version": self.schema_version()?,
            "created_at": now_utc(),
            "tables": tables,
        })
        .to_string())
    }

    /// Replaces all data with a backup, atomically. Fails without changes on any error.
    pub fn restore_json(&mut self, text: &str) -> Result<()> {
        let v: Json = serde_json::from_str(text).map_err(|e| StoreError::Data(e.to_string()))?;
        let schema = v["schema_version"]
            .as_u64()
            .ok_or_else(|| StoreError::Data("missing schema_version".into()))?
            as u32;
        if schema > self.schema_version()? {
            return Err(StoreError::Data(format!(
                "backup schema {schema} is newer than this app"
            )));
        }
        let tables = v["tables"]
            .as_object()
            .ok_or_else(|| StoreError::Data("missing tables".into()))?
            .clone();
        let names = self.tables()?;
        let tx = self.conn.transaction()?;
        tx.execute_batch("PRAGMA defer_foreign_keys = ON;")?;
        for t in names.iter().rev() {
            tx.execute(&format!("DELETE FROM \"{t}\""), [])?;
        }
        for t in &names {
            let Some(rows) = tables.get(t).and_then(|r| r.as_array()) else {
                continue;
            };
            for row in rows {
                let o = row
                    .as_object()
                    .ok_or_else(|| StoreError::Data("row is not an object".into()))?;
                let cols: Vec<&String> = o.keys().collect();
                let sql = format!(
                    "INSERT INTO \"{t}\" ({}) VALUES ({})",
                    cols.iter()
                        .map(|c| format!("\"{c}\""))
                        .collect::<Vec<_>>()
                        .join(","),
                    (1..=cols.len())
                        .map(|i| format!("?{i}"))
                        .collect::<Vec<_>>()
                        .join(",")
                );
                let vals: Vec<Value> = cols
                    .iter()
                    .map(|c| match &o[*c] {
                        Json::Null => Ok(Value::Null),
                        Json::Bool(b) => Ok(Value::Integer(*b as i64)),
                        Json::Number(n) => Ok(n
                            .as_i64()
                            .map(Value::Integer)
                            .unwrap_or_else(|| Value::Real(n.as_f64().unwrap_or(0.0)))),
                        Json::String(s) => Ok(Value::Text(s.clone())),
                        Json::Object(b) => b
                            .get("$blob")
                            .and_then(|h| h.as_str())
                            .and_then(unhex)
                            .map(Value::Blob)
                            .ok_or_else(|| StoreError::Data("bad blob".into())),
                        Json::Array(_) => Err(StoreError::Data("unexpected array".into())),
                    })
                    .collect::<Result<_>>()?;
                tx.execute(&sql, rusqlite::params_from_iter(vals))?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

mod erased {
    pub trait Ser {
        fn json(&self) -> String;
    }
    impl<T: serde::Serialize> Ser for T {
        fn json(&self) -> String {
            serde_json::to_string(self).expect("serialisable")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_format() {
        assert_eq!(format_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_utc(1_795_000_000), "2026-11-18T11:06:40Z");
    }
}
