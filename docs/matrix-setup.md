---
layout: default
title: Matrix setup
nav_order: 4
---

# Matrix setup

> **Status:** the account-connection flow and the encryption recovery-key step below describe the intended behavior. They are **not implemented yet**: the **Connect** and **Recovery key** actions in the UI currently report that they are unavailable, and notifications stay queued. Destinations, invitations, test messages and disconnecting are implemented.

Botrix connects to Matrix using OAuth 2.0 Device Authorization Grant. Authentication is done through the web UI — you never need to put passwords or tokens in configuration files.

## Requirements

- A Matrix account for the bot (separate from your personal account is recommended)
- The account must be on a homeserver that uses [Matrix Authentication Service (MAS)](https://matrix-org.github.io/matrix-authentication-service/) — plain password-based homeservers are not supported

## Connecting an account

1. Open the Botrix web UI and go to **Matrix**
2. Click **Connect Matrix account**
3. Botrix displays a verification URL and a short user code
4. Open the URL in a browser and complete authentication for the bot account
5. Return to Botrix — the connection status updates automatically

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
