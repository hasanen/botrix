-- Initial schema for Botrix application database.
-- Managed by sqlx migrate; do not edit applied migrations.

CREATE TABLE admins (
    id            INTEGER PRIMARY KEY,
    username      TEXT    NOT NULL UNIQUE,
    password_hash TEXT    NOT NULL,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);

-- Singleton row: session timeout settings.
CREATE TABLE session_settings (
    id                       INTEGER PRIMARY KEY CHECK (id = 1),
    idle_timeout_secs        INTEGER NOT NULL DEFAULT 3600,
    absolute_lifetime_secs   INTEGER NOT NULL DEFAULT 86400,
    updated_at               INTEGER NOT NULL
);

-- Singleton row: Matrix bot session state.
CREATE TABLE matrix_state (
    id                   INTEGER PRIMARY KEY CHECK (id = 1),
    user_id              TEXT,
    homeserver           TEXT,
    access_token         TEXT,    -- AES-256-GCM encrypted, base64-encoded
    refresh_token        TEXT,    -- AES-256-GCM encrypted, base64-encoded
    device_id            TEXT,
    oauth_client_id      TEXT,
    connected_at         INTEGER,
    last_token_refresh   INTEGER,
    status               TEXT NOT NULL DEFAULT 'disconnected'
                         CHECK (status IN ('disconnected', 'connecting', 'connected', 'error'))
);

-- Insert the singleton row so UPDATE statements always find a row.
INSERT INTO matrix_state (id, status) VALUES (1, 'disconnected');

CREATE TABLE webhook_tokens (
    id          INTEGER PRIMARY KEY,
    webhook_id  TEXT    NOT NULL UNIQUE,
    token_hash  TEXT,            -- SHA-256 hex; NULL = no token set
    enabled     INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    created_at  INTEGER,
    updated_at  INTEGER NOT NULL
);

CREATE TABLE notifications (
    id               INTEGER PRIMARY KEY,
    webhook_id       TEXT    NOT NULL,
    destination      TEXT    NOT NULL,
    rendered_message TEXT    NOT NULL,
    status           TEXT    NOT NULL DEFAULT 'pending'
                     CHECK (status IN ('pending', 'processing', 'delivered', 'failed')),
    attempts         INTEGER NOT NULL DEFAULT 0,
    last_error       TEXT,
    created_at       INTEGER NOT NULL,
    delivered_at     INTEGER,
    expires_at       INTEGER NOT NULL
);

CREATE INDEX idx_notifications_status     ON notifications (status);
CREATE INDEX idx_notifications_expires_at ON notifications (expires_at);

-- Browser admin sessions (tower-sessions SqliteSessionStore).
CREATE TABLE sessions (
    id          TEXT    PRIMARY KEY,   -- session Id as decimal i128 string
    data        TEXT    NOT NULL,      -- serde_json-encoded Record
    expiry_date INTEGER NOT NULL       -- Unix timestamp
);
CREATE INDEX idx_sessions_expiry ON sessions (expiry_date);

CREATE TABLE log_events (
    id         INTEGER PRIMARY KEY,
    level      TEXT    NOT NULL,
    component  TEXT    NOT NULL,
    message    TEXT    NOT NULL,
    details    TEXT,
    created_at INTEGER NOT NULL
);

CREATE INDEX idx_log_events_created_at ON log_events (created_at);
