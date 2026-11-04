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

server:
  host: "0.0.0.0"
  port: 8080
  max_body_bytes: 262144
```

## `matrix`

| Field | Required | Description |
|---|---|---|
| `homeserver` | Yes | HTTPS URL of your Matrix homeserver |

> The Matrix account (user ID) is connected through the web UI using OAuth Device Authorization Grant — it is not configured here. See [Matrix setup](matrix-setup.md).

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

## `server`

| Field | Default | Description |
|---|---|---|
| `host` | `0.0.0.0` | Address to bind |
| `port` | `8080` | Port to listen on |
| `max_body_bytes` | `262144` | Maximum inbound webhook body size in bytes (256 KiB) |

## Environment variables

| Variable | Required | Description |
|---|---|---|
| `BOTRIX_SESSION_SECRET` | Yes | Signs browser session cookies. At least 32 characters; Botrix refuses to start otherwise. |
| `BOTRIX_CRYPTO_PASSPHRASE` | Yes | Encrypts Matrix crypto state on disk. See note below. |
| `BOTRIX_CONFIG` | No | Path to config file (default: `/config/config.yaml`) |
| `BOTRIX_DATA_DIR` | No | Path to data directory (default: `/data`) |
| `BOTRIX_INSECURE_COOKIES` | No | Set to `1` to send session cookies without the `Secure` flag, for trying Botrix over plain HTTP. Leave unset behind HTTPS. |
| `BOTRIX_FRONTEND_DIR` | No | Serve the web UI from this directory instead of the copy built into the binary. For development. |
| `RUST_LOG` | No | Log filter. Examples: `info`, `warn`, `botrix=debug`, `botrix=trace`. |

### Why two secrets?

`BOTRIX_SESSION_SECRET` protects the web UI — it signs the session cookies that keep you logged into the admin interface.

`BOTRIX_CRYPTO_PASSPHRASE` protects Matrix key material at rest — it encrypts the database that stores Megolm session keys and cross-signing keys. Without it, the application will not start. Anyone who copies the `/data` volume without this passphrase cannot read any Matrix E2EE key material.

**Use different values for each.** If both secrets are the same, compromising one compromises both.
