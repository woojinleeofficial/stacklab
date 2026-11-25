//! Matches, rounds (with replays), 40-line sprint records and the atomic training checkpoint
//! (spec 10.1 `Match` / `Round`, 11 "문제 중간 종료", 14.4 forced-exit recovery).

use crate::{now_utc, Result, Store};
use rusqlite::{params, OptionalExtension};

#[derive(Debug, Clone, PartialEq)]
pub struct MatchRow {
    pub id: String,
    /// "mini" | "evaluation"
    pub kind: String,
    pub bot_preset_id: String,
    pub bot_rating: f64,
    pub calibration_version: u32,
    pub out_of_band: bool,
    pub ruleset_version: String,
    /// in_progress / completed / skipped / deferred / closed_unrated / void
    pub status: String,
    /// Some(true) win, Some(false) loss
    pub result: Option<bool>,
    pub rating_delta: Option<f64>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RoundRow {
    pub match_id: String,
    pub round_index: u32,
    pub result: Option<bool>,
    pub void: bool,
    /// `VersusReplay` JSON (verifiable with the engine)
    pub replay: Option<String>,
    pub observations: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SprintRow {
    pub id: i64,
    /// None = not finished (top out / abandoned)
    pub finish_ticks: Option<u64>,
    pub pieces: u32,
    pub inputs: u32,
    pub replay: String,
    pub ruleset_version: String,
    pub created_at: String,
}

/// The training checkpoint: flow state plus the problem currently on screen (if any).
#[derive(Debug, Clone, PartialEq)]
pub struct Checkpoint {
    pub training: String,
    pub current_problem: Option<String>,
    /// An evaluation / mini round was in progress when the checkpoint was written.
    pub open_round: Option<String>,
}

fn res_str(r: Option<bool>) -> Option<&'static str> {
    r.map(|w| if w { "win" } else { "loss" })
}

fn res_parse(s: Option<String>) -> Option<bool> {
    s.map(|s| s == "win")
}

impl Store {
    /// Creates the match row if missing (idempotent).
    pub fn begin_match(&self, m: &MatchRow) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO match(id, set_id, type, bot_preset_id, bot_rating, calibration_version, status,
               skipped, result, rating_delta, out_of_band, ruleset_version, created_at)
             VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?8, ?9, ?10, ?11)",
            params![
                m.id,
                m.kind,
                m.bot_preset_id,
                m.bot_rating,
                m.calibration_version as i64,
                m.status,
                res_str(m.result),
                m.rating_delta,
                m.out_of_band as i64,
                m.ruleset_version,
                if m.created_at.is_empty() { now_utc() } else { m.created_at.clone() },
            ],
        )?;
        Ok(())
    }

    pub fn save_round(&self, r: &RoundRow) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO round(match_id, round_index, result, void, replay, observations, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                r.match_id,
                r.round_index as i64,
                res_str(r.result),
                r.void as i64,
                r.replay.as_ref().map(|s| s.as_bytes().to_vec()),
                r.observations,
                now_utc(),
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn finish_match(
        &self,
        id: &str,
        status: &str,
        result: Option<bool>,
        rating_delta: Option<f64>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE match SET status = ?2, result = ?3, rating_delta = ?4, skipped = (?2 = 'skipped') WHERE id = ?1",
            params![id, status, res_str(result), rating_delta],
        )?;
        Ok(())
    }

    fn row_to_match(r: &rusqlite::Row) -> rusqlite::Result<MatchRow> {
        Ok(MatchRow {
            id: r.get("id")?,
            kind: r.get("type")?,
            bot_preset_id: r.get("bot_preset_id")?,
            bot_rating: r.get("bot_rating")?,
            calibration_version: r.get::<_, i64>("calibration_version")? as u32,
            out_of_band: r.get::<_, i64>("out_of_band")? == 1,
            ruleset_version: r.get("ruleset_version")?,
            status: r.get("status")?,
            result: res_parse(r.get("result")?),
            rating_delta: r.get("rating_delta")?,
            created_at: r.get("created_at")?,
        })
    }

    pub fn load_match(&self, id: &str) -> Result<Option<MatchRow>> {
        Ok(self
            .conn
            .query_row(
                "SELECT * FROM match WHERE id = ?1",
                [id],
                Self::row_to_match,
            )
            .optional()?)
    }

    /// Newest first.
    pub fn list_matches(&self, limit: u32) -> Result<Vec<MatchRow>> {
        let mut st = self
            .conn
            .prepare("SELECT * FROM match ORDER BY created_at DESC, rowid DESC LIMIT ?1")?;
        let v = st
            .query_map([limit as i64], Self::row_to_match)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    pub fn rounds(&self, match_id: &str) -> Result<Vec<RoundRow>> {
        let mut st = self
            .conn
            .prepare("SELECT * FROM round WHERE match_id = ?1 ORDER BY id")?;
        let v = st
            .query_map([match_id], |r| {
                let replay: Option<Vec<u8>> = r.get("replay")?;
                Ok(RoundRow {
                    match_id: r.get("match_id")?,
                    round_index: r.get::<_, i64>("round_index")? as u32,
                    result: res_parse(r.get("result")?),
                    void: r.get::<_, i64>("void")? == 1,
                    replay: replay.map(|b| String::from_utf8_lossy(&b).into_owned()),
                    observations: r.get("observations")?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    // ----- 40-line sprint (spec 4.1) ----------------------------------------------------

    pub fn save_sprint(
        &self,
        finish_ticks: Option<u64>,
        pieces: u32,
        inputs: u32,
        replay: &str,
        ruleset_version: &str,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO sprint_record(finish_ticks, pieces, inputs, replay, ruleset_version, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![finish_ticks.map(|t| t as i64), pieces as i64, inputs as i64, replay.as_bytes().to_vec(), ruleset_version, now_utc()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Finished runs fastest first, then unfinished ones newest first.
    pub fn sprint_records(&self, limit: u32) -> Result<Vec<SprintRow>> {
        let mut st = self.conn.prepare(
            "SELECT * FROM sprint_record ORDER BY finish_ticks IS NULL, finish_ticks ASC, id DESC LIMIT ?1",
        )?;
        let v = st
            .query_map([limit as i64], |r| {
                let replay: Vec<u8> = r.get("replay")?;
                Ok(SprintRow {
                    id: r.get("id")?,
                    finish_ticks: r.get::<_, Option<i64>>("finish_ticks")?.map(|t| t as u64),
                    pieces: r.get::<_, i64>("pieces")? as u32,
                    inputs: r.get::<_, i64>("inputs")? as u32,
                    replay: String::from_utf8_lossy(&replay).into_owned(),
                    ruleset_version: r.get("ruleset_version")?,
                    created_at: r.get("created_at")?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    // ----- checkpoint (spec 11, 14.4) ---------------------------------------------------

    /// Writes the whole checkpoint in one transaction: a crash leaves either the previous or
    /// the new checkpoint, never a mix.
    pub fn write_checkpoint(&mut self, c: &Checkpoint) -> Result<()> {
        let tx = self.conn.transaction()?;
        let now = now_utc();
        let put = |k: &str, v: Option<&String>| -> rusqlite::Result<()> {
            match v {
                Some(v) => tx.execute(
                    "INSERT INTO app_state(key, value, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                    params![k, v, now],
                ),
                None => tx.execute("DELETE FROM app_state WHERE key = ?1", [k]),
            }
            .map(|_| ())
        };
        put("training", Some(&c.training))?;
        put("current_problem", c.current_problem.as_ref())?;
        put("open_round", c.open_round.as_ref())?;
        tx.commit()?;
        Ok(())
    }

    pub fn read_checkpoint(&self) -> Result<Option<Checkpoint>> {
        let Some(training) = self.get_state("training")? else {
            return Ok(None);
        };
        Ok(Some(Checkpoint {
            training,
            current_problem: self.get_state("current_problem")?,
            open_round: self.get_state("open_round")?,
        }))
    }
}
