-- Command triggers and access profiles now live in the YAML config.
-- Only secret values stay in the database, keyed by command and secret name.
-- Drop children before parents (foreign keys are enforced).
DROP TABLE trigger_headers;
DROP TABLE trigger_secrets;
DROP TABLE command_triggers;
DROP TABLE access_profiles;

CREATE TABLE trigger_secrets (
    command    TEXT NOT NULL,              -- lowercase; matches a key in config `triggers`
    name       TEXT NOT NULL,              -- matches a name in that trigger's `secrets`
    ciphertext TEXT NOT NULL,              -- AES-256-GCM hex, info=b"botrix:trigger-secret:v1"
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (command, name)
);
