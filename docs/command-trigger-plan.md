# Botrix – Dynamic Matrix Command Triggers

> **Status:** In progress. Tasks 1-10 are implemented as originally designed
> (UI-managed, DB-backed). **Revision proposed in
> [§13](#13-revision-yaml-defined-triggers-proposed): define triggers and access
> profiles in YAML (version-controlled, read into memory at startup) and keep
> only headers and secrets in the UI/DB. Pending review; tasks 11-17.**
> **Scope:** Add command triggers that let a Matrix user mention the
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
13. [Revision: YAML-defined triggers (proposed)](#13-revision-yaml-defined-triggers-proposed)

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

> **Decisions 1, 2, 5 and 6 are superseded by [§13](#13-revision-yaml-defined-triggers-proposed)
> if the revision is approved.**

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

- [x] **Task 10: End-to-end wiring, docs, and hardening.** Confirm full path
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

---

## 13. Revision: YAML-defined triggers (proposed)

> **Status:** Proposal, pending review. Nothing below is implemented.
> If approved, it supersedes §4 decisions 1, 2, 5, 6, the `command_triggers` /
> `access_profiles` / `trigger_headers` parts of §5, the CRUD parts of §6, the
> UI in task 9, and the "no config schema change" line in §9.

### 13.1 Why

The original intent was that triggers live in version control. The UI-managed
design (§4.1) made the DB the source of truth, so triggers cannot be reviewed,
diffed, or reproduced from a repository. Webhooks and destinations are already
YAML; triggers should follow the same model, including how the UI shows them.

### 13.2 Principle: everything in YAML except secret values

| Lives in YAML (version-controlled, read at startup) | Lives in the DB (set via the UI) |
|---|---|
| command, description, `enabled`, method, URL, response template | secret **values** only (AES-GCM, unchanged) |
| headers (values may use `{{secret:NAME}}`) | |
| secret **names** | |
| inline `allowed_rooms` / `allowed_users`, `access_profile` reference | |
| the access profiles themselves | |

Secret values never appear in the YAML. Declaring secret names there documents
what a trigger needs, and headers reference them with `{{secret:NAME}}`, so a
header can be committed safely.

### 13.3 Design: no DB seeding

Triggers and profiles are **not** copied into SQLite. They are parsed and
validated with the rest of the config, then held in memory for the process
lifetime.

- `Config` gains `triggers: HashMap<String, TriggerConfig>` (key = command) and
  `access_profiles: HashMap<String, AccessProfileConfig>` (key = profile name).
  Both default to empty, so existing configs keep working.
- The handler reads `ctx.config` (an `Arc<Config>`, already in `AppState`) for
  trigger, profile and header lookup. No cache, no sync step.
- The DB keeps one table, `trigger_secrets`, keyed by `(command, name)`.
- **Hot reload is lost.** Changing a trigger needs a restart, same as webhooks
  and destinations. Setting or clearing a secret in the UI applies immediately
  (read from the DB on each command).
- `enabled` is config-owned. Disable a command by editing the YAML and
  restarting.

Example:

```yaml
access_profiles:
  ops:
    allowed_rooms: ["!ops:example.com"]
    allowed_users: ["@alice:example.com", "@bob:example.com"]

triggers:
  deploy:
    description: Deploy the app
    enabled: true
    method: POST
    url: https://ci.example.com/hook
    headers:
      Authorization: "Bearer {{secret:TOKEN}}"
      X-Source: botrix
    secrets: [TOKEN]               # values are set in the web UI
    response_template: "${status}"
    access_profile: ops            # optional; by name
    allowed_rooms: []              # optional inline override (same rule as §4.6)
    allowed_users: []
```

Access semantics are unchanged from §4.6: a non-empty inline list overrides the
profile's list for that dimension; room and user dimensions are ANDed; empty
means unrestricted.

A declared secret with no stored value makes the command fail with a logged
"secret NAME is not set" (existing behavior for a missing secret) and the usual
"Command failed" reply. The UI shows such secrets as "Not set".

### 13.4 Startup validation (`config/validate.rs`)

Invalid trigger config prevents startup with a descriptive error, like the rest
of the config:

- command: non-empty, no whitespace, not `help`; duplicates rejected after
  lowercasing (keys `Deploy` and `deploy` collide).
- method in `GET|POST|PUT|PATCH|DELETE`.
- URL parses and uses `http` or `https`. (SSRF screening still happens at call
  time on resolved addresses; unchanged.)
- `access_profile`, when set, names an existing profile.
- room IDs start with `!`; user IDs look like `@local:domain`.
- secret names are unique per trigger and use the charset the existing
  `{{secret:NAME}}` parser accepts.
- every `{{secret:NAME}}` in a header value names a declared secret.
- **Guard against committed credentials:** headers whose names are
  conventionally credentials (`Authorization`, `Proxy-Authorization`, `Cookie`,
  `X-Api-Key`) must contain a `{{secret:...}}` placeholder; a literal value is
  rejected. This is a heuristic, not a guarantee; the docs also say never to
  put credentials in the YAML.

### 13.5 Data model

New forward-only migration `0003_triggers_from_config.sql`. There are no
existing triggers to preserve, so nothing is carried over:

```sql
DROP TABLE trigger_headers;
DROP TABLE trigger_secrets;
DROP TABLE command_triggers;
DROP TABLE access_profiles;

CREATE TABLE trigger_secrets (
    command    TEXT NOT NULL,              -- lowercase; matches a key in config triggers
    name       TEXT NOT NULL,              -- matches a name in that trigger's `secrets`
    ciphertext TEXT NOT NULL,              -- AES-256-GCM hex, info=b"botrix:trigger-secret:v1"
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (command, name)
);
```

- **Orphans:** a stored secret whose command or name is no longer in the config
  is ignored by the handler and the UI, and kept in the DB, so re-adding it
  restores the value. It is not deleted at startup, so a typo in the YAML cannot
  destroy stored secrets. Renaming a command or secret in YAML orphans its value.

### 13.6 API

Mirrors the webhooks API (`GET /api/v1/webhooks`, token generate/revoke).
Triggers and profiles are read-only over the API; only secret values are
writable.

| Method | Path | Description |
|---|---|---|
| GET | `/api/v1/triggers` | List config triggers: command, description, enabled, method, URL, header names, access summary, declared secrets with `is_set` flags (never values) |
| PUT | `/api/v1/triggers/{command}/secrets/{name}` | Set or replace a secret value (`{ "value": "..." }`); 404 if the command or secret name is not declared in config |
| DELETE | `/api/v1/triggers/{command}/secrets/{name}` | Clear a secret value; same 404 rule |

Both mutations require session + CSRF. The value must be non-empty and within a
size limit (suggest 4 KiB), and it is write-only: it is never echoed.

**Removed:** `POST /api/v1/triggers`, `GET/PUT/DELETE /api/v1/triggers/{id}`,
`POST /api/v1/triggers/{id}/test`, and all `/api/v1/access-profiles*` routes.
This also removes the dry-run Test action; see open question 1.

`botrix-common/src/api/triggers.rs` shrinks to `TriggerInfo` (list item),
`SecretStatusDto` and `SecretSetRequest`. `TriggerInput`, `TriggerDto`,
`AccessProfileDto/Input`, `HeaderDto` and `SecretInput` are removed.

### 13.7 UI

Only the Triggers page remains, laid out like the **Webhook endpoints** card:
a table with command, description, method, URL, enabled, access, and a
secrets column. Each declared secret shows "Set" or "Not set" with *Set value*
and *Clear* actions (value entered in a modal with a password field; clear goes
through `ConfirmModal`). Nothing else is editable.

Removed: the trigger create/edit page, the access-profiles page and nav entry,
the header editor, the enable/disable toggle, delete, and Test.

### 13.8 Impact on existing code

Mostly deletion. Estimated net lines are strongly negative.

- **Unchanged:** `triggers/webhook.rs`, `reqwest_caller.rs`, `ssrf.rs`,
  `respond.rs`, `matrix/command.rs`, `matrix/crypto.rs`, the Matrix event
  handler wiring.
- **Adapted:** `triggers/authz.rs` and `triggers/help.rs` take the config types
  instead of `db::models::*`; `triggers/handler.rs` looks up from
  `ctx.config` (add `config: Arc<Config>` to `HandlerContext`), reads headers
  from the config and secret values from the DB by command; `server.rs` passes
  the config in and updates routes.
- **Shrunk:** `db/triggers.rs` (about 790 lines) keeps only secret queries;
  `api/triggers.rs` (about 300 lines) becomes list + set/clear secret;
  frontend `pages/triggers.rs` becomes the read-only table.
- **Removed:** `db/access_profiles.rs`, `api/access_profiles.rs`, frontend
  `pages/trigger_edit.rs`, `triggers_form.rs`, `pages/access_profiles.rs`, the
  `CommandTrigger` / `AccessProfile` / `TriggerHeader` row structs.
- **Added:** config types in `botrix-common/src/config.rs`, validation in
  `config/validate.rs`, migration `0003`, docs.

### 13.9 Security notes

- Secret handling is unchanged: AES-256-GCM, same HKDF label, never serialized
  to API, UI or logs, `debug_redacting!` on the row.
- Access rules, URLs and headers are now reviewable in git.
- Removing the API write paths for triggers and profiles reduces the
  authenticated attack surface: a stolen UI session can no longer widen an
  allowlist or repoint a URL. It can only set or clear secret values, and only
  for names declared in config.
- Credentials in YAML are the new risk. Mitigations: no secret-value field in
  the schema, the credential-header guard in §13.4, and a docs warning.

### 13.10 Decisions and open questions

Decided:
- No existing triggers, so the migration drops the old tables and carries
  nothing over.
- Secret names are declared in YAML.
- The UI only lists triggers and sets/clears secret values, like webhooks.
- No UI-created triggers alongside config ones; one source of truth.

Open (defaults assumed above):
1. **Dry-run Test** is removed along with the rest of the extra UI. It was the
   only way to try a webhook without a Matrix message. Keep the API-only
   `POST /api/v1/triggers/{command}/test`? Default: drop it.
2. **Headers moved to YAML**, which differs from your earlier "headers in the
   UI". It follows from naming secrets in YAML: headers hold only
   `{{secret:NAME}}` placeholders, so they are safe to commit and the UI needs
   no header editor. Confirm, or say if you want headers kept in the UI.
3. **Reload without restart** (e.g. SIGHUP) is out of scope; it fits later
   because the config is held in an `Arc`.

### 13.11 Task breakdown (if approved)

Each task: tests first (Arrange, Act, Assert, observable behavior), then
implementation; all cargo/trunk commands in the devcontainer; one commit per
task. No new dependencies.

- [x] **Task 11: Config types and validation (TDD).** Add `TriggerConfig` and
  `AccessProfileConfig` to `botrix-common/src/config.rs` (`#[serde(default)]`
  maps on `Config`). Add the §13.4 rules to `config/validate.rs`.
  - Test: valid config parses; each invalid rule returns its typed
    `ValidationError` variant (`help`, duplicate after lowercasing, bad method,
    bad URL scheme, unknown profile, malformed room/user ID, duplicate secret
    name, header placeholder naming an undeclared secret, literal
    `Authorization` value); a config with no `triggers` key still loads.
  - Demo: `cargo test -p botrix-server config` passes.

> **Note on Tasks 12-14:** migration `0003` drops the tables the handler, authz,
> help and API queried, so these three could not each leave a green build. They
> landed as one commit (backend switch). The old trigger/profile DTOs stay in
> `botrix-common` until Task 15 so the frontend keeps compiling; the old UI
> calls removed routes in the meantime.

- [x] **Task 12: Secret persistence keyed by command (TDD).** Add migration
  `0003` (§13.5) and register it. Rewrite `db/triggers.rs` to secret queries by
  `(command, name)`; delete `db/access_profiles.rs` and the unused row structs.
  - Test: migration applies on a fresh and on a `0002`-migrated DB and leaves
    only `trigger_secrets`; set/replace/clear/list by command; orphan rows
    survive; `Debug` still redacts ciphertext.
  - Demo: DB tests green.

- [x] **Task 13: Handler, authz and help read from config (TDD).** Add
  `config: Arc<Config>` to `HandlerContext`; look up triggers, profiles and
  headers from it; load secret values by command; adapt `authz.rs` and
  `help.rs` to the config types. Reply texts are unchanged.
  - Test: existing handler/authz/help tests re-pointed at config fixtures (same
    assertions: unknown, disabled, unauthorized, success JSON/non-JSON, failure,
    help filtering, profile override, AND logic), plus: header placeholder
    resolved from a stored secret; declared secret not set fails with a logged
    error and no secret value in logs.
  - Demo: `cargo test -p botrix-server triggers` passes.

- [x] **Task 14: API: list triggers, set/clear secret (TDD).** Replace the
  handlers per §13.6; shrink the DTOs; update `server.rs` routes; remove the
  access-profile, create/update/delete and test routes.
  - Test: 401 without session; CSRF on `PUT`/`DELETE`; list reflects config and
    `is_set`; undeclared command or secret name returns 404; empty or oversized
    value rejected; secret values never in any response or `Debug`; removed
    routes return 404/405.
  - Demo: API integration tests green.

- [ ] **Task 15: Frontend: read-only triggers table with secret actions (TDD for
  helpers).** Rewrite `pages/triggers.rs` like the Webhook endpoints card;
  delete `trigger_edit.rs`, `triggers_form.rs`, `access_profiles.rs`, their
  routes and the nav entry.
  - Test: unit tests for the set-secret request mapping and display helpers;
    wasm/`trunk build` succeeds.
  - Demo: in-browser, config triggers are listed read-only and a secret can be
    set and cleared; the value is never shown after saving.

- [ ] **Task 16: Docs and example config.** Document `triggers:` and
  `access_profiles:` in `docs/configuration.md` and `config.example.yaml`
  (including the never-put-credentials-in-YAML warning); update `README` /
  `CHANGELOG`; update `docs/webhooks.md`-style trigger docs if present; mark §13
  as the current design in §4 and §9.
  - Test: the example config passes the validation test from Task 11.
  - Demo: the example config starts the server.

- [ ] **Task 17: Hardening and verification.** Full `cargo test`,
  `cargo clippy --all-targets --all-features -D warnings`, `cargo fmt --check`,
  `cargo deny check`, wasm build, all in the devcontainer. Manual end-to-end:
  declare a trigger with a secret in YAML, set the secret in the UI, run the
  command from Matrix, then restart with the trigger removed and confirm the
  command is unknown.
  - Demo: green devcontainer run plus the documented manual end-to-end.
