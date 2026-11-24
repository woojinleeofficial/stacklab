-- Schema version 2: key-value app state (skill profile, open cycle, settings) as JSON.
CREATE TABLE app_state (
    key             TEXT PRIMARY KEY,
    value           TEXT NOT NULL,
    schema_version  INTEGER NOT NULL DEFAULT 2,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);
