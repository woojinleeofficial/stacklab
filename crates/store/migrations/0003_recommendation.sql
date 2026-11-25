-- Schema version 3: recommendation events keep their full analysis; the trigger is the
-- evaluation's set id (matches are not separate rows yet), so the foreign key is dropped.
DROP INDEX IF EXISTS one_automatic_recommendation;
DROP TABLE opener_recommendation;
CREATE TABLE opener_recommendation (
    event_id        TEXT PRIMARY KEY,
    trigger_evaluation_id TEXT,
    is_automatic    INTEGER NOT NULL,
    status          TEXT NOT NULL CHECK (status IN
                    ('PENDING','ANALYZING','READY','APPLIED','DECLINED','DEFERRED','NO_CHANGE','UNAVAILABLE')),
    analysis_json   TEXT,
    source_loadout_version INTEGER NOT NULL,
    schema_version  INTEGER NOT NULL DEFAULT 3,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);
-- at most one automatic recommendation per user (spec 9.3 firstRecommendationEventId)
CREATE UNIQUE INDEX one_automatic_recommendation ON opener_recommendation(is_automatic) WHERE is_automatic = 1;
