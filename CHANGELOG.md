# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Command triggers: mention the bot in a Matrix room to call an outbound webhook and reply with the result. Triggers are defined under `triggers` in the YAML config.
- Access profiles under `access_profiles` in the config, to control who can run which triggers.
- Triggers page in the web UI that lists the config-defined triggers and sets or clears their secret values, with `GET /api/v1/triggers` and `PUT`/`DELETE /api/v1/triggers/{command}/secrets/{name}`.
- Config validation for triggers, including a check that credential headers such as `Authorization` use a `{{secret:NAME}}` placeholder.
- SSRF-guarded outbound webhook caller.
- Encrypted trigger secrets, using a separate key derivation label.
- Dependabot configuration.

### Changed

- Upgraded `leptos` and `leptos_router` to 0.8, `tower-http` to 0.7, `rand` to 0.10, `constant_time_eq` to 0.4, `gloo-timers` to 0.4, and GitHub Actions dependencies.
- The CI container image now builds only after all other jobs pass.

### Security

- Bumped `vodozemac` to 0.11.2 to fix RUSTSEC-2026-0445 and RUSTSEC-2026-0451.
- Ignored RUSTSEC-2026-0459 until `rusqlite` 0.41 is supported upstream.

## [0.1.0] - 2026-11-05

### Added

- Inbound webhook system with token management.
- Background notification worker with retry, crash recovery and history cleanup.
- Matrix integration: client, sender, token encryption at rest, OAuth login, background sync loop and E2EE bootstrap from the recovery key.
- Web UI (Leptos) with setup, login, dashboard, Matrix, webhooks, logs and settings pages, embedded in the server binary.
- Notification history, logs, app info and room join endpoints.
- `botrix status` and `botrix healthcheck` commands.
- Operational event logging.
- Container image, compose profile and GitHub Pages documentation.

### Changed

- Migrated the app database from sqlx to rusqlite.
- Upgraded `matrix-sdk` to 0.19.

### Fixed

- Session idle and absolute timeouts are now enforced live from settings.
- SQLite switches to WAL once before the pool starts, avoiding startup lock errors.
- Matrix room IDs without a `:server` suffix are accepted.
- Matrix connect failures are surfaced instead of a blank 500.
- Session cookies use `SameSite=Lax` so the Matrix OAuth redirect keeps the session.

### Security

- Login timing is equalized for unknown usernames.
- Secrets are redacted from `Debug` output of credential-bearing types.
- The 64 KiB body limit is enforced on API routes.

[Unreleased]: https://github.com/hasanen/botrix/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/hasanen/botrix/releases/tag/v0.1.0
