# Compatibility

## Supported

- **Editor:** VSCodium desktop. Tested release pinned in [`compat.json`](../compat.json)
  (1.143.17232 on macOS x64 against Linux x64 containers).
- **Server:** the VSCodium REH of exactly the editor's commit. Any OS/arch with a VSCodium
  REH build (`linux`, `alpine`, `darwin` × `x64`, `arm64`, `armhf`, …).
- **Extensions:** from Open VSX (VSCodium's default gallery). Tested: built-in Node
  debugging (js-debug), `ms-python.debugpy` with `ms-python.python` installed remotely.

## Build match

The REH refuses a client whose commit differs (its RPC isn't versioned). The client sends
`commit`, `quality`, `version` and `release` from the editor's `product.json`. The agent
looks for that commit in its search path and cache, or downloads it:

```
https://github.com/VSCodium/vscodium/releases/download/${version}${releaseSuffix}/vscodium-reh-${os}-${arch}-${version}${releaseSuffix}.tar.gz
```

Since 1.9x, VSCodium has no separate `release` field and the version (`1.143.17232`) is the
tag. Older releases append `.${release}`; `${releaseSuffix}` is `.<release>` or empty, and
open-remote-ssh's `${version}.${release}` form is also understood. Set
`AGENT_SERVER_URL_TEMPLATE` for a mirror. Each installed commit takes about 280 MB in
`/data/servers`.

The devenv image pre-bakes the pinned release in `/opt/vscodium-server/<commit>`. When you
update VSCodium, the first connection downloads the new server (about 90 MB), and
`POST /api/v1/sessions` waits for it.

## Proposed API

The extension needs the `resolvers` API proposal. VSCodium only enables proposals for
allow-listed extensions, so users add this to `~/.vscode-oss/argv.json` (*Preferences:
Configure Runtime Arguments*) and restart:

```jsonc
"enable-proposed-api": ["opentunnel.client"]
```

VSCodium already allow-lists open-remote-ssh for `resolvers`; an upstream request to add
`opentunnel.client` to its `product.json` would make this zero-config.

Proposed APIs can change between releases. The typings are pinned in
`client/src/vscode.proposed.resolvers.d.ts` (from microsoft/vscode 1.143.1). The parts used
are `registerRemoteAuthorityResolver`, `ManagedResolvedAuthority`,
`ManagedMessagePassing` (including optional `drain`) and `RemoteAuthorityResolverError`.
Re-check them on every VSCodium update; CI should run the pinned release and the latest.

## Testing a release

```sh
docker compose -f deploy/docker-compose.yml up --build -d
npm --prefix client run test:e2e      # sign in through a launch link, file system and terminal
npm --prefix client run test:phase3   # + port forwarding, debugging, Open VSX, restarts
```

Set `VSCODIUM_PATH` (or `--codium`) to test another VSCodium build. The runner uses an
isolated profile (`--user-data-dir`, `--extensions-dir`, `--shared-data-dir`), so it doesn't
touch, or wait for, an everyday VSCodium.

## Not supported

- **Microsoft VS Code desktop.** It only accepts Microsoft's server, whose license doesn't
  allow hosting it for others. See [licensing](licensing.md).
- **Microsoft-only extensions** (Pylance, C# Dev Kit, Live Share, Remote-*): licensed for
  Microsoft products only, and not on Open VSX.
- **VS Code for the Web / vscode.dev.**
