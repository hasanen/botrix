---
layout: default
title: Deployment guide
nav_order: 6
---

# Deployment guide

Botrix runs as a single OCI container. It is designed for rootless Podman but works with Docker too.

## Data layout

```
/config/config.yaml    ← your config file, mount read-only
/data/
  botrix.db            ← application database (auto-created)
  matrix/
    state.db           ← Matrix session state (auto-created)
    crypto.db          ← E2EE key material (auto-created)
```

Both directories must survive container recreation. Mount them as volumes.

## Running with Podman

```bash
podman run -d \
  --name botrix \
  --restart unless-stopped \
  -p 8080:8080 \
  -v /opt/botrix/config:/config:ro \
  -v /opt/botrix/data:/data \
  -e BOTRIX_SESSION_SECRET=<random-32-byte-value> \
  -e BOTRIX_CRYPTO_PASSPHRASE=<random-passphrase> \
  ghcr.io/hasanen/botrix:latest
```

> **Rootless Podman note:** `--restart unless-stopped` requires either rootful Podman or a systemd unit. For rootless Podman, generate a systemd service instead:
> ```bash
> podman generate systemd --name botrix --files --new
> ```
> Then enable and start it with `systemctl --user enable --now container-botrix`.

Generate secret values with:
```bash
openssl rand -base64 32
```

Use **different values** for `BOTRIX_SESSION_SECRET` and `BOTRIX_CRYPTO_PASSPHRASE`.

## Running with Docker Compose / Podman Compose

```yaml
services:
  botrix:
    image: ghcr.io/hasanen/botrix:latest
    restart: unless-stopped
    ports:
      - "8080:8080"
    volumes:
      - ./config:/config:ro
      - ./data:/data
    environment:
      BOTRIX_SESSION_SECRET: "${BOTRIX_SESSION_SECRET}"
      BOTRIX_CRYPTO_PASSPHRASE: "${BOTRIX_CRYPTO_PASSPHRASE}"
      RUST_LOG: info
    healthcheck:
      test: ["CMD", "botrix", "healthcheck"]
      interval: 30s
      timeout: 5s
      start_period: 10s
      retries: 3
```

Store the secret values in a `.env` file (not committed to version control).

## First-run setup

On the first start with no existing data, Botrix shows a setup screen at `http://localhost:8080/setup`. Create your admin username and password there.

After setup, go to **Matrix** to connect the bot account.

## Updating

```bash
podman pull ghcr.io/hasanen/botrix:latest
podman stop botrix
podman rm botrix
# re-run with the same podman run command as above
```

The `/data` volume is preserved across updates.

## Reverse proxy

Botrix does not handle TLS and has **no built-in rate limiting**. Place it behind a reverse proxy (Caddy, nginx, Traefik) for HTTPS, and add rate limiting there, in particular for `/api/v1/auth/login` and `/api/v1/webhooks/notify/`, which are reachable without a session.

Example Caddy block:
```
botrix.yourdomain.com {
    reverse_proxy localhost:8080
}
```

## Environment variables

See the [Configuration reference](configuration.md#environment-variables) for a full list.

## Health check

Botrix exposes a health check endpoint:

```
GET /health
```

It returns `200 OK` while the application is running. The image already defines a `HEALTHCHECK` that runs `botrix healthcheck`, a built-in probe of the local `/health` endpoint, so there is nothing to configure with Docker. The image contains no `curl`; use the built-in command if you define your own check:

```bash
podman exec botrix botrix healthcheck   # exit code 0 = healthy
```

For a quick look at the queue without the web UI:

```bash
podman exec botrix botrix status   # Matrix status, queued and failed notifications
```

Podman's default image format ignores `HEALTHCHECK`; the compose `healthcheck:` block above (or `--health-cmd 'botrix healthcheck'`) covers that.

## Hardening

Botrix only writes to `/data`, so it runs with a locked-down profile:

```bash
podman run -d --name botrix \
  --security-opt no-new-privileges --cap-drop ALL \
  --read-only --tmpfs /tmp \
  -p 8080:8080 \
  -v /opt/botrix/config:/config:ro \
  -v /opt/botrix/data:/data \
  -e BOTRIX_SESSION_SECRET=... -e BOTRIX_CRYPTO_PASSPHRASE=... \
  ghcr.io/hasanen/botrix:latest
```

The container runs as UID 1000. Make sure that user can write to the `/data` volume.

## Logs

The **Logs** page in the web UI shows operational events (logins, token changes, rejected webhook payloads, delivery failures, Matrix connection events). Events are kept for 30 days, capped at 10,000 entries, and never contain passwords, tokens or message bodies. Unauthenticated requests, such as webhook calls with a bad token, appear only in the container log (`podman logs botrix`), not in the UI.

## Backup and restore

The entire state of a Botrix instance lives in `/data`. Back this directory up regularly.

```bash
# Backup
tar -czf botrix-backup-$(date +%Y%m%d).tar.gz /opt/botrix/data

# Restore (stop the container first)
podman stop botrix
tar -xzf botrix-backup-20260101.tar.gz -C /
podman start botrix
```

**What is in `/data`:**

| Path | Contents | Impact if lost |
|---|---|---|
| `botrix.db` | Admin account, sessions, webhook tokens, notification history | Must redo first-run setup, regenerate all tokens |
| `matrix/state.db` | Matrix session state | Must reconnect Matrix account |
| `matrix/crypto.db` | E2EE keys | Must re-enter recovery key to restore encryption state |

Back up `/config/config.yaml` separately — it is not stored in `/data`.
