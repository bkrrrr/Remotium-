# Feasibility (M0 spike)

> Historical: this spike predates the current design (remotium-host, no broker). The
> finding that matters still holds: a VSCodium remote window can run over a WebSocket the
> extension provides.

**Question:** can a VSCodium desktop editor talk to a VSCodium REH over a WebSocket that
our own extension provides, without modifying the editor and without SSH or a local TCP
port?

**Answer: yes.** Verified on 2026-11-19.

## Setup

| | |
|---|---|
| Editor | VSCodium 1.143.17232 (commit `c072cf642cbbadabefa22f39c58cc902a8b7427f`), macOS x64 |
| Server | `vscodium-reh-darwin-x64-1.143.17232.tar.gz`, same commit, sha256 checked against the release's `.sha256` file |
| Agent | `remotium-agent --direct-listen 127.0.0.1:8099 --server-dir <extracted REH>` |
| Client | `remotium.client` loaded with `--extensionDevelopmentPath`, `--enable-proposed-api remotium.client`, settings `remotium.directUrl=ws://127.0.0.1:8099` and `remotium.directConnectionToken=<printed by the agent>` |

The agent starts `bin/codium-server --host 127.0.0.1 --port 0 --connection-token-file <f>
--telemetry-level off --accept-server-license-terms` and reads the port from the stdout
line `Extension host agent listening on <port>`. Each accepted WebSocket is piped to a
new TCP connection to that port (`tokio::io::copy_bidirectional`).

The client's resolver returns
`new vscode.ManagedResolvedAuthority(() => connectWs(url), connectionToken)`.
`connectWs` (`client/src/wsMessagePassing.ts`) adapts a `ws` WebSocket to
`ManagedMessagePassing`.

## Run

The spike's direct mode (agent `--direct-listen`, client setting `directUrl`) connected
VSCodium straight to an agent with no authentication. It was removed once the server path
worked; it is in the history at commit `3e034b6` ("Add M0 direct-mode spike").

## Results

- The editor logs `resolveAuthority(remotium) returned 'Managed(1)' after 39 ms`.
- The window opens on the remote folder; `vscode.env.remoteName` is `remotium`.
- `workspace.fs.writeFile` / `readFile` / `readDirectory` on `vscode-remote://remotium+direct/...`
  work, and the edit is on the REH machine's disk.
- `window.createTerminal` starts a shell on the REH machine; a command run in it writes a file
  that the client then sees through the remote FS.

So the editor's own protocol (management connection, extension host connection, file
system, terminals) runs unchanged over our byte stream. The editor makes its own
HTTP-upgrade handshake inside the stream, and the REH checks the connection token itself.

## Findings that shape the design

- **Version string:** VSCodium 1.143 has `"version": "1.143.17232"` and no `release`
  field. The release tag and asset names use the version alone
  (`.../download/1.143.17232/vscodium-reh-linux-x64-1.143.17232.tar.gz`). Older
  releases had a separate `release` that is appended as `.<release>`. The download
  template supports both.
- **REH archives** have no top-level directory (`./bin`, `./out`, …) and a `.sha256`
  sidecar file next to every asset.
- **`ManagedMessagePassing.drain()`** exists in 1.143 as an optional method. The client
  implements it on top of `ws.bufferedAmount`, so there is a basic backpressure path
  after all; we still log when the buffer grows large.
- **Launching VSCodium from inside another VS Code** inherits `ELECTRON_RUN_AS_NODE`, which
  turns the editor into plain Node (`bad option: --user-data-dir`). The launcher removes it.
- **macOS socket path limit:** the editor's IPC socket sits in `--user-data-dir` and must
  stay under 103 characters, so the launcher keeps the profile in a short temp dir.
- Data in the WebSocket is opaque to us: no framing of our own is needed on top of it.
