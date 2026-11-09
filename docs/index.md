---
layout: home
title: Botrix
nav_order: 1
---

# Botrix

A self-hosted Matrix notification gateway. Botrix receives HTTP webhooks from external services and forwards formatted messages to Matrix rooms — no custom code required, just YAML configuration.

## Features

- **Dynamic webhook routing** — one endpoint, unlimited integrations defined in YAML
- **Simple template syntax** — map JSON payload fields to message text with `${variable}`
- **End-to-end encryption** — sends messages to encrypted Matrix rooms (E2EE)
- **Web UI** — manage connections, tokens, and notification history from a browser
- **Single container** — one OCI image, no external services required

## Get started

1. [Install and configure](deployment.md) the container
2. [Connect your Matrix account](matrix-setup.md)
3. [Configure webhook integrations](webhooks.md)

## Documentation

- [Configuration reference](configuration.md) — YAML config, environment variables
- [Webhook integrations](webhooks.md) — template syntax, examples, response codes
- [Matrix setup](matrix-setup.md) — connecting an account, E2EE, room management
- [Deployment guide](deployment.md) — container setup, backup, reverse proxy
- [Development](development.md) — devcontainer, checks, running locally

## Source

[github.com/hasanen/botrix](https://github.com/hasanen/botrix) — MIT license
