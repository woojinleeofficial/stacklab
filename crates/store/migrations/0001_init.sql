-- StackLab local database, schema version 1 (SQLite, rusqlite bundled)
-- Migration rule: each file NNNN_*.sql runs in one transaction and sets PRAGMA user_version = NNNN.
-- Every table carries schema_version and created_at (spec 10.1).

PRAGMA foreign_keys = ON;

CREATE TABLE ruleset (
    version         TEXT PRIMARY KEY,          -- 'TL-S2'
    toml            TEXT NOT NULL,             -- full ruleset text at registration time
    sha256          TEXT NOT NULL,
    schema_version  INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL
);

CREATE TABLE profile (                          -- single local user, but keep the id explicit
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    display_name    TEXT NOT NULL DEFAULT '',
    handling_json   TEXT NOT NULL,             -- DAS/ARR/SDF/DCD + key bindings
    first_recommendation_event_id TEXT UNIQUE, -- spec 9.3: at most one automatic event
    schema_version  INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL
);

CREATE TABLE skill_rating (
    module          TEXT PRIMARY KEY CHECK (module IN ('A','B','C','D','E','F','G','H')),
    rating          REAL NOT NULL,
    rd              REAL NOT NULL,
    volatility      REAL NOT NULL,
    valid_attempts  INTEGER NOT NULL DEFAULT 0,
    last_played_at  TEXT,
    updated_at      TEXT NOT NULL,
    schema_version  INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL
);

CREATE TABLE match_rating (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    rating          REAL NOT NULL,
    rd              REAL NOT NULL,
    volatility      REAL NOT NULL,
    last_played_at  TEXT,
    updated_at      TEXT NOT NULL,
    schema_version  INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL
);

CREATE TABLE rating_history (                   -- every confirmed rating period, for undo (5.3)
    id              INTEGER PRIMARY KEY,
    kind            TEXT NOT NULL CHECK (kind IN ('skill','opener','match','difficulty','bot')),
    subject         TEXT NOT NULL,             -- module / opener_id / bucket / preset id
    period_ref      TEXT NOT NULL,             -- cycle id or evaluation id
    rating_before   REAL NOT NULL, rd_before REAL NOT NULL, vol_before REAL NOT NULL,
    rating_after    REAL NOT NULL, rd_after  REAL NOT NULL, vol_after  REAL NOT NULL,
    reverted        INTEGER NOT NULL DEFAULT 0,
    schema_version  INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL
);

CREATE TABLE tl_anchor (                        -- spec 6.3, full input history kept
    id              INTEGER PRIMARY KEY,
    glicko          REAL NOT NULL,
    rd              REAL NOT NULL,
    entered_at      TEXT NOT NULL,
    app_rating_at_entry REAL,
    offset          REAL,
    schema_version  INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL
);

CREATE TABLE difficulty_calibration (
    module          TEXT NOT NULL,
    param_bucket    TEXT NOT NULL,
    rating          REAL NOT NULL,
    rd              REAL NOT NULL,
    attempts        INTEGER NOT NULL DEFAULT 0,
    version         INTEGER NOT NULL,
    schema_version  INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL,
    PRIMARY KEY (module, param_bucket, version)
);

CREATE TABLE problem (
    id              TEXT PRIMARY KEY,          -- fingerprint-derived
    module          TEXT NOT NULL,
    subtype_tags    TEXT NOT NULL,             -- JSON array
    seed            INTEGER NOT NULL,
    ruleset_version TEXT NOT NULL REFERENCES ruleset(version),
    generator_version TEXT NOT NULL,
    validation_version TEXT NOT NULL,
    initial_state   BLOB NOT NULL,
    goal            TEXT NOT NULL,             -- JSON
    witness         BLOB NOT NULL,             -- verified input log
    visible_next    INTEGER NOT NULL DEFAULT 5,
    time_limit_secs INTEGER NOT NULL DEFAULT 60,
    difficulty      REAL NOT NULL,
    param_bucket    TEXT NOT NULL,
    fingerprint     TEXT NOT NULL UNIQUE,
    is_anchor       INTEGER NOT NULL DEFAULT 0,
    is_unrated      INTEGER NOT NULL DEFAULT 0, -- low-confidence judgment problems (4, H)
    cached          INTEGER NOT NULL DEFAULT 0, -- pre-validated fallback cache (5.3)
    invalidated     INTEGER NOT NULL DEFAULT 0,
    schema_version  INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL
);
CREATE INDEX problem_cache_idx ON problem(module, cached, difficulty);

