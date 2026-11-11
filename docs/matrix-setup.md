---
layout: default
title: Matrix setup
nav_order: 5
---

# Matrix setup

> **Status:** account connection (the redirect login below), destinations, invitations, test messages and disconnecting are implemented. The **encryption recovery-key** step is still a stub: until it lands, Botrix can send to unencrypted rooms but not to end-to-end-encrypted ones.

Botrix connects to Matrix using OAuth 2.0 Device Authorization Grant. Authentication is done through the web UI — you never need to put passwords or tokens in configuration files.

## Requirements

- A Matrix account for the bot (separate from your personal account is recommended)
- The account must be on a homeserver that uses [Matrix Authentication Service (MAS)](https://matrix-org.github.io/matrix-authentication-service/) — plain password-based homeservers are not supported

## Connecting an account

Botrix uses the OAuth 2.0 Authorization Code (redirect) flow:

1. Open the Botrix web UI and go to **Matrix**
2. Click **Connect Matrix account**
3. Your browser is sent to the homeserver's login page
4. Log in as the bot account and approve the requested access
5. The homeserver redirects you back to Botrix, which completes the connection

For the redirect to return to the right place, Botrix must know its own public
URL. Set `server.public_url` (or `BOTRIX_PUBLIC_URL`) to where Botrix is
reachable, e.g. `https://botrix.example.com`; the redirect target is
`{public_url}/api/v1/matrix/callback`.

Your homeserver must support [Matrix Authentication Service (MAS)](https://matrix-org.github.io/matrix-authentication-service/) — the redirect login is an OAuth flow, so password-only homeservers are not supported.

### Requirements for `public_url`

MAS validates the URL Botrix registers, and most deployments enforce:

- **HTTPS.** Plain `http://` is rejected. Put Botrix behind a TLS-terminating
  reverse proxy (Caddy, nginx, Traefik).
- **A real hostname or IP, not loopback.** `localhost` and `127.0.0.1` are
  rejected even over HTTPS. Use a routable hostname (`https://botrix.example.com`),
  a LAN IP (`https://10.0.0.5`), or a local name you map in `/etc/hosts`
  (`https://botrix.local`). Botrix warns at startup if `public_url` is loopback.

You must reach the Botrix UI at the **same** URL as `public_url`, so your admin
login cookie is valid when the homeserver redirects back to the callback.

Minimal local setup with Caddy and an `/etc/hosts` entry:

```
# /etc/hosts
127.0.0.1  botrix.local
```

```
# Caddyfile — Caddy issues a trusted local certificate automatically
botrix.local {
    reverse_proxy localhost:8080
}
```

```yaml
# config.yaml
server:
  public_url: "https://botrix.local"
```

Then open `https://botrix.local` and connect. (With HTTPS in place you do not
need `BOTRIX_INSECURE_COOKIES`.)

## E2EE setup

After connecting, Botrix needs to bootstrap end-to-end encryption. If prompted:

1. Go to **Matrix** in the web UI
2. Click **Set up encryption**
3. Enter your Matrix recovery key
4. Botrix restores its cryptographic state and is ready to send encrypted messages

The recovery key is used once and never stored. On subsequent restarts, Botrix restores E2EE state from its local crypto store.

## Room destinations

Rooms are configured in `config.yaml`, not through the UI. After adding destinations to the config and restarting, they appear in the Matrix page.

For each destination, the UI shows:
- Room name (resolved from Matrix)
- Room ID
- Bot membership state (`joined`, `invited`, `not joined`)
- Room encryption state

### Accepting room invitations

Botrix does not automatically join rooms. If the bot receives an invitation, it appears in the web UI with an **Accept invitation** button. Click it to join.

To invite the bot to a room, use your regular Matrix client and invite `@botrix:your-server`.

## Test messages

To verify a destination is working:

1. Go to **Matrix** in the web UI
2. Select a destination from the list
3. Click **Send test message**

The result (delivered or error) is shown immediately.

## Disconnecting

To disconnect the current Matrix account:

1. Go to **Matrix → Disconnect account**
2. Confirm the action

This removes the Matrix session and all cryptographic state. Webhook history and tokens are not affected. You can connect a new account immediately after.
