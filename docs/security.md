# Security

## Who can do what

| Credential | Gets | Stored on the host as |
|---|---|---|
| pairing code (single use, 10 min) | one device credential | SHA-256 |
| browser device cookie (`remotium_device`) | the browser UI for the host's folder | SHA-256 |
| desktop device token | sessions and editor connections | SHA-256 |

Both kinds of device have the full power of the editor: files, terminals and extensions, as
the user running `remotium-host`. Pair only your own devices; `remotium-host revoke` cuts one
off immediately.

## Transport

- HTTPS by default. With the self-signed certificate, the VSCodium pairing link carries the
  certificate's SHA-256 and the client checks every connection against it before sending a
  request. Browsers show a warning once; compare the fingerprint `remotium-host pair` prints.
- `--plain-http` refuses non-loopback addresses unless `--plain-http-any-address` is given;
  use it only behind your own HTTPS proxy or tunnel.

## VSCodium servers

- They listen on 127.0.0.1 only, with a random connection token (file mode 0600, kept across
  restarts).
- Browsers receive that token (cookie `vscode-tkn`) because the stock workbench reads it in
  JavaScript. It opens nothing on its own: the server is reachable only through the host's
  proxy, which requires the device cookie on every request.
- Downloads come only from the configured template (VSCodium's GitHub releases by default),
  must match a published `.sha256` unless `--allow-unverified-downloads`, and must contain the
  requested commit. Version fields are validated before they reach a path or URL; extraction
  uses the `tar` crate's path checks.

## Pairing

- `GET /pair` only shows a confirmation; the code is spent by the `POST`, so link previews
  can't consume it. Device cookies are `HttpOnly` and `SameSite=Strict`.
- The client asks before pairing and shows the URL and fingerprint.

## Client-side setup

The one-time setup only adds `remotium.client` to `enable-proposed-api` in VSCodium's own
`argv.json`, after the user confirms.

## Not covered

- Several users on one host: every paired device acts as the user running the host.
- Rate limiting of `/pair` and `/remotium/v1/pair` (codes are 256-bit random and expire).
