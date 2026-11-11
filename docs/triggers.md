---
layout: default
title: Command triggers
nav_order: 4
---

{% raw %}

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

## Using secrets in headers

Anything an API needs to keep private, such as a token, key or password, goes in a header as a secret. Setting one up takes three steps:

1. List the secret's name under `secrets`.
2. Use it in a header value as `{{secret:NAME}}`.
3. Open the **Triggers** page, click **Set value** next to the name, and paste the value.

Botrix puts the stored value in place when the command runs. The text around the placeholder stays as you wrote it.

### Bearer token

The most common case. The secret holds only the token, because `Bearer ` is part of the header text in the config.

```yaml
triggers:
  deploy:
    description: Deploy a service
    method: POST
    url: https://ci.example.com/hooks/deploy
    headers:
      Authorization: "Bearer {{secret:TOKEN}}"
    secrets: [TOKEN]
```

### API key in a custom header

Some services expect the key in a header of their own instead of `Authorization`.

```yaml
triggers:
  restart:
    description: Restart the app
    method: POST
    url: https://api.example.com/v1/restart
    headers:
      X-Api-Key: "{{secret:API_KEY}}"
    secrets: [API_KEY]
```

### HTTP Basic authentication

Basic authentication sends `user:password` encoded as Base64. Botrix inserts the value exactly as stored and does not encode anything, so put the **encoded** value in the secret. Encode it yourself on your own machine, as a single line:

```bash
printf '%s' 'user:password' | base64
```

Use `printf` rather than `echo`, because `echo` adds a newline that ends up inside the encoded value. Then paste the result as the secret's value:

```yaml
triggers:
  backup:
    description: Start a backup
    method: POST
    url: https://backup.example.com/run
    headers:
      Authorization: "Basic {{secret:BASIC_CREDENTIALS}}"
    secrets: [BASIC_CREDENTIALS]
```

### Several secrets and plain headers

A trigger can declare as many secrets as it needs. One secret can be used in several headers, and one header value can contain several placeholders. Headers that hold nothing private are written out in full.

```yaml
triggers:
  sync:
    description: Sync the data
    method: POST
    url: https://api.example.com/v1/sync
    headers:
      Authorization: "Bearer {{secret:TOKEN}}"
      X-Client: "{{secret:CLIENT_ID}}:{{secret:CLIENT_SECRET}}"
      Accept: application/json
      X-Source: botrix
    secrets: [TOKEN, CLIENT_ID, CLIENT_SECRET]
```

### Rules

- Secret names use letters, digits, `_` and `-`, and are case-sensitive: `{{secret:token}}` does not match `TOKEN`.
- Every placeholder must name a secret listed under `secrets` for that trigger.
- The headers `Authorization`, `Proxy-Authorization`, `Cookie` and `X-Api-Key` must contain a placeholder. A literal value there stops Botrix from starting, so a credential cannot be committed by accident. Botrix cannot recognise a credential under another header name such as `X-Auth-Token`, so use a placeholder for those yourself.
- Paste the value as it is, without quotes or surrounding spaces. It is inserted exactly as stored. A stored value that itself contains `{{secret:...}}` is not expanded again.
- To rotate a secret, use **Set value** again. The next command uses the new value and nothing needs a restart. **Clear** removes it, and commands that use it fail until a new value is set.

### When something is wrong

| Mistake | What happens |
|---|---|
| A header uses `{{secret:NAME}}` but `NAME` is not under `secrets`, or the case differs | Botrix refuses to start and names the trigger, the header and the secret |
| `Authorization`, `Cookie` or another credential header has a literal value | Botrix refuses to start and names the header |
| The secret is declared but has no value yet | The command replies `Command failed`. The Logs page shows `command setup failed` with `missing secret: NAME`. |

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

## Seeing what happened

Every command that runs, is refused or fails is recorded on the **Logs** page, along with the sender and room. Setting or clearing a secret is recorded too, with the secret's name but never its value.

- On the **Triggers** page, **View activity** next to a command opens the Logs page filtered to that command.
- On the Logs page you can filter by component and by command yourself. Commands are matched exactly and case-insensitively, so `echo` does not match `echo2`.
- A command that is not configured, such as a typo, is recorded under the command that was typed, so you can filter for it as well.

Events are kept for 30 days, and at most 10,000 are stored.

## Outbound request limits

- Only `http` and `https` URLs are allowed, and certificates are validated.
- Requests to loopback, private, link-local, unique-local, unspecified and multicast addresses are blocked, including when a hostname resolves to one.
- This also means a trigger cannot call a service on your local network by its private IP, or by a hostname that resolves to one. Point the trigger at an address that resolves to a public IP.
- Redirects are not followed.
- The connection times out after 5 seconds and the whole request after 10 seconds.
- Responses larger than 1 MiB are rejected.

## Access control

A trigger can limit the rooms and users that may run it, directly with `allowed_rooms` and `allowed_users` or through an `access_profile`. The sender must pass both checks, and an empty list does not restrict that dimension. Anyone who can mention the bot in a room can otherwise run an enabled command, so restrict commands that change things.

{% endraw %}