CREATE TABLE training_set (
    id              TEXT PRIMARY KEY,
    status          TEXT NOT NULL,             -- state machine 7.1
    checkpoint      BLOB,                      -- resumable state (round / problem boundary)
    schema_version  INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL
);

CREATE TABLE cycle (
    id              TEXT PRIMARY KEY,
    set_id          TEXT NOT NULL REFERENCES training_set(id),
    cycle_index     INTEGER NOT NULL CHECK (cycle_index BETWEEN 1 AND 3),
    solved_count    INTEGER NOT NULL DEFAULT 0 CHECK (solved_count BETWEEN 0 AND 12),
    status          TEXT NOT NULL,
    rated_at        TEXT,                      -- Glicko period closed
    schema_version  INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL,
    UNIQUE (set_id, cycle_index)
);

CREATE TABLE attempt (
    id              INTEGER PRIMARY KEY,
    cycle_id        TEXT REFERENCES cycle(id), -- NULL for independent tests / free practice
    problem_id      TEXT NOT NULL REFERENCES problem(id),
    slot            INTEGER,                   -- 1..12 position in the cycle
    success         INTEGER NOT NULL,
    finesse_success INTEGER,
    use_hint        INTEGER NOT NULL DEFAULT 0,
    use_rewind      INTEGER NOT NULL DEFAULT 0,
    use_answer      INTEGER NOT NULL DEFAULT 0,
    valid_for_mmr   INTEGER NOT NULL,
    counted         INTEGER NOT NULL DEFAULT 0, -- 1 = the attempt that decided this slot
    invalidated     INTEGER NOT NULL DEFAULT 0,
    elapsed_ticks   INTEGER NOT NULL,
    event_log       BLOB NOT NULL,
    ruleset_version TEXT NOT NULL,
    schema_version  INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL
);
-- a slot is counted once: retries never add problems (spec 7.1, 11)
CREATE UNIQUE INDEX attempt_slot_counted ON attempt(cycle_id, slot) WHERE counted = 1;

CREATE TABLE bot_preset (
    id              TEXT NOT NULL,
    calibration_version INTEGER NOT NULL,
    params_json     TEXT NOT NULL,
    rating          REAL NOT NULL,
    rd              REAL NOT NULL,
    is_reference    INTEGER NOT NULL DEFAULT 0,
    schema_version  INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL,
    PRIMARY KEY (id, calibration_version)
);

CREATE TABLE match (
    id              TEXT PRIMARY KEY,
    set_id          TEXT REFERENCES training_set(id),
    type            TEXT NOT NULL CHECK (type IN ('mini','evaluation')),
    bot_preset_id   TEXT NOT NULL,
    bot_rating      REAL NOT NULL,
    calibration_version INTEGER NOT NULL,
    status          TEXT NOT NULL,             -- in_progress / completed / skipped / deferred / closed_unrated / void
    skipped         INTEGER NOT NULL DEFAULT 0,
    result          TEXT,                      -- 'win' / 'loss' (no draws)
    rating_delta    REAL,
    out_of_band     INTEGER NOT NULL DEFAULT 0, -- no preset within 40-60% expected
    ruleset_version TEXT NOT NULL,
    schema_version  INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL
);

