# Protocol

JSON field names are camelCase. All paths are on the host's HTTPS origin.

## Pairing

`remotium-host pair` prints:

- browser: `https://<host>:<port>/pair?code=<code>`
- VSCodium: `vscodium://remotium.client/pair?host=<url-encoded host URL>&code=<code>&fp=<sha256 hex>`
  (`fp` only for self-signed certificates)

Codes look like `rmp_<43 chars base64url>`, work once and expire after 10 minutes.

## Browser

| Request | |
|---|---|
| `GET /pair?code=…` | confirmation page (doesn't use the code, so link previews can't burn it) |
| `POST /pair` (form `code`, `name`) | sets cookie `remotium_device` (HttpOnly, SameSite=Strict, Secure on HTTPS, ~400 days) and redirects to `/?folder=<folder>` |
| anything else | needs the cookie (401 page otherwise), proxied to the VSCodium server; the response sets `vscode-tkn` (SameSite=Strict, readable by the workbench) |

## Desktop (Remotium Client)

| Request | Auth | Body → Response |
|---|---|---|
| `POST /remotium/v1/pair` | none | `{code, deviceName}` → `{deviceToken, deviceId, hostId, name, folder}` |
| `GET /remotium/v1/info` | Bearer device token | → `{hostId, name, folder, version}` |
| `POST /remotium/v1/session` | Bearer | `{commit, quality, version, release}` → `{connectionToken}` (may wait for a download) |
| `GET /remotium/v1/stream?commit=…` | Bearer | WebSocket; binary frames carry the editor's raw byte stream to and from the server |
| `GET /remotium/client.vsix` | none | the client package, if the host has one |
| `GET /remotium/healthz` | none | `ok` |

Errors are `{error, message}`: 400 `bad_request`, 401 `unauthorized` / `invalid` / `expired`,
409 `version_unavailable`, 500 `internal`. The stream endpoint checks everything (device,
commit, running server) before upgrading, so failures arrive as HTTP statuses, not as a
WebSocket that opens and closes.

The authority of a remote window is `remotium+<hostId>`; the folder is the URI path.
