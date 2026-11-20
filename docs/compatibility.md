# Compatibility

## Supported

- **Editor:** VSCodium desktop. Tested release pinned in [`compat.json`](../compat.json)
  (1.143.17232 on macOS x64 against Linux x64 containers).
- **Server:** VSCodium's web server variant (`vscodium-reh-web-*`) of exactly the editor's
  commit; it serves desktop clients and the browser UI. remotium-host runs on Linux (glibc or
  musl) and macOS, x64 and arm64.
- **Browser:** current Chromium, Firefox and Safari (the stock VSCodium workbench).
- **Extensions:** from Open VSX (VSCodium's default gallery), installed into the server like
  with any VSCodium remote. Tested: debugging with the built-in Node debugger (js-debug).

## Build match

The REH refuses a client whose commit differs (its RPC isn't versioned). The client sends
`commit`, `quality`, `version` and `release` from the editor's `product.json`. The agent
Desktop clients send `commit`, `quality`, `version` and `release` from the editor's
`product.json`; remotium-host looks for that commit in its cache or downloads it:

```
https://github.com/VSCodium/vscodium/releases/download/${version}${releaseSuffix}/vscodium-reh-web-${os}-${arch}-${version}${releaseSuffix}.tar.gz
```

Since 1.9x, VSCodium has no separate `release` field and the version (`1.143.17232`) is the
tag. Older releases append `.${release}`; `${releaseSuffix}` is `.<release>` or empty, and
open-remote-ssh's `${version}.${release}` form is also understood. Set
`--reh-url-template` (`REMOTIUM_REH_URL_TEMPLATE`) for a mirror. Each installed commit takes
about 320 MB in `~/.remotium-host/servers`; the download is about 95 MB, and the first
connection of a new VSCodium version waits for it.

Browsers get the release pinned in `compat.json`, whatever desktop clients use.

## Proposed API

The extension needs the `resolvers` API proposal. VSCodium only enables proposals for
allow-listed extensions or ones listed in `argv.json`:

```jsonc
"enable-proposed-api": ["remotium.client"]
```

Without it the extension still activates (only the proposed calls throw), so it offers to add
that line itself and asks for a restart. It edits the file VSCodium reads,
`~/.vscode-oss/argv.json` or `$VSCODE_PORTABLE/argv.json` in portable mode, with a
comment-preserving JSONC edit. `env.remoteAuthority` also belongs to the proposal, so the
extension uses `env.remoteName` and the workspace folder URI instead.

VSCodium already allow-lists open-remote-ssh for `resolvers`; an upstream request to add
`remotium.client` to its `product.json` would remove the restart.

Proposed APIs can change between releases. The typings are pinned in
`client/src/vscode.proposed.resolvers.d.ts` (from microsoft/vscode 1.143.1). The parts used
are `registerRemoteAuthorityResolver`, `ManagedResolvedAuthority`,
`ManagedMessagePassing` (including optional `drain`) and `RemoteAuthorityResolverError`.
Re-check them on every VSCodium update; CI should run the pinned release and the latest.

## Testing a release

```sh
cargo build -p remotium-host
npm --prefix client run test:e2e:vsix   # install the VSIX, setup, restart, pair, connect
npm --prefix client run test:phase3     # terminal, port forwarding, debugging, host restart
npm --prefix client run test:browser    # Chromium: pair, workbench, edit a file
```

Set `VSCODIUM_PATH` (or `--codium`) to test another VSCodium build. The runner uses an
isolated profile (`--user-data-dir`, `--extensions-dir`, `--shared-data-dir`), so it doesn't
touch, or wait for, an everyday VSCodium.

## Not supported

- **Microsoft VS Code desktop.** It only accepts Microsoft's server, whose license doesn't
  allow hosting it for others. See [licensing](licensing.md).
- **Microsoft-only extensions** (Pylance, C# Dev Kit, Live Share, Remote-*): licensed for
  Microsoft products only, and not on Open VSX.
- **vscode.dev** (Microsoft's hosted web editor). Browser access goes through the host's own UI.
