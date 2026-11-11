---
layout: default
title: Command triggers
nav_order: 4
---

# Command triggers

Command triggers are the reverse of [webhooks](webhooks.md). Someone mentions the bot in a Matrix room, Botrix calls an external HTTP webhook, and the bot replies in the room with the result.

```
@botrix deploy production
```

Triggers are defined in the YAML config, so they live in version control. The web UI lists them and manages their secret values. Changes to the config need a restart.

## Define a trigger

Add the trigger under `triggers` and, optionally, shared allowlists under `access_profiles`. The full field list is in the [configuration reference](configuration.md#triggers).

```yaml
access_profiles:
  ops:
    allowed_rooms: ["!ops123:matrix.example.com"]
    allowed_users: ["@alice:matrix.example.com"]

triggers:
  deploy:
    description: Deploy a service
    method: POST
    url: https://ci.example.com/hooks/deploy
    headers:
      Authorization: "Bearer {{secret:TOKEN}}"
    secrets: [TOKEN]
    response_template: "Deploy ${status}: ${message}"
    access_profile: ops
```

Then open **Triggers** in the web UI and use **Set value** next to `TOKEN`. Until a declared secret has a value, commands that use it fail.

## Secrets

- Config holds secret **names** only. Values are set on the Triggers page, stored encrypted in the database, and never shown again; the UI only shows whether each is set.
- A header value can combine text and placeholders, for example `Bearer {{secret:TOKEN}}`.
- Renaming a secret or a command in the config leaves the old stored value unused in the database. Setting the old name again later picks it up.

## How a message becomes a command

A message is a command when the bot is mentioned in it and the text starts with that mention. The first word after the mention is the command, matched case-insensitively. Everything after it is passed on as arguments.

- `help` is built in. It lists the enabled commands the sender may run in that room.
- An unknown or disabled command gets the reply `unknown command`.
- A sender who fails the access check gets `not authorized`, and the webhook is not called.

## The request

Botrix calls the trigger's `url` with the configured method and headers. The JSON body always has these keys:

| Key | Value |
|---|---|
| `command` | The command word, lowercase |
| `arguments` | The text after the command, as one string |
| `args` | The same text split on whitespace |
| `raw_message` | The full message text |
| `sender` | The sender's Matrix user ID |
| `room_id` | The room ID |
| `event_id` | The event ID of the message |

## The reply

The reply is sent as a reply to the triggering message.

| Outcome | Reply |
|---|---|
| 2xx JSON response and a `response_template` | The template rendered with the response body, using the same `${field.path}` syntax as [webhook templates](webhooks.md#message-templates) |
| 2xx but no template, a non-JSON body, or a template that cannot be rendered | `Command succeeded` |
| Non-2xx status, timeout, blocked address, missing secret or any other error | `Command failed` |

Details of failures go to the **Logs** page. They are never put in the room, and logs never contain secret values or response bodies.

Replies are not retried. A command is never replayed, so a failed `deploy` is reported once and not run again after a restart.

## Outbound request limits

- Only `http` and `https` URLs are allowed, and certificates are validated.
- Requests to loopback, private, link-local, unique-local, unspecified and multicast addresses are blocked, including when a hostname resolves to one.
- This also means a trigger cannot call a service on your local network by its private IP, or by a hostname that resolves to one. Point the trigger at an address that resolves to a public IP.
- Redirects are not followed.
- The connection times out after 5 seconds and the whole request after 10 seconds.
- Responses larger than 1 MiB are rejected.

## Access control

A trigger can limit the rooms and users that may run it, directly with `allowed_rooms` and `allowed_users` or through an `access_profile`. The sender must pass both checks, and an empty list does not restrict that dimension. Anyone who can mention the bot in a room can otherwise run an enabled command, so restrict commands that change things.
