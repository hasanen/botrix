---
layout: default
title: Webhook integrations
nav_order: 3
---

# Webhook integrations

Botrix exposes a single dynamic endpoint for all integrations:

```
POST /api/v1/webhooks/notify/:identifier
```

The `:identifier` maps to a webhook definition in your `config.yaml`. Each definition specifies which Matrix room to send to and how to format the message.

## Authentication

Every endpoint requires a Bearer token. Tokens are managed in the web UI under **Webhooks**.

```
Authorization: Bearer <your-token>
```

Generate a token for an endpoint, copy it when shown (it is only displayed once), and add it to your integration's webhook configuration.

## Message templates

Templates use `${path}` syntax to reference values from the incoming JSON payload.

```yaml
msg_template: |
  Docker image update!

  Container: ${name}
  New tag: ${result.tag}
```

### Supported syntax

| Syntax | Description |
|---|---|
| `${field}` | Top-level JSON field |
| `${object.field}` | Nested object field |
| `${object.nested.deep}` | Arbitrary depth |
| `${array.0.field}` | Array element by index |

### Error handling

A missing variable causes the webhook to return `400 Bad Request` with a message identifying the missing path. This is intentional — silent empty substitutions hide misconfiguration.

## Example integrations

### What's Up Docker (WUD)

Configure WUD to send webhooks to Botrix using the [HTTP trigger](https://getwud.github.io/wud/docs/configuration/triggers/http/):

```yaml
# WUD configuration (environment variables)
# WUD_TRIGGER_HTTP_BOTRIX_URL=https://your-botrix/api/v1/webhooks/notify/wud
# WUD_TRIGGER_HTTP_BOTRIX_METHOD=POST
# WUD_TRIGGER_HTTP_BOTRIX_AUTH_TYPE=BEARER
# WUD_TRIGGER_HTTP_BOTRIX_AUTH_BEARER=<your-token>
```

WUD sends a POST request with this payload structure (abbreviated):

```json
{
  "name": "homeassistant",
  "watcher": "local",
  "image": {
    "name": "homeassistant/home-assistant",
    "tag": { "value": "2024.6.4" }
  },
  "result": { "tag": "2024.6.5" }
}
```

Botrix webhook configuration:

```yaml
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
```

WUD webhook URL: `https://your-botrix/api/v1/webhooks/notify/wud`

### Home Assistant

If Home Assistant is sending webhooks directly to Botrix (e.g. from a `rest_command` or `notify` call), the payload fields depend on what HA sends. A typical automation payload might look like:

```yaml
webhooks:
  homeassistant:
    enabled: true
    destination: homelab
    msg_template: |
      ${title}

      ${message}
```

Webhook URL: `https://your-botrix/api/v1/webhooks/notify/homeassistant`

> **Note:** If you are using WUD's [Home Assistant trigger](https://getwud.github.io/wud/docs/configuration/triggers/homeassistant/) to send from WUD → HA → Botrix, the WUD HA payload also includes top-level `title` and `message` fields, so this template works for that flow too.

### GitHub (via webhooks)

```yaml
webhooks:
  github:
    enabled: true
    destination: homelab
    msg_template: |
      ${repository.full_name}: ${commits.0.message}

      Pushed by ${pusher.name}
      ${compare}
```

Webhook URL: `https://your-botrix/api/v1/webhooks/notify/github`

### Generic / curl test

```bash
curl -X POST https://your-botrix/api/v1/webhooks/notify/mywebhook \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{"title": "Test", "message": "Hello from curl"}'
```

## Response codes

| Code | Meaning |
|---|---|
| `202 Accepted` | Notification queued for delivery |
| `400 Bad Request` | Invalid JSON or missing template variable |
| `401 Unauthorized` | Missing or invalid token |
| `404 Not Found` | Unknown or disabled identifier |
| `413 Payload Too Large` | Body exceeds size limit |
| `500 Internal Server Error` | Unexpected error |
