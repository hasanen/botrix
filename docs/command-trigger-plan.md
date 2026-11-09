# Botrix – Dynamic Matrix Command Triggers

> **Status:** In progress.
> **Scope:** Add UI-managed command triggers that let a Matrix user mention the
> bot to invoke an external HTTP webhook and get the response back in the room.

---

## Table of Contents

1. [Problem Statement](#1-problem-statement)
2. [Requirements](#2-requirements)
3. [Background (verified in the codebase)](#3-background-verified-in-the-codebase)
4. [Design Decisions](#4-design-decisions)
5. [Data Model](#5-data-model)
6. [API](#6-api)
7. [Message → Command Flow](#7-message--command-flow)
8. [Security / Threat Mitigations](#8-security--threat-mitigations)
9. [Backward Compatibility / Migration](#9-backward-compatibility--migration)
10. [Dependency Approval](#10-dependency-approval)
11. [Task Breakdown](#11-task-breakdown)
12. [Conventions to Honor](#12-conventions-to-honor)

---

## 1. Problem Statement

Add a UI-managed "command triggers" capability. When a user mentions the bot in
a Matrix room (e.g. `@botrix deploy production`), Botrix matches the first word
after the mention to a configured trigger, calls that trigger's external HTTP
webhook with a fixed JSON payload (command, arguments, metadata), renders the
webhook's JSON response through the existing template engine, and posts the
result back as a reply to the triggering message. Triggers are created, edited,
enabled/disabled and deleted entirely through the Web UI, persisted in SQLite,
with per-trigger access control, encrypted secrets, a built-in `help` command,
and SSRF-hardened outbound HTTP.

This is the inverse of the existing webhook flow (inbound HTTP → Matrix). It is
net-new: Botrix today has no incoming-message handling and no outbound HTTP
client of its own.

---

## 2. Requirements

- **Delivery:** call the webhook, render the JSON response body through the
  existing `${field.path}` template engine, post the rendered text back.
  Non-JSON or failed responses post a short status-aware notice
  ("Command succeeded" / "Command failed"); full detail goes to the Web UI log.
- **Request payload:** fixed JSON shape
  `{ command, arguments, args, raw_message, sender, room_id, event_id }`.
  No request-body templating.
- **Command matching:** case-insensitive; first word after the mention.
- **Mention detection:** identity-based — act only on messages that start by
  mentioning the bot's own account (`m.mentions` user_ids / pill), using the bot
  MXID from `client.user_id()`. The bot ignores its own messages.
- **Unknown command:** log it (so admins can decide whether to add a trigger)
  and reply to the invoking user.
- **Unauthorized:** log it and reply "not authorized".
- **Replies:** sent as `m.in_reply_to` the triggering message.
- **Access control:** per-trigger allowed rooms + allowed users, AND logic,
  empty allowlist = unrestricted for that dimension, checked server-side before
  calling the webhook. `help` lists only commands the requesting user can run in
  the current room. Reusable default access profiles avoid duplicating lists.
- **Security:** encrypt stored secrets at rest; never expose secrets in
  API/UI/logs; replace secrets without revealing them; SSRF protection (block
  loopback/private/link-local, deny redirects, guard DNS rebinding); per-request
  timeout; response size limit; TLS cert validation; no shell/eval.

---

## 3. Background (verified in the codebase)

- **No incoming-message handling exists.** `matrix/worker.rs::spawn_sync` runs
  `client.sync()` but registers no handlers and discards events. Reacting to
  room messages requires `Client::add_event_handler` for
  `OriginalSyncRoomMessageEvent`, registered before/around the sync loop in
  `MatrixHandle`.
- **No direct outbound HTTP client.** `reqwest 0.13.5` is already in
  `Cargo.lock` transitively via `matrix-sdk`, with `rustls` +
  `rustls-platform-verifier` (cert validation on by default), and `ipnet 2.12.2`
  is present for address classification. reqwest exposes per-request `timeout`,
  `redirect(Policy::none())`, and `dns_resolver`/`resolve_to_addrs` for SSRF
  control with zero new crates. Promoting reqwest + ipnet to direct deps is a
  manifest-only change (allowed per the "promoting transitive deps to direct is
  ok" preference; still requires the dependency summary before editing the
  manifest — see §10).
- **Reusable building blocks:**
  - `matrix/crypto.rs` — AES-256-GCM + HKDF-SHA256 `encrypt_token`/
    `decrypt_token`. Add a parallel pair (or generalized function) with a new
    HKDF `info` label (`b"botrix:trigger-secret:v1"`) for domain separation.
  - `webhooks/template.rs::render(template, &serde_json::Value)` — reused
    verbatim for the response-body template (dot/array paths, scalar coercion,
    control-char stripping, NFC, 2048/32768 truncation).
  - `db/` conventions: forward-only migrations (`db/migrate.rs` `MIGRATIONS`
    array + `migrations/NNNN_*.sql`), `now_unix()`, r2d2 pool, manual positional
    row mapping, `ON CONFLICT ... DO UPDATE`, `RETURNING`, `debug_redacting!`
    macro, `test_pool()`.
  - API conventions: per-handler `require_user(&session)` +
    `verify_csrf(&token, &headers)`, `ApiError` enum + `From` impls, DTOs in
    `botrix-common/src/api/`, route registration in `server.rs::build_router`,
    `DefaultBodyLimit::max(API_MAX_BODY_BYTES)`.
  - Matrix reply/mention types (already in locked crates, not yet used):
    `OriginalSyncRoomMessageEvent`, `content.body()`, `content.mentions`
    (`Mentions { user_ids, room }`),
    `RoomMessageEventContent::...make_reply_to(&full_event, ForwardThread::No, AddMentions::Yes)`,
    `ev.into_full_event(room_id)`, `Client::add_event_handler`.
- **Bot identity** comes from `client.user_id()` at runtime (persisted in
  `matrix_state.user_id`), not from config. There is no configured display name;
  mention matching keys off the MXID.

---

## 4. Design Decisions

1. **Triggers are DB-backed, not YAML.** The requirement is explicit:
   UI-managed, hot-reloading, no restart. This differs from the existing YAML
   webhooks/destinations and follows the `webhook_tokens`/`notifications` row
   pattern instead. New tables in a `0002_*.sql` migration.
2. **Hot reload via DB reads, not a config object.** The message handler and API
   read triggers from SQLite on each use (same approach as
   `session::enforce_timeouts` reading `session_settings` every request), so UI
   edits apply immediately. No in-memory cache to invalidate; SQLite reads are
   cheap and already WAL-backed.
3. **Delivery is synchronous with a timeout (hybrid), not queued.** A command
   reply is interactive — the user is waiting. The handler calls the webhook with
   a per-request timeout and posts the reply (rendered body or status notice)
   immediately. The `notifications` retry queue is deliberately NOT reused: a
   stale retried reply minutes later would be confusing, and command execution
   must not be replayed on restart (at-least-once replay could re-trigger
   `deploy`). Failures are logged and reported in-room once. Each incoming
   command is handled in its own spawned task so a slow webhook never blocks the
   sync loop.
4. **Outbound HTTP client = reqwest**, promoted to a direct dependency, shared as
   a single `reqwest::Client` with redirects disabled and a custom `dns_resolver`
   that screens every resolved IP against a denylist (loopback, private,
   link-local, ULA, unspecified, multicast) using `ipnet`/`std::net`. Screening
   at resolution time (not just pre-connect) closes the DNS-rebinding gap because
   reqwest connects to the exact addresses the resolver returns. The webhook
   execution lives behind a trait (`WebhookCaller`) so tests use an in-memory
   fake with no network, mirroring the `MessageSender` boundary.
5. **Secrets:** stored encrypted (AES-GCM, new HKDF info label). Headers are
   templated with `{{secret:NAME}}`; secrets are a per-trigger named map. The API
   never returns secret values — only the set of secret names and whether each is
   set. Updating a secret replaces ciphertext; omitting a secret in an update
   leaves it unchanged; a sentinel clears it. `debug_redacting!` on all
   secret-bearing rows.
6. **Access profiles** are a separate table; a trigger may reference a profile
   for its allowed_rooms/allowed_users, or define its own inline lists.
   Authorization resolves the effective allowlists (non-empty inline lists
   override a referenced profile), then applies AND logic across the room and
   user dimensions.
7. **Mention/command parsing is a pure function** over
   `(body, mentions, bot_mxid)` → `Option<ParsedCommand { command, arguments, args }>`,
   fully unit-testable without Matrix. The handler layer does the Matrix I/O
   (reply, room lookup) around it.

---

## 5. Data Model

Migration `crates/botrix-server/migrations/0002_command_triggers.sql`:

```sql
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
```

`command` is reserved-word checked: `help` cannot be created (built-in).

---

## 6. API

New endpoints under `/api/v1/`, authenticated + CSRF on mutations:

| Method | Path | Description |
|---|---|---|
| GET | `/api/v1/triggers` | List triggers (no secret values; secret names + set-flags only) |
| POST | `/api/v1/triggers` | Create trigger |
| GET | `/api/v1/triggers/{id}` | Get one trigger (headers, secret names, access) |
| PUT | `/api/v1/triggers/{id}` | Update trigger (secrets omitted = unchanged) |
| DELETE | `/api/v1/triggers/{id}` | Delete trigger |
| POST | `/api/v1/triggers/{id}/test` | Dry-run: call webhook with a sample payload, return rendered reply + status (never leaks secret values) |
| GET | `/api/v1/access-profiles` | List profiles |
| POST | `/api/v1/access-profiles` | Create profile |
| PUT | `/api/v1/access-profiles/{id}` | Update profile |
| DELETE | `/api/v1/access-profiles/{id}` | Delete profile (triggers' FK set null) |

DTOs added to `botrix-common/src/api/triggers.rs` (re-exported in `api/mod.rs`),
with a custom `Debug`/serialization that never includes secret values.

---

## 7. Message → Command Flow

1. Receive `m.room.message` event.
2. If sender == bot MXID, ignore.
3. If message does not start by mentioning the bot MXID, ignore.
4. Parse: command = first word (case-insensitive), arguments = raw remainder,
   args = whitespace split.
5. If command == `help`: list triggers visible to sender in this room; reply
   `in_reply_to`.
6. Else if trigger missing or disabled: log unknown command; reply
   "unknown command".
7. Else if not authorized (room AND user allowlists): log unauthorized; reply
   "not authorized".
8. Else build fixed JSON payload + resolve headers/secrets; call webhook
   (SSRF-guarded, timeout, size cap):
   - 2xx JSON + template → render response template → reply.
   - 2xx non-JSON or no template → "Command succeeded" → reply.
   - error/timeout/size → log detail → "Command failed" → reply.

---

## 8. Security / Threat Mitigations

- **SSRF:** custom reqwest `dns_resolver` resolves the host and rejects any
  resolved IP in loopback/private/link-local/ULA/unspecified/multicast ranges;
  redirects disabled (`Policy::none()`); only `http`/`https` schemes; connect +
  total timeouts; response capped (read at most N bytes). Screening resolved
  addresses defeats DNS rebinding.
- **TLS:** rustls with platform verifier (default); no
  `danger_accept_invalid_certs`.
- **Secrets:** AES-256-GCM at rest with domain-separated HKDF label; never
  serialized to API/UI/logs; `{{secret:NAME}}` resolved only at call time into
  header values; unresolved/missing secrets fail the call with a logged error
  (no secret in the log).
- **No shell/eval:** payload and templates are pure data.
- **Authorization** is server-side, before any outbound call, AND logic across
  dimensions; `help` is permission-filtered.
- **Logging:** reuse `db::logs` with a new `Component::Trigger`; log
  unknown/unauthorized/success/failure with command + sender + room, never
  secret values.

---

## 9. Backward Compatibility / Migration

- Additive migration `0002`; no existing tables touched. Existing YAML
  webhooks/destinations/notifications keep working.
- New `Component::Trigger` variant added to the logs component enum.
- reqwest + ipnet promoted to direct deps (no new crates in the tree).
- No config-file schema change required for triggers (they live in the DB). Bot
  identity continues to come from the connected session.

---

## 10. Dependency Approval

Both crates are promoted from transitive to direct dependencies; neither adds a
new crate to the build because both are already present in `Cargo.lock` (pulled
by `matrix-sdk 0.19.1`). This falls under the user's standing
"promoting-transitive-deps-to-direct-is-ok" preference, but the summary is
recorded here per the dependency-review rule.

### `reqwest` — proposed version `0.13.5` (already locked)

- **Purpose:** outbound HTTP client for calling a trigger's external webhook.
  Needed for per-request timeouts, disabling redirects, TLS certificate
  validation, and a custom DNS resolver for SSRF screening.
- **Already in tree:** yes — direct dependency of `matrix-sdk 0.19.1` and
  `oauth2-reqwest`. Promoting it to a direct workspace dependency is a
  manifest-only change (zero new compiled crates).
- **Security findings:** reqwest is one of the most widely used Rust HTTP
  clients, actively maintained by the seanmonstar/hyper ecosystem. No open
  RustSec advisory applies to 0.13.x at the pinned version in the current tree;
  the existing `cargo deny check` job already audits it transitively and passes.
  The build uses the rustls + `rustls-platform-verifier` path (no OpenSSL),
  keeping certificate validation on by default.
- **Maintenance findings:** frequent releases, large contributor base, tracks
  current `hyper 1.x` / `rustls 0.23`. Not archived/deprecated. Runtime is tokio,
  matching the workspace.
- **Source links:** <https://crates.io/crates/reqwest>,
  <https://github.com/seanmonstar/reqwest>,
  <https://rustsec.org/packages/reqwest.html>.
- **Risks / notes:** reqwest's default GAI resolver would not screen addresses;
  the SSRF mitigation depends on installing a custom `dns_resolver`/
  `resolve_to_addrs` that rejects private ranges. This is a code concern, not a
  dependency risk. No `danger_accept_invalid_certs`.
- **Recommendation:** approve promotion to a direct dependency, reusing the
  locked `0.13.5` with the rustls feature set already resolved by matrix-sdk.

### `ipnet` — proposed version `2.12.2` (already locked)

- **Purpose:** classify resolved IP addresses (private/link-local/ULA ranges)
  for the SSRF denylist. Complements `std::net::IpAddr`'s built-in
  `is_loopback`/`is_multicast`/`is_unspecified` checks.
- **Already in tree:** yes — pulled by `hyper-util`. Promotion is manifest-only.
- **Security findings:** small, pure-Rust, no `unsafe`-heavy surface; no
  applicable RustSec advisory. Audited by the existing `cargo deny` job.
- **Maintenance findings:** stable, mature crate with a long release history;
  widely depended upon. Not archived/deprecated.
- **Source links:** <https://crates.io/crates/ipnet>,
  <https://github.com/krisprice/ipnet>,
  <https://rustsec.org/packages/ipnet.html>.
- **Risks / notes:** some private-range checks (`Ipv4Addr::is_private`,
  `is_link_local`) exist in std; `ipnet` is used where std lacks a direct
  predicate (e.g. IPv6 ULA `fc00::/7`, IPv4 CGNAT `100.64.0.0/10`). Keep the
  denylist explicit and unit-tested.
- **Recommendation:** approve promotion to a direct dependency at the locked
  `2.12.2`.

> **Install note:** per the Artifactory-only rule, dependency resolution/install
> must go through `https://oppartifactory.eficode.com`. Since both crates are
> already resolved in `Cargo.lock`, promoting them should not fetch anything new;
> if any fetch is triggered it must use the Artifactory registry, and if that is
> blocked, stop and report.

---

## 11. Task Breakdown

- [x] **Task 1: Create `docs/feature-development.md` and record the dependency
  approval.** Write this plan to `docs/feature-development.md`; produce the
  dependency summary for promoting `reqwest 0.13.5` and `ipnet 2.12.2` (§10). No
  manifest edit yet.
  - Test: n/a (doc). Verify the file renders and matches the agreed scope.
  - Demo: the committed plan doc with the dependency summary section.

- [x] **Task 2: Data model + persistence layer (TDD).** Add
  `migrations/0002_command_triggers.sql`, register it in `db/migrate.rs`. Add row
  structs to `db/models.rs` with `debug_redacting!` on the secret row. Add
  `db/triggers.rs` and `db/access_profiles.rs` with CRUD + list queries following
  the existing pool/params/`query_map`/`RETURNING`/`now_unix` patterns; JSON
  arrays for allowlists via `serde_json`.
  - Test: `test_pool()` — migration applies and is idempotent; CRUD for triggers,
    headers, secrets, profiles; `command` uniqueness; FK cascade on header/secret
    delete; profile delete sets trigger FK null; `Debug` redaction of ciphertext.
  - Demo: `cargo test -p botrix-server` shows new DB tests passing.

- [x] **Task 3: Trigger-secret encryption (TDD).** Add encryption for trigger
  secrets using the AES-256-GCM/HKDF pattern with a new `info` label
  `b"botrix:trigger-secret:v1"`. Typed errors only.
  - Test: round-trip; ciphertext ≠ plaintext; wrong passphrase fails; different
    `info` label than OAuth tokens → different ciphertext for the same input.
  - Demo: `cargo test` shows secret crypto tests passing.

- [x] **Task 4: Command parsing (pure function, TDD).** Add `matrix/command.rs`
  with `parse_command(body, mentions, bot_mxid) -> Option<ParsedCommand>`:
  identity-based mention-at-start, case-insensitive command (first word),
  `arguments` (raw remainder), `args` (whitespace split).
  - Test: mention parses; no mention → None; mention not at start → None;
    case-insensitivity; `arguments` preserves original; `args` splits; bare
    `help` handled.
  - Demo: `cargo test` shows parser tests passing.

- [x] **Task 5: SSRF-guarded outbound webhook caller (TDD).** Promote `reqwest` +
  `ipnet` to direct deps. Add `triggers/webhook.rs`: `WebhookCaller` trait, real
  `ReqwestCaller` (redirects off, connect/total timeouts, response-size cap,
  custom `dns_resolver` rejecting blocked ranges), in-memory `FakeCaller`, fixed
  JSON payload struct, pure `{{secret:NAME}}` resolution.
  - Test: payload serialization; secret substitution + missing-secret error;
    IP-screening predicate rejects each blocked range, accepts a public IP;
    timeout → typed error; oversize response rejected; no secret in any error
    string.
  - Demo: `cargo test` shows caller + SSRF predicate tests passing.

- [x] **Task 6: Authorization + response rendering + help (TDD).** Add
  `triggers/authz.rs` (effective allowlists: non-empty inline overrides profile;
  AND logic; empty dimension = allow) and `triggers/respond.rs` (JSON+template →
  render via `webhooks::template::render`; non-JSON/no template →
  "Command succeeded"; error/timeout → "Command failed"). Add a `help` builder
  listing authorized commands with descriptions.
  - Test: authz matrix (empty/empty, room-only, user-only, both, profile ref, AND
    semantics); response branches; help filtering.
  - Demo: `cargo test` shows authz/respond/help tests passing.

- [x] **Task 7: Incoming-message handler wired into the sync loop.** Register
  `Client::add_event_handler` for `OriginalSyncRoomMessageEvent` on
  connect/activate, attached before syncing. Handler ignores own messages, runs
  `parse_command`, spawns a task doing DB lookup → authz → webhook call → render →
  reply via `make_reply_to(&ev.into_full_event(room_id), ForwardThread::No, AddMentions::Yes)`;
  logs via `db::logs` (`Component::Trigger`). Thread `Pool`, `crypto_passphrase`,
  and a shared `WebhookCaller` into the handler context.
  - Test: handler-core tests with `FakeCaller` + `test_pool()` for each branch
    (unknown, unauthorized, success JSON, success non-JSON, failure), asserting
    reply text and logged rows; wiring covered by a compile/smoke test; live
    dispatch validated manually.
  - Demo: `handle_command` produces correct replies and log rows per branch;
    `cargo build` confirms registration in the sync path.

- [x] **Task 8: REST API for triggers + access profiles (TDD).** DTOs in
  `botrix-common/src/api/triggers.rs`; handlers in `api/triggers.rs` +
  `api/access_profiles.rs` using `require_user` + `verify_csrf` + `ApiError`;
  routes in `server.rs::build_router` inside the authenticated group. `test`
  endpoint uses the shared `WebhookCaller`. List/get never serialize secret
  values; PUT leaves omitted secrets unchanged and supports explicit clear.
  - Test: HTTP integration tests (style of `tests/api_read.rs`): auth 401 without
    session, CSRF on mutations, CRUD round-trips, secrets never in responses,
    reserved `help`/duplicate command rejected, unknown profile ref rejected,
    `test` returns a rendered preview via fake caller.
  - Demo: test-suite shows full CRUD with write-only secrets and a working test
    endpoint.

- [x] **Task 9: Web UI — Triggers and Access Profiles pages.** Add a `/triggers`
  Leptos page (and nav entry) under `AuthLayout`: list with enable/disable
  toggle, create/edit form (command, description, method, URL, response template,
  header rows, secret name + write-only value with set/clear, access via profile
  selector or inline lists), delete with `ConfirmModal`, Test action. Add
  `/access-profiles` page for profile CRUD. Use CSRF-aware `put`/`delete` helpers
  and existing shared components; keep view-model logic in unit-tested helpers.
  - Test: unit tests for view-model helpers (form ↔ DTO mapping, secret
    write-only handling, allowlist parsing); `trunk build` / wasm build succeeds.
  - Demo: in-browser create a trigger with a secret + access profile, toggle, run
    Test, see rendered preview; secret value never shown after save.

- [ ] **Task 10: End-to-end wiring, docs, and hardening.** Confirm full path
  wired (handler registered on connect/activate/restore; API routes live; UI
  reachable; `Component::Trigger` logs in the Logs page). Update `README` /
  `docs/configuration.md`. Review `deny.toml`; run `cargo deny check`.
  - Test: full `cargo test`, `cargo clippy --all-targets --all-features -D
    warnings`, `cargo fmt --check`, `cargo deny check`, wasm build — in the
    devcontainer.
  - Demo: green devcontainer run plus a documented manual end-to-end.

---

## 12. Conventions to Honor

- Typed `thiserror` enums only (no `String` error types in library code); one
  error enum per concern, `#[from]` conversions.
- Business logic in server library modules; handlers stay thin.
- TDD: failing test first, then implementation; assert on typed variants /
  observable behavior.
- Dependencies promoted (not newly introduced), with the recorded summary (§10),
  installed per the Artifactory-only rule.
- All cargo/trunk/test commands run in the Botrix devcontainer.
- Commit in meaningful chunks per the user's git preference.