CREATE TABLE round (
    id              INTEGER PRIMARY KEY,
    match_id        TEXT NOT NULL REFERENCES match(id),
    round_index     INTEGER NOT NULL,
    result          TEXT CHECK (result IN ('win','loss')),
    void            INTEGER NOT NULL DEFAULT 0, -- AI crash / technical error (11)
    replay          BLOB,
    observations    TEXT,                      -- JSON engine-observed stats (7.4)
    schema_version  INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL
);

CREATE TABLE opener_definition (
    opener_id       TEXT NOT NULL,
    version         INTEGER NOT NULL,
    name            TEXT NOT NULL,
    kind            TEXT NOT NULL CHECK (kind IN ('memorized','template')),
    ruleset_version TEXT NOT NULL,
    requirement_vector TEXT NOT NULL,          -- JSON
    branch_graph    TEXT NOT NULL,             -- JSON
    coverage        TEXT NOT NULL,             -- JSON OpenerCoverage
    schema_version  INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL,
    PRIMARY KEY (opener_id, version)
);

CREATE TABLE opener_profile (                   -- never touched by loadout changes (10.3)
    opener_id       TEXT PRIMARY KEY,
    rating          REAL NOT NULL,
    rd              REAL NOT NULL,
    volatility      REAL NOT NULL,
    attempt_stats   TEXT NOT NULL,             -- JSON
    branch_stats    TEXT NOT NULL,             -- JSON
    last_played_at  TEXT,
    schema_version  INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL
);

CREATE TABLE opener_loadout (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    primary_opener_id  TEXT NOT NULL,
    fallback_opener_id TEXT NOT NULL,
    version         INTEGER NOT NULL,
    updated_at      TEXT NOT NULL,
    schema_version  INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL,
    CHECK (primary_opener_id <> fallback_opener_id)
);

CREATE TABLE opener_history (                   -- role at the time (roleAtTime) is preserved
    id              INTEGER PRIMARY KEY,
    opener_id       TEXT NOT NULL,
    role_at_time    TEXT NOT NULL CHECK (role_at_time IN ('primary','fallback','none')),
    ref_kind        TEXT NOT NULL,             -- attempt / round
    ref_id          TEXT NOT NULL,
    schema_version  INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL
);

CREATE TABLE opener_recommendation (
    event_id        TEXT PRIMARY KEY,
    trigger_evaluation_id TEXT REFERENCES match(id),
    is_automatic    INTEGER NOT NULL,
    status          TEXT NOT NULL CHECK (status IN
                    ('PENDING','ANALYZING','READY','APPLIED','DECLINED','DEFERRED','NO_CHANGE','UNAVAILABLE')),
    score_breakdown TEXT,                      -- JSON
    confidence      TEXT,                      -- JSON (80% intervals)
    source_loadout_version INTEGER NOT NULL,
    schema_version  INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL
);
CREATE UNIQUE INDEX one_automatic_recommendation ON opener_recommendation(is_automatic) WHERE is_automatic = 1;

CREATE TABLE opener_change_history (
    id              INTEGER PRIMARY KEY,
    before_json     TEXT NOT NULL,
    after_json      TEXT NOT NULL,
    reason          TEXT NOT NULL,             -- manual / recommendation / restore
    approval_event_id TEXT UNIQUE,             -- idempotency for duplicate clicks (9.4)
    schema_version  INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL
);

CREATE TABLE sprint_record (                    -- 40-line mode (4.1)
    id              INTEGER PRIMARY KEY,
    finish_ticks    INTEGER,
    pieces          INTEGER NOT NULL,
    inputs          INTEGER NOT NULL,
    replay          BLOB NOT NULL,
    ruleset_version TEXT NOT NULL,
    schema_version  INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL
);

CREATE TABLE diagnostic_log (
    id              INTEGER PRIMARY KEY,
    kind            TEXT NOT NULL,
    detail          TEXT NOT NULL,
    schema_version  INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL
);

PRAGMA user_version = 1;
