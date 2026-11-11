---
layout: default
title: Configuration reference
nav_order: 2
---

# Configuration reference

Botrix is configured with a single YAML file mounted at `/config/config.yaml` (path configurable via `BOTRIX_CONFIG`). Changes require a container restart. The configuration is validated at startup — invalid config prevents the application from starting.

## Full example

```yaml
matrix:
  homeserver: https://matrix.pikaviestin.fi

  destinations:
    containers:
      room_id: "!abc123:matrix.pikaviestin.fi"
      enabled: true
    homelab:
      room_id: "!def456:matrix.pikaviestin.fi"
      enabled: true

webhooks:
  wud:
    enabled: true
    destination: containers
    msg_template: |
      Docker image update available!

      Container: ${name}
      Image: ${image.name}
      Current tag: ${image.tag.value}
      New tag: ${result.tag}

  homeassistant:
    enabled: true
    destination: homelab
    msg_template: |
      ${title}

      ${message}

access_profiles:
  ops:
    allowed_rooms:
      - "!ops123:matrix.pikaviestin.fi"
    allowed_users:
      - "@alice:matrix.pikaviestin.fi"

triggers:
  deploy:
    description: Deploy a service
    enabled: true
    method: POST
    url: https://ci.example.com/hooks/deploy
    headers:
      Authorization: "Bearer {{secret:TOKEN}}"
      X-Source: botrix
    secrets: [TOKEN]
    response_template: "Deploy ${status}: ${message}"
    access_profile: ops

server:
  host: "0.0.0.0"
  port: 8080
  max_body_bytes: 262144
```

## `matrix`

| Field | Required | Description |
|---|---|---|
| `homeserver` | Yes | HTTPS URL of your Matrix homeserver |

> The Matrix account (user ID) is connected through the web UI using an OAuth login — it is not configured here. See [Matrix setup](matrix-setup.md).

### `matrix.destinations`

A map of logical names to room definitions. The names are arbitrary — use them to reference rooms from webhook definitions.

| Field | Required | Description |
|---|---|---|
| `room_id` | Yes | Matrix room ID (`!id:server`) |
| `enabled` | No | Whether messages can be sent to this room (default: `true`) |

## `webhooks`

A map of webhook identifiers to definitions. Each identifier becomes part of the endpoint URL: `POST /api/v1/webhooks/notify/<identifier>`.

| Field | Required | Description |
|---|---|---|
| `enabled` | No | Whether the endpoint accepts requests (default: `true`) |
| `destination` | Yes | Logical name from `matrix.destinations` |
| `msg_template` | Yes | Message template string (see [Webhooks](webhooks.md)) |

## `triggers`

A map of command words to definitions. Mentioning the bot with a command calls the trigger's webhook and replies with the result. See [Command triggers](triggers.md) for how they behave.

The key is the command word. It must not contain whitespace, it is matched case-insensitively (so `Deploy` and `deploy` cannot both exist), and `help` is reserved for the built-in command.

| Field | Required | Description |
|---|---|---|
| `description` | No | Text shown by `help` (default: empty) |
| `enabled` | No | Whether the command runs (default: `true`) |
| `method` | Yes | `GET`, `POST`, `PUT`, `PATCH` or `DELETE`, in uppercase |
| `url` | Yes | An `http` or `https` URL with a host |
| `headers` | No | Map of header name to value. Values may use `{{secret:NAME}}` placeholders; see [Using secrets in headers](triggers.md#using-secrets-in-headers) for examples. |
| `secrets` | No | Names of the secrets this trigger uses (letters, digits, `_` and `-`). Set the values in the web UI. |
| `response_template` | No | Template for the reply, rendered from the JSON response. Without it, the reply is a short success or failure notice. |
| `access_profile` | No | Name of an entry in `access_profiles` |
| `allowed_rooms` | No | Room IDs allowed to run the command. Overrides the profile's list when non-empty. |
| `allowed_users` | No | User IDs allowed to run the command. Overrides the profile's list when non-empty. |

> **Never put credentials in this file.** There is no field for secret values. Declare the secret's name under `secrets`, reference it from a header as `{{secret:NAME}}`, and set the value on the **Triggers** page. The headers `Authorization`, `Proxy-Authorization`, `Cookie` and `X-Api-Key` must use a placeholder, and Botrix refuses to start if one has a literal value. This catches common mistakes only; it cannot recognise every credential.

Unknown keys are rejected, so a typo such as `allowed_room` stops startup instead of silently dropping an access restriction. Every `{{secret:NAME}}` in a header must name a declared secret, and `access_profile` must name an existing profile.

## `access_profiles`

A map of profile names to reusable allowlists that triggers reference with `access_profile`.

| Field | Required | Description |
|---|---|---|
| `allowed_rooms` | No | Room IDs (`!id:server`). Empty means any room. |
| `allowed_users` | No | User IDs (`@user:server`). Empty means any user. |

For each trigger, a non-empty inline `allowed_rooms` or `allowed_users` replaces the profile's list for that one dimension. The sender must pass both the room and the user check. An empty effective list does not restrict that dimension.

## `server`

| Field | Default | Description |
|---|---|---|
| `host` | `0.0.0.0` | Address to bind |
| `port` | `8080` | Port to listen on |
| `max_body_bytes` | `262144` | Maximum inbound webhook body size in bytes (256 KiB) |
| `public_url` | _(derived from request)_ | Public base URL for the Matrix login redirect, e.g. `https://botrix.example.com`. Required behind a reverse proxy. Also settable with `BOTRIX_PUBLIC_URL`. |

## Environment variables

| Variable | Required | Description |
|---|---|---|
| `BOTRIX_SESSION_SECRET` | Yes | Signs browser session cookies. At least 32 characters; Botrix refuses to start otherwise. |
| `BOTRIX_CRYPTO_PASSPHRASE` | Yes | Encrypts Matrix crypto state on disk. See note below. |
| `BOTRIX_CONFIG` | No | Path to config file (default: `/config/config.yaml`) |
| `BOTRIX_DATA_DIR` | No | Path to data directory (default: `/data`) |
| `BOTRIX_PUBLIC_URL` | No | Overrides `server.public_url`. The public base URL for the Matrix login redirect. |
| `BOTRIX_INSECURE_COOKIES` | No | Set to `1` to send session cookies without the `Secure` flag, for trying Botrix over plain HTTP. Leave unset behind HTTPS. |
| `BOTRIX_FRONTEND_DIR` | No | Serve the web UI from this directory instead of the copy built into the binary. For development. |
| `RUST_LOG` | No | Log filter. Examples: `info`, `warn`, `botrix=debug`, `botrix=trace`. |

### Why two secrets?

`BOTRIX_SESSION_SECRET` protects the web UI — it signs the session cookies that keep you logged into the admin interface.

`BOTRIX_CRYPTO_PASSPHRASE` protects Matrix key material at rest — it encrypts the database that stores Megolm session keys and cross-signing keys. Without it, the application will not start. Anyone who copies the `/data` volume without this passphrase cannot read any Matrix E2EE key material.

**Use different values for each.** If both secrets are the same, compromising one compromises both.
