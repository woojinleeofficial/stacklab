//! One training set (spec 7.1): DIAGNOSIS → TRAINING_1(12) → MINI_1 → TRAINING_2(12) → MINI_2 →
//! TRAINING_3(12) → EVALUATION_BO3 → RESULT → TRAINING_1, plus the match rating (spec 6.1/7.4),
//! TETR.IO anchor (6.3), opponent selection (7.2) and engine-observed weaknesses (7.4).

use crate::glicko::{expected, Outcome, Rating};
use crate::policy::Policy;
use crate::problem::Module;
use serde::{Deserialize, Serialize};
use stacklab_engine::board::Board;
use stacklab_engine::events::EventKind;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Diagnosis,
    Training(u8),
    Mini(u8),
    Evaluation,
    Result,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MiniStatus {
    Pending,
    Won,
    Lost,
    Skipped,
    Deferred,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoundRecord {
    pub won: bool,
    /// AI crash / technical error: the round is replayed and never counted (spec 11).
    pub void: bool,
    pub bot: String,
    pub bot_rating: f64,
    pub bot_rd: f64,
    pub calibration_version: u32,
    pub observations: Observations,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetState {
    pub number: u32,
    pub phase: Phase,
    pub minis: [MiniStatus; 2],
    pub eval_rounds: Vec<RoundRecord>,
    pub eval_deferred: bool,
    /// "평가 없이 종료": set closed, match rating untouched.
    pub closed_unrated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetError {
    WrongPhase(Phase),
}

impl SetState {
    pub fn new_first() -> SetState {
        SetState {
            number: 1,
            phase: Phase::Diagnosis,
            minis: [MiniStatus::Pending; 2],
            eval_rounds: Vec::new(),
            eval_deferred: false,
            closed_unrated: false,
        }
    }

    pub fn diagnosis_done(&mut self) -> Result<(), SetError> {
        self.expect(Phase::Diagnosis)?;
        self.phase = Phase::Training(1);
        Ok(())
    }

    fn expect(&self, p: Phase) -> Result<(), SetError> {
        if self.phase == p {
            Ok(())
        } else {
            Err(SetError::WrongPhase(self.phase))
        }
    }

    /// A 12-problem cycle finished (matches only become available after all 12, spec 14.2).
    pub fn cycle_done(&mut self) -> Result<(), SetError> {
        match self.phase {
            Phase::Training(k @ (1 | 2)) => self.phase = Phase::Mini(k),
            Phase::Training(3) => self.phase = Phase::Evaluation,
            p => return Err(SetError::WrongPhase(p)),
        }
        Ok(())
    }

    fn mini_index(&self) -> Result<usize, SetError> {
        match self.phase {
            Phase::Mini(k) => Ok(k as usize - 1),
            p => Err(SetError::WrongPhase(p)),
        }
    }

    fn after_mini(&mut self) {
        if let Phase::Mini(k) = self.phase {
            self.phase = Phase::Training(k + 1);
        }
    }

    /// Single-round mini match finished (no match-rating change, spec 7.4).
    pub fn mini_result(&mut self, won: bool) -> Result<(), SetError> {
        let i = self.mini_index()?;
        self.minis[i] = if won {
            MiniStatus::Won
        } else {
            MiniStatus::Lost
        };
        self.after_mini();
        Ok(())
    }

    pub fn skip_mini(&mut self) -> Result<(), SetError> {
        let i = self.mini_index()?;
        self.minis[i] = MiniStatus::Skipped;
        self.after_mini();
        Ok(())
    }

    /// Postpone a mini match: training continues; it can still be played before the evaluation.
    pub fn defer_mini(&mut self) -> Result<(), SetError> {
        let i = self.mini_index()?;
        self.minis[i] = MiniStatus::Deferred;
        self.after_mini();
        Ok(())
    }

    /// Plays a deferred mini later (any phase before the result).
    pub fn deferred_mini_result(&mut self, which: usize, won: bool) -> bool {
        if self.minis.get(which) == Some(&MiniStatus::Deferred) && self.phase != Phase::Result {
            self.minis[which] = if won {
                MiniStatus::Won
            } else {
                MiniStatus::Lost
            };
            true
        } else {
            false
        }
    }

    pub fn score(&self) -> (u32, u32) {
        let valid = self.eval_rounds.iter().filter(|r| !r.void);
        valid.fold(
            (0, 0),
            |(w, l), r| if r.won { (w + 1, l) } else { (w, l + 1) },
        )
    }

    /// Records an evaluation round. BO3: first to 2 wins; void rounds do not count (spec 7.2, 11).
    pub fn eval_round(&mut self, r: RoundRecord) -> Result<bool, SetError> {
        self.expect(Phase::Evaluation)?;
        self.eval_deferred = false;
        self.eval_rounds.push(r);
        let (w, l) = self.score();
        if w == 2 || l == 2 {
            self.phase = Phase::Result;
            return Ok(true);
        }
        Ok(false)
    }

    /// Save progress and leave; recommendation decisions wait (spec 11 "평가전 연기").
    pub fn defer_evaluation(&mut self) -> Result<(), SetError> {
        self.expect(Phase::Evaluation)?;
        self.eval_deferred = true;
        Ok(())
    }

    /// "평가 없이 종료": close the set, match rating unchanged, never a recommendation trigger.
    pub fn close_without_evaluation(&mut self) -> Result<(), SetError> {
        self.expect(Phase::Evaluation)?;
        self.closed_unrated = true;
        self.phase = Phase::Result;
        Ok(())
    }

    /// Completed normally (2 wins for someone, not closed unrated).
    pub fn evaluation_completed(&self) -> bool {
        let (w, l) = self.score();
        self.phase == Phase::Result && !self.closed_unrated && (w == 2 || l == 2)
    }

    /// Leave the result screen: the next set starts at TRAINING_1.
    pub fn next_set(&self) -> Result<SetState, SetError> {
        self.expect(Phase::Result)?;
        Ok(SetState {
            number: self.number + 1,
            phase: Phase::Training(1),
            minis: [MiniStatus::Pending; 2],
            eval_rounds: Vec::new(),
            eval_deferred: false,
            closed_unrated: false,
        })
    }
}

// ---------------------------------------------------------------------------------------
// Match rating with optional TETRA LEAGUE anchor (spec 6.3)

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TlAnchor {
    pub glicko: f64,
    pub rd: f64,
    pub entered_at: String,
    pub app_rating_at_entry: f64,
    pub offset: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchProfile {
    /// Match rating; on the TL Glicko scale once an anchor exists, else the app's bot scale.
    pub rating: Rating,
    /// Shift from the bots' app-scale ratings to the user's scale (TL when anchored).
    pub offset: f64,
    pub offset_rd: f64,
    pub anchors: Vec<TlAnchor>,
    pub evaluations: u32,
    /// In-use calibration (spec 7.3 step 4): bot ratings adjusted by this user's evaluations,
    /// on the bots' app scale, with an RD floor so they move slowly.
    #[serde(default)]
    pub bot_adjust: BTreeMap<String, Rating>,
    /// Local calibration version: league version * 1000 + in-use adjustments.
    #[serde(default)]
    pub calibration_bumps: u32,
}

/// Uncertainty of the bot scale shift before any anchored evaluation.
pub const OFFSET_PRIOR_RD: f64 = 200.0;
/// The offset never becomes more certain than this (moves slowly, like the bot RD floor).
pub const OFFSET_RD_FLOOR: f64 = 40.0;

impl MatchProfile {
    pub fn new(policy: &Policy) -> MatchProfile {
        MatchProfile {
            rating: Rating::new(
                policy.rating.initial_rating,
                policy.rating.initial_rd,
                policy.rating.initial_volatility,
            ),
            offset: 0.0,
            offset_rd: 0.0,
            anchors: Vec::new(),
            evaluations: 0,
            bot_adjust: BTreeMap::new(),
            calibration_bumps: 0,
        }
    }

    pub fn anchored(&self) -> bool {
        !self.anchors.is_empty()
    }

    /// Enter a TL Glicko ± RD (first entry or re-entry). All entries are kept.
    pub fn enter_anchor(&mut self, glicko: f64, rd: f64, now: &str, min_rd: f64) {
        let rd_in = rd.max(min_rd);
        let app_at_entry = self.rating.rating;
        if !self.anchored() && self.evaluations == 0 {
            // start the match rating at the TL value (spec 6.3 "초기값")
            self.rating = Rating::new(glicko, rd_in, self.rating.volatility);
            self.offset = 0.0;
            self.offset_rd = OFFSET_PRIOR_RD;
        } else {
            if !self.anchored() {
                self.offset_rd = OFFSET_PRIOR_RD;
            }
            // precision-weighted combination; the same correction moves the bot scale
            let w = self.rating.rd.powi(2) / (self.rating.rd.powi(2) + rd_in.powi(2));
            let diff = glicko - self.rating.rating;
            self.rating.rating += w * diff;
            self.rating.rd = (1.0 / (1.0 / self.rating.rd.powi(2) + 1.0 / rd_in.powi(2))).sqrt();
            self.offset += w * diff;
        }
        self.anchors.push(TlAnchor {
            glicko,
            rd: rd_in,
            entered_at: now.to_string(),
            app_rating_at_entry: app_at_entry,
            offset: self.offset,
        });
    }

    /// Bot rating on the user's scale.
    pub fn bot_on_scale(&self, bot_rating: f64, bot_rd: f64) -> (f64, f64) {
        (
            bot_rating + self.offset,
            (bot_rd.powi(2) + self.offset_rd.powi(2)).sqrt(),
        )
    }

    /// Glicko-2 update over the non-void rounds of a completed evaluation (one rating period).
    /// When anchored, the bot-scale offset is learned from the same rounds.
    pub fn rate_evaluation(&mut self, rounds: &[RoundRecord], policy: &Policy) -> (Rating, Rating) {
        let before = self.rating;
        let games: Vec<Outcome> = rounds
            .iter()
            .filter(|r| !r.void)
            .map(|r| {
                let (opp, opp_rd) = self.bot_on_scale(r.bot_rating, r.bot_rd);
                Outcome {
                    opp_rating: opp,
                    opp_rd,
                    score: if r.won { 1.0 } else { 0.0 },
                }
            })
            .collect();
        if games.is_empty() {
            return (before, before);
        }
        if self.anchored() {
            // the bots, seen as one player at (mean bot + offset), play the user
            let mean_bot = rounds
                .iter()
                .filter(|r| !r.void)
                .map(|r| r.bot_rating)
                .sum::<f64>()
                / games.len() as f64;
            let pseudo = Rating::new(
                mean_bot + self.offset,
                self.offset_rd.max(OFFSET_RD_FLOOR),
                0.06,
            );
            let vs_user: Vec<Outcome> = games
                .iter()
                .map(|g| Outcome {
                    opp_rating: before.rating,
                    opp_rd: before.rd,
                    score: 1.0 - g.score,
                })
                .collect();
            let upd = pseudo.update(
                &vs_user,
                policy.rating.tau,
                OFFSET_RD_FLOOR,
                OFFSET_PRIOR_RD,
            );
            self.offset = upd.rating - mean_bot;
            self.offset_rd = upd.rd;
        }
        self.rating = before.update(
            &games,
            policy.rating.tau,
            policy.rating.rd_min,
            policy.rating.rd_max,
        );
        self.evaluations += 1;
        // in-use bot calibration: each opponent preset is rated against the user's rating
        let mut by_bot: BTreeMap<String, (Rating, Vec<Outcome>)> = BTreeMap::new();
        for r in rounds.iter().filter(|r| !r.void) {
            let cur = self.bot_adjust.get(&r.bot).copied().unwrap_or(Rating::new(
                r.bot_rating,
                r.bot_rd,
                0.06,
            ));
            let user_on_bot_scale = before.rating - self.offset;
            by_bot
                .entry(r.bot.clone())
                .or_insert((cur, Vec::new()))
                .1
                .push(Outcome {
                    opp_rating: user_on_bot_scale,
                    opp_rd: before.rd,
                    score: if r.won { 0.0 } else { 1.0 },
                });
        }
        for (id, (cur, games)) in by_bot {
            let mut upd = cur.update(
                &games,
                policy.rating.tau,
                policy.matches.bot_rd_floor,
                350.0,
            );
            upd.rd = upd.rd.max(policy.matches.bot_rd_floor);
            self.bot_adjust.insert(id, upd);
        }
        self.calibration_bumps += 1;
        (before, self.rating)
    }

    /// Current (league + in-use adjusted) rating of a preset on the bots' app scale.
    pub fn bot_rating(&self, id: &str, league_rating: f64, league_rd: f64) -> (f64, f64) {
        self.bot_adjust
            .get(id)
            .map_or((league_rating, league_rd), |r| (r.rating, r.rd))
    }

    /// calibrationVersion stored with match records.
    pub fn calibration_version(&self, league_version: u32) -> u32 {
        league_version * 1000 + self.calibration_bumps
    }
}

/// Spec 7.2: the preset whose expected win rate (for the user) lies in the band and is closest
/// to 50 %; otherwise the closest overall, flagged as out of band.
pub fn select_opponent(
    m: &MatchProfile,
    bots: &[(String, f64, f64)],
    band: (f64, f64),
) -> Option<(String, f64, bool)> {
    let scored: Vec<(String, f64)> = bots
        .iter()
        .map(|(id, r, rd)| {
            let (opp, opp_rd) = m.bot_on_scale(*r, *rd);
            (id.clone(), expected(m.rating.rating, opp, opp_rd))
        })
        .collect();
    let best = |v: &mut Vec<(String, f64)>| {
        v.sort_by(|a, b| {
            (a.1 - 0.5)
                .abs()
                .partial_cmp(&(b.1 - 0.5).abs())
                .expect("finite")
        });
        v.first().cloned()
    };
    let mut in_band: Vec<(String, f64)> = scored
        .iter()
        .filter(|s| s.1 >= band.0 && s.1 <= band.1)
        .cloned()
        .collect();
    if let Some((id, p)) = best(&mut in_band) {
        return Some((id, p, true));
    }
    let mut all = scored;
    best(&mut all).map(|(id, p)| (id, p, false))
}

// ---------------------------------------------------------------------------------------
// Engine-observed statistics of one round (spec 7.4) and the weaknesses they reveal

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Observations {
    pub pieces: u32,
    pub lines: u32,
    pub attack: u32,
    pub spins: u32,
    pub perfect_clears: u32,
    pub garbage_received: u32,
    pub garbage_cancelled: u32,
    pub max_b2b: i32,
    pub topped_out: bool,
    /// Whether an opener branch of the loadout was completed in the first 14 pieces
    /// (None when no loadout / not observed).
    pub opener_built: Option<bool>,
}

impl Observations {
    pub fn from_events(events: &[EventKind]) -> Observations {
        let mut o = Observations {
            max_b2b: -1,
            ..Default::default()
        };
        for e in events {
            match e {
                EventKind::PieceLock { .. } => o.pieces += 1,
                EventKind::LineClear { lines } => o.lines += lines,
                EventKind::AttackSent { lines } | EventKind::AttackCancelled { lines } => {
                    o.attack += lines
                }
                EventKind::Spin { .. } => o.spins += 1,
                EventKind::PerfectClear => o.perfect_clears += 1,
                EventKind::GarbageReceived { lines, .. } => o.garbage_received += lines,
                EventKind::B2bChange { to, .. } => o.max_b2b = o.max_b2b.max(*to),
                EventKind::TopOut { .. } => o.topped_out = true,
                _ => {}
            }
            if let EventKind::AttackCancelled { lines } = e {
                o.garbage_cancelled += lines;
            }
        }
        o
    }

    /// Weaknesses confirmed by engine statistics only (a lost round alone is not a weakness).
    pub fn weaknesses(&self) -> Vec<(Module, &'static str)> {
        let mut v = Vec::new();
        if self.pieces >= 20 && (self.attack as f64) < 0.25 * self.pieces as f64 {
            v.push((Module::F, "low-attack"));
        }
        if self.garbage_received >= 8 && self.topped_out {
            v.push((Module::G, "garbage-topout"));
        }
        if self.pieces >= 30 && self.spins == 0 {
            v.push((Module::C, "no-spin"));
        }
        if self.opener_built == Some(false) {
            v.push((Module::E, "opener-missed"));
        }
        v
    }
}

/// Did the board match an opener branch shape at any of the first 14 locks?
pub fn opener_built(boards_after_lock: &[Board], shapes: &[Board]) -> bool {
    boards_after_lock
        .iter()
        .take(14)
        .any(|b| shapes.iter().any(|s| s == b))
}

/// Per-module weight boosts from observed weaknesses (added on top of scheduler weights,
/// halved every cycle so old matches fade).
pub fn weakness_boosts(rounds: &[Observations]) -> BTreeMap<Module, f64> {
    let mut out = BTreeMap::new();
    for o in rounds {
        for (m, _) in o.weaknesses() {
            *out.entry(m).or_insert(0.0) += 0.25;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round(won: bool, void: bool) -> RoundRecord {
        RoundRecord {
            won,
            void,
            bot: "L3".into(),
            bot_rating: 1500.0,
            bot_rd: 60.0,
            calibration_version: 1,
            observations: Observations::default(),
        }
    }

    fn at_evaluation() -> SetState {
        let mut s = SetState::new_first();
        s.diagnosis_done().unwrap();
        s.cycle_done().unwrap();
        s.mini_result(true).unwrap();
        s.cycle_done().unwrap();
        s.skip_mini().unwrap();
        s.cycle_done().unwrap();
        s
    }

    #[test]
    fn phases_follow_the_spec_order() {
        let mut s = SetState::new_first();
        assert!(
            s.cycle_done().is_err(),
            "no matches before 12 problems / diagnosis"
        );
        s.diagnosis_done().unwrap();
        assert_eq!(s.phase, Phase::Training(1));
        s.cycle_done().unwrap();
        assert_eq!(s.phase, Phase::Mini(1));
        assert!(s.eval_round(round(true, false)).is_err());
        s.defer_mini().unwrap();
        assert_eq!(s.phase, Phase::Training(2));
        assert!(s.deferred_mini_result(0, false));
        assert_eq!(s.minis[0], MiniStatus::Lost);
        s.cycle_done().unwrap();
        assert_eq!(s.phase, Phase::Mini(2));
        s.skip_mini().unwrap();
        s.cycle_done().unwrap();
        assert_eq!(s.phase, Phase::Evaluation);
    }

    #[test]
    fn bo3_first_to_two_max_three_voids_ignored() {
        let mut s = at_evaluation();
        assert!(!s.eval_round(round(true, false)).unwrap());
        assert!(
            !s.eval_round(round(false, true)).unwrap(),
            "void round replayed, not counted"
        );
        assert!(!s.eval_round(round(false, false)).unwrap());
        assert!(s.eval_round(round(true, false)).unwrap());
        assert_eq!(s.score(), (2, 1));
        assert!(s.evaluation_completed());
        assert!(s.eval_round(round(true, false)).is_err(), "no 4th round");
        let next = s.next_set().unwrap();
        assert_eq!((next.number, next.phase), (2, Phase::Training(1)));
        // 2-0 ends after two rounds
        let mut t = at_evaluation();
        t.eval_round(round(false, false)).unwrap();
        assert!(t.eval_round(round(false, false)).unwrap());
        assert_eq!(t.score(), (0, 2));
    }

    #[test]
    fn defer_and_close_unrated() {
        let mut s = at_evaluation();
        s.defer_evaluation().unwrap();
        assert!(s.eval_deferred && s.phase == Phase::Evaluation);
        s.close_without_evaluation().unwrap();
        assert!(!s.evaluation_completed());
        assert!(s.closed_unrated);
    }

    #[test]
    fn match_rating_moves_only_on_completed_evaluations() {
        let p = Policy::bundled();
        let mut m = MatchProfile::new(&p);
        let before = m.rating;
        m.rate_evaluation(&[round(true, true)], &p);
        assert_eq!(m.rating, before, "void-only: no change");
        m.rate_evaluation(&[round(true, false), round(true, false)], &p);
        assert!(m.rating.rating > before.rating);
        assert!(m.rating.rd < before.rd);
    }

    #[test]
    fn tl_anchor_initial_and_reentry() {
        let p = Policy::bundled();
        let mut m = MatchProfile::new(&p);
        m.enter_anchor(1320.0, 90.0, "t1", p_min(&p));
        assert_eq!(m.rating.rating, 1320.0);
        assert_eq!(m.rating.rd, 150.0, "RD = max(input, 150)");
        assert_eq!(m.anchors.len(), 1);
        assert_eq!(m.anchors[0].app_rating_at_entry, 1500.0);
        // evaluation then a new TL record: the difference is recorded and the offset recomputed
        m.rate_evaluation(&[round(false, false), round(false, false)], &p);
        let r_before = m.rating.rating;
        let off_before = m.offset;
        m.enter_anchor(1450.0, 80.0, "t2", p_min(&p));
        assert_eq!(m.anchors.len(), 2, "history kept");
        assert_eq!(m.anchors[1].app_rating_at_entry, r_before);
        assert!(m.rating.rating > r_before && m.rating.rating < 1450.0);
        assert!(m.offset > off_before, "bots shift with the correction");
        assert_eq!(m.anchors[1].offset, m.offset);
    }

    fn p_min(p: &Policy) -> f64 {
        let _ = p;
        150.0
    }

    #[test]
    fn in_use_calibration_moves_bots_slowly_and_bumps_version() {
        let p = Policy::bundled();
        let mut m = MatchProfile::new(&p);
        m.rating = Rating::new(1500.0, 60.0, 0.06);
        let v0 = m.calibration_version(1);
        m.rate_evaluation(&[round(true, false), round(true, false)], &p);
        let (r, rd) = m.bot_rating("L3", 1500.0, 60.0);
        assert!(r < 1500.0, "bot lost twice -> rated down");
        assert!(r > 1300.0, "but slowly (RD floor)");
        assert!(rd >= p.matches.bot_rd_floor);
        assert_eq!(m.calibration_version(1), v0 + 1);
    }

    #[test]
    fn opponent_selection_prefers_band_closest_to_half() {
        let p = Policy::bundled();
        let mut m = MatchProfile::new(&p);
        m.rating = Rating::new(1500.0, 60.0, 0.06);
        let bots = vec![
            ("A".to_string(), 1200.0, 50.0),
            ("B".to_string(), 1480.0, 50.0),
            ("C".to_string(), 1560.0, 50.0),
            ("D".to_string(), 2000.0, 50.0),
        ];
        let (id, pr, in_band) = select_opponent(&m, &bots, (0.4, 0.6)).unwrap();
        assert_eq!(id, "B");
        assert!(in_band && (0.4..=0.6).contains(&pr));
        let far = vec![
            ("X".to_string(), 2500.0, 50.0),
            ("Y".to_string(), 3000.0, 50.0),
        ];
        let (id, _, in_band) = select_opponent(&m, &far, (0.4, 0.6)).unwrap();
        assert_eq!(id, "X");
        assert!(!in_band, "marked out of band");
    }

    #[test]
    fn weaknesses_need_engine_evidence() {
        let quiet_loss = Observations {
            pieces: 15,
            topped_out: true,
            ..Default::default()
        };
        assert!(
            quiet_loss.weaknesses().is_empty(),
            "losing alone is not a weakness"
        );
        let o = Observations {
            pieces: 60,
            attack: 5,
            garbage_received: 12,
            topped_out: true,
            spins: 0,
            opener_built: Some(false),
            ..Default::default()
        };
        let w: Vec<Module> = o.weaknesses().into_iter().map(|x| x.0).collect();
        assert_eq!(w, vec![Module::F, Module::G, Module::C, Module::E]);
    }
}
