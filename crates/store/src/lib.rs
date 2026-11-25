//! Local profile store (spec 10.1): SQLite with versioned migrations, problems, attempts,
//! and full JSON backup / restore.

use rusqlite::types::{Value, ValueRef};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Map, Value as Json};
use stacklab_training::problem::{Module, Problem};
use stacklab_training::session::AttemptResult;
use std::path::Path;

pub const SCHEMA_VERSION: u32 = 3;

/// (version, SQL). Each runs in its own transaction and must end at that `user_version`.
pub const MIGRATIONS: &[(u32, &str)] = &[
    (1, include_str!("../migrations/0001_init.sql")),
    (2, include_str!("../migrations/0002_kv.sql")),
    (3, include_str!("../migrations/0003_recommendation.sql")),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyOutcome {
    Applied(stacklab_opener::loadout::OpenerLoadout),
    /// Same event applied before: nothing changed (duplicate click).
    AlreadyApplied,
    /// The loadout changed since the analysis: a new approval is required (spec 11).
    Conflict {
        current_version: u32,
        event_version: u32,
    },
}

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

    // ----- opener loadout & history (spec 9.4/9.5, 10.3) -------------------------------

    pub fn load_loadout(&self) -> Result<Option<stacklab_opener::loadout::OpenerLoadout>> {
        Ok(self
            .conn
            .query_row("SELECT primary_opener_id, fallback_opener_id, version FROM opener_loadout WHERE id = 1", [], |r| {
                Ok(stacklab_opener::loadout::OpenerLoadout { primary: r.get(0)?, fallback: r.get(1)?, version: r.get::<_, i64>(2)? as u32 })
            })
            .optional()?)
    }

    /// Sets the loadout atomically: version check -> validity (DB CHECK) -> change history ->
    /// loadout row. `expected_version` = None only for the very first loadout. Opener profiles
    /// are never touched.
    pub fn set_loadout(
        &mut self,
        primary: &str,
        fallback: &str,
        expected_version: Option<u32>,
        reason: &str,
        approval_event_id: Option<&str>,
    ) -> Result<stacklab_opener::loadout::OpenerLoadout> {
        let tx = self.conn.transaction()?;
        let current: Option<(String, String, i64)> = tx
            .query_row("SELECT primary_opener_id, fallback_opener_id, version FROM opener_loadout WHERE id = 1", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()?;
        let actual = current.as_ref().map(|c| c.2 as u32);
        if actual != expected_version {
            return Err(StoreError::Data(format!(
                "loadout version conflict: expected {expected_version:?}, actual {actual:?}"
            )));
        }
        let version = actual.unwrap_or(0) + 1;
        let now = now_utc();
        let before = current.as_ref().map_or(
            Json::Null,
            |c| json!({"primary": c.0, "fallback": c.1, "version": c.2}),
        );
        let after = json!({"primary": primary, "fallback": fallback, "version": version});
        tx.execute(
            "INSERT INTO opener_change_history(before_json, after_json, reason, approval_event_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![before.to_string(), after.to_string(), reason, approval_event_id, now],
        )?;
        tx.execute(
            "INSERT INTO opener_loadout(id, primary_opener_id, fallback_opener_id, version, updated_at, created_at) VALUES (1, ?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(id) DO UPDATE SET primary_opener_id = excluded.primary_opener_id, fallback_opener_id = excluded.fallback_opener_id,
               version = excluded.version, updated_at = excluded.updated_at",
            params![primary, fallback, version as i64, now],
        )?;
        tx.commit()?;
        Ok(stacklab_opener::loadout::OpenerLoadout {
            primary: primary.into(),
            fallback: fallback.into(),
            version,
        })
    }

    /// Append-only role history (the role at the time is never rewritten).
    pub fn add_opener_history(
        &self,
        e: &stacklab_opener::loadout::OpenerHistoryEntry,
        ref_kind: &str,
    ) -> Result<()> {
        let role = match e.role_at_time {
            stacklab_opener::loadout::Role::Primary => "primary",
            stacklab_opener::loadout::Role::Fallback => "fallback",
            stacklab_opener::loadout::Role::None => "none",
        };
        self.conn.execute(
            "INSERT INTO opener_history(opener_id, role_at_time, ref_kind, ref_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![e.opener, role, ref_kind, e.reference, now_utc()],
        )?;
        Ok(())
    }

    pub fn opener_history(&self, opener: &str) -> Result<Vec<(String, String)>> {
        let mut st = self.conn.prepare(
            "SELECT role_at_time, ref_id FROM opener_history WHERE opener_id = ?1 ORDER BY id",
        )?;
        let v = st
            .query_map([opener], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    // ----- opener recommendation (spec 9.3 / 9.4) ---------------------------------------

    /// Inserts an event. A second automatic event violates the unique index and fails.
    pub fn create_recommendation(
        &self,
        e: &stacklab_training::recommend::RecommendationEvent,
    ) -> Result<()> {
        let now = now_utc();
        self.conn.execute(
            "INSERT INTO opener_recommendation(event_id, trigger_evaluation_id, is_automatic, status, analysis_json, source_loadout_version, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![
                e.event_id,
                e.trigger_evaluation_id,
                e.automatic as i64,
                e.status.as_str(),
                e.analysis.as_ref().map(|a| serde_json::to_string(a).expect("analysis serialises")),
                e.source_loadout_version as i64,
                now
            ],
        )?;
        Ok(())
    }

    pub fn has_automatic_recommendation(&self) -> Result<bool> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM opener_recommendation WHERE is_automatic = 1",
            [],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    fn row_to_event(
        r: &rusqlite::Row,
    ) -> rusqlite::Result<stacklab_training::recommend::RecommendationEvent> {
        let status: String = r.get("status")?;
        let analysis: Option<String> = r.get("analysis_json")?;
        Ok(stacklab_training::recommend::RecommendationEvent {
            event_id: r.get("event_id")?,
            trigger_evaluation_id: r.get("trigger_evaluation_id")?,
            automatic: r.get::<_, i64>("is_automatic")? == 1,
            status: stacklab_training::recommend::RecStatus::parse(&status)
                .unwrap_or(stacklab_training::recommend::RecStatus::Unavailable),
            analysis: analysis.and_then(|a| serde_json::from_str(&a).ok()),
            source_loadout_version: r.get::<_, i64>("source_loadout_version")? as u32,
        })
    }

    pub fn load_recommendation(
        &self,
        event_id: &str,
    ) -> Result<Option<stacklab_training::recommend::RecommendationEvent>> {
        Ok(self
            .conn
            .query_row(
                "SELECT * FROM opener_recommendation WHERE event_id = ?1",
                [event_id],
                Self::row_to_event,
            )
            .optional()?)
    }

    pub fn list_recommendations(
        &self,
    ) -> Result<Vec<stacklab_training::recommend::RecommendationEvent>> {
        let mut st = self
            .conn
            .prepare("SELECT * FROM opener_recommendation ORDER BY created_at, rowid")?;
        let v = st
            .query_map([], Self::row_to_event)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    /// Saves status + analysis after a state-machine transition done in memory.
    pub fn save_recommendation(
        &self,
        e: &stacklab_training::recommend::RecommendationEvent,
    ) -> Result<()> {
        let prev = self
            .load_recommendation(&e.event_id)?
            .ok_or_else(|| StoreError::Data("unknown recommendation".into()))?;
        if prev.status != e.status && !prev.status.can_go(e.status) {
            return Err(StoreError::Data(format!(
                "illegal transition {} -> {}",
                prev.status.as_str(),
                e.status.as_str()
            )));
        }
        self.conn.execute(
            "UPDATE opener_recommendation SET status = ?2, analysis_json = ?3, updated_at = ?4 WHERE event_id = ?1",
            params![e.event_id, e.status.as_str(), e.analysis.as_ref().map(|a| serde_json::to_string(a).expect("analysis serialises")), now_utc()],
        )?;
        Ok(())
    }

    /// Spec 9.4 step 4, one transaction: loadout version check -> pair validity -> change history
    /// -> loadout -> APPLIED. Idempotent per event id (a repeated click returns AlreadyApplied and
    /// changes nothing). Any failure rolls back everything.
    pub fn apply_recommendation(
        &mut self,
        event_id: &str,
        lib: &stacklab_opener::library::Library,
        ruleset: &str,
    ) -> Result<ApplyOutcome> {
        use stacklab_training::recommend::RecStatus;
        let tx = self.conn.transaction()?;
        let ev = tx
            .query_row(
                "SELECT * FROM opener_recommendation WHERE event_id = ?1",
                [event_id],
                Self::row_to_event,
            )
            .optional()?
            .ok_or_else(|| StoreError::Data("unknown recommendation".into()))?;
        if ev.status == RecStatus::Applied {
            return Ok(ApplyOutcome::AlreadyApplied);
        }
        if !matches!(ev.status, RecStatus::Ready | RecStatus::Deferred) {
            return Err(StoreError::Data(format!(
                "cannot apply from {}",
                ev.status.as_str()
            )));
        }
        let (primary, fallback) = ev
            .recommended()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .ok_or_else(|| StoreError::Data("no recommended pair".into()))?;
        let current: Option<(String, String, i64)> = tx
            .query_row("SELECT primary_opener_id, fallback_opener_id, version FROM opener_loadout WHERE id = 1", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()?;
        let Some(cur) = current else {
            return Err(StoreError::Data("no loadout".into()));
        };
        if cur.2 as u32 != ev.source_loadout_version {
            return Ok(ApplyOutcome::Conflict {
                current_version: cur.2 as u32,
                event_version: ev.source_loadout_version,
            });
        }
        stacklab_opener::loadout::validate(lib, ruleset, &primary, &fallback)
            .map_err(|e| StoreError::Data(e.to_string()))?;
        let now = now_utc();
        let version = cur.2 + 1;
        tx.execute(
            "INSERT INTO opener_change_history(before_json, after_json, reason, approval_event_id, created_at) VALUES (?1, ?2, 'recommendation', ?3, ?4)",
            params![
                json!({"primary": cur.0, "fallback": cur.1, "version": cur.2}).to_string(),
                json!({"primary": primary, "fallback": fallback, "version": version}).to_string(),
                event_id,
                now
            ],
        )?;
        tx.execute(
            "UPDATE opener_loadout SET primary_opener_id = ?1, fallback_opener_id = ?2, version = ?3, updated_at = ?4 WHERE id = 1",
            params![primary, fallback, version, now],
        )?;
        tx.execute("UPDATE opener_recommendation SET status = 'APPLIED', updated_at = ?2 WHERE event_id = ?1", params![event_id, now])?;
        tx.commit()?;
        Ok(ApplyOutcome::Applied(
            stacklab_opener::loadout::OpenerLoadout {
                primary,
                fallback,
                version: version as u32,
            },
        ))
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
