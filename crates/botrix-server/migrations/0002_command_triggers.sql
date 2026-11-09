CREATE TABLE access_profiles (
    id            INTEGER PRIMARY KEY,
    name          TEXT NOT NULL UNIQUE,
    allowed_rooms TEXT NOT NULL DEFAULT '[]',  -- JSON array of room IDs
    allowed_users TEXT NOT NULL DEFAULT '[]',  -- JSON array of MXIDs
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);

CREATE TABLE command_triggers (
    id               INTEGER PRIMARY KEY,
    command          TEXT NOT NULL UNIQUE,     -- stored lowercase; matched case-insensitively
    description      TEXT NOT NULL DEFAULT '',
    enabled          INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0,1)),
    method           TEXT NOT NULL CHECK (method IN ('GET','POST','PUT','PATCH','DELETE')),
    url              TEXT NOT NULL,
    response_template TEXT,                     -- NULL => status-only reply
    access_profile_id INTEGER REFERENCES access_profiles(id) ON DELETE SET NULL,
    allowed_rooms    TEXT NOT NULL DEFAULT '[]',
    allowed_users    TEXT NOT NULL DEFAULT '[]',
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);

CREATE TABLE trigger_headers (
    id         INTEGER PRIMARY KEY,
    trigger_id INTEGER NOT NULL REFERENCES command_triggers(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    value      TEXT NOT NULL,                   -- may contain {{secret:NAME}} placeholders
    UNIQUE(trigger_id, name)
);

CREATE TABLE trigger_secrets (
    id          INTEGER PRIMARY KEY,
    trigger_id  INTEGER NOT NULL REFERENCES command_triggers(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    ciphertext  TEXT NOT NULL,                  -- AES-256-GCM hex, info=b"botrix:trigger-secret:v1"
    updated_at  INTEGER NOT NULL,
    UNIQUE(trigger_id, name)
);

CREATE INDEX idx_trigger_headers_trigger ON trigger_headers(trigger_id);
CREATE INDEX idx_trigger_secrets_trigger ON trigger_secrets(trigger_id);
