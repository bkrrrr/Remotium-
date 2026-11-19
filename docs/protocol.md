# Protocol

Version 1. JSON field names are camelCase.

## Control API (HTTPS)

| Method & path | Auth | Request | Response |
|---|---|---|---|
| `GET /api/v1/environments` | proxy identity | | `[{id, name, online, agent, openStreams, projects:[{name, path}]}]` |
| `POST /api/v1/launch-tickets` | proxy identity, same-site | `{envId, project}` | `{uri, expiresIn}` |
| `POST /api/v1/tokens` | none | `{ticket}` or `{refreshToken}` | `{accessToken, accessExpiresIn, refreshToken, refreshExpiresIn, user, env, project?}` |
| `POST /api/v1/tokens/revoke` | none | `{refreshToken}` | 204 |
| `POST /api/v1/sessions` | Bearer access token | `{envId, commit, quality?, version, release?}` | `{sessionId, streamUrl, streamToken, streamTokenExpiresIn, connectionToken}` |
| `POST /api/v1/sessions/{id}/stream-tokens` | Bearer access token | | `{streamToken, expiresIn}` |
| `POST /api/v1/sessions/{id}/revoke` | Bearer access token | | 204 |
| `GET /api/v1/status` | none | | `{version, uptimeSecs, envsConfigured, envsOnline, openStreams, sessions}` |
| `GET /healthz` | none | | `ok` |

The dashboard (`GET /`, `POST /launch`) is server-rendered HTML on top of the same functions.

Launch URI: `<editor_scheme>://opentunnel.client/connect?ticket=T&env=E&path=/workspace/p&broker=<public_url>`.

Errors are `{error, message}` with these codes:

| HTTP | `error` | meaning |
|---|---|---|
| 400 | `bad_request` | malformed input (e.g. a commit that isn't hex) |
| 401 | `unauthorized`, `expired`, `reused` | missing/invalid/expired token; `reused` means a rotated refresh token was replayed and the sign-in was revoked |
| 403 | `forbidden` | no grant, or a token used for another env |
| 404 | `not_found` | unknown session |
| 409 | `version_unavailable` | no server for this editor version could be installed |
| 503 | `offline` | the environment's agent isn't connected |
| 500 | `internal` | |

Lifetimes: ticket 60 s, single use. Access token 15 min. Refresh token 12 h, rotating; the
previous one keeps working for 60 s (two windows refreshing at once), after that a replay
revokes the whole sign-in. Stream token 60 s, reusable for reconnects within its session.

## Data plane

### Client stream: `GET /data/v1/stream`

WebSocket upgrade with `Authorization: Bearer <streamToken>` (never in the URL). The broker
validates the token, the grant and the env, and opens the agent stream **before** answering.
Refusals are plain HTTP responses with the JSON error body and header
`x-opentunnel-close-code`:

| HTTP | close code | |
|---|---|---|
| 401 | 4401 | invalid or expired stream token |
| 403 | 4403 | forbidden |
| 503 | 4404 | environment offline |
| 409 | 4409 | server version unavailable |
| 500 | 4500 | internal |

After `101`, binary frames carry raw bytes in both directions. Text frames are treated as
bytes; ping/pong as keepalive. The close codes above are used if a failure happens after the
upgrade.

### Agent link: `GET /agent/v1/connect`

WebSocket upgrade with `Authorization: Bearer <agentToken>` (401 if unknown). Then the
connection carries [yamux](https://github.com/hashicorp/yamux/blob/master/spec.md); the
agent is the yamux client.

Frames on yamux streams: big-endian `u32` length + JSON, at most 1 MiB.

**Control stream** (stream 1, opened by the agent):

```jsonc
// agent → broker, first message
{"type":"hello","agentId":"…","envId":"demo","projects":["demo"],"arch":"linux-x64","version":"0.1.0"}
// broker → agent
{"type":"welcome","heartbeatSecs":30}
{"type":"ensureServer","id":7,"server":{"commit":"c072…","quality":"stable","version":"1.143.17232","release":null}}
// agent → broker
{"type":"ensureServerResult","id":7,"result":{"status":"ok","connectionToken":"…"}}
{"type":"ensureServerResult","id":7,"result":{"status":"error","code":"versionUnavailable","message":"…"}}
// both directions
{"type":"heartbeat"}
```

**Data streams** (opened by the broker), first frame and answer:

```jsonc
{"v":1,"kind":"reh","commit":"c072…","sessionId":"ses_…"}   // kind "tcp" is reserved
{"ok":true}
{"ok":false,"code":"versionUnavailable","error":"…"}
```

After `{"ok":true}`, raw bytes flow to and from `127.0.0.1:<REH port>`.
