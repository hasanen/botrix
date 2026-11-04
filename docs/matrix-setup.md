---
layout: default
title: Matrix setup
nav_order: 4
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
URL. Behind a reverse proxy, set `server.public_url` in the config (e.g.
`https://botrix.example.com`); the redirect target is
`{public_url}/api/v1/matrix/callback`. Without a proxy, Botrix derives it from
the request automatically.

Your homeserver must support [Matrix Authentication Service (MAS)](https://matrix-org.github.io/matrix-authentication-service/) — the redirect login is an OAuth flow, so password-only homeservers are not supported.

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
