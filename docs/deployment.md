---
layout: default
title: Deployment guide
nav_order: 5
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

Botrix does not handle TLS. Place it behind a reverse proxy (Caddy, nginx, Traefik) for HTTPS.

Example Caddy block:
```
botrix.yourdomain.com {
    reverse_proxy localhost:8080
}
```

## Environment variables

See the [Configuration reference](configuration.md#environment-variables) for a full list.
