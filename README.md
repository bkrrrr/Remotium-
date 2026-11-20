# Remotium

Open VSCodium on your own machines over HTTPS: in the browser, or from desktop VSCodium.
There's no broker, no SSH and no account.

```
browser ──────────────HTTPS──┐
                              ├──► remotium-host ──► VSCodium server (REH)    machine with your code
VSCodium + Remotium Client ──┘
```

| Part | Where | What |
|------|-------|------|
| **remotium-host** ([`crates/host`](crates/host)) | the machine with your code (server, VM, container, Linux or macOS) | One program. It serves HTTPS, pairs devices, downloads and runs the matching VSCodium server, serves the browser UI, and carries desktop connections. |
| **Remotium Client** ([`client`](client)) | desktop VSCodium | The only plugin (`remotium.client`). It connects VSCodium to a host as a normal remote window. |

The editor's own remote machinery does the work (files, terminals, debugging, extensions
from Open VSX, port forwarding, reconnecting). Remotium only sets up the connection.

Docs: [running the host](docs/running.md) · [architecture](docs/architecture.md) ·
[protocol](docs/protocol.md) · [security](docs/security.md) ·
[compatibility](docs/compatibility.md) · [licensing](docs/licensing.md) ·
[releasing](docs/releasing.md)

## Quick start

On the machine with your code (download `remotium-host` for your system from the
[releases](../../releases), or `cargo build --release -p remotium-host`):

```sh
./remotium-host --folder ~/project
```

The first time, it creates a self-signed certificate and prints a pairing link:

```
Pairing link (works once, for 10 minutes):
  Browser:  https://devbox:8443/pair?code=rmp_…
  VSCodium: vscodium://remotium.client/pair?host=https%3A%2F%2Fdevbox%3A8443&code=rmp_…&fp=…
The certificate is self-signed. Browsers will warn; continue only if it shows this SHA-256 fingerprint:
  CD:C8:41:…
```

- **Browser:** open the browser link, accept the certificate warning if the fingerprint
  matches, and click **Pair**. The full VSCodium UI opens on `~/project`.
- **Desktop VSCodium:**
  1. Install Remotium Client from the host's pairing page (download, then drag it onto the
     Extensions view), from Open VSX (search **Remotium**), or from the release page.
  2. On first start the client asks once to finish its setup (allow its remote connection
     API) and to restart VSCodium.
  3. Open the VSCodium link. The client pins the certificate and opens a remote window on
     `~/project`.

More devices: `remotium-host pair`. List and revoke: `remotium-host devices`,
`remotium-host revoke <id>`.

The machine has to be reachable from your computer: same network, VPN (e.g. Tailscale),
a port forward, or a tunnel or reverse proxy you run. See [running the host](docs/running.md)
for your own certificate, `--plain-http` behind a proxy, systemd and containers.

Only VSCodium is supported. Microsoft VS Code needs Microsoft's server, whose license
doesn't allow this. See [licensing](docs/licensing.md).

## Development

```sh
cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
npm --prefix client ci && npm --prefix client run build && npm --prefix client run test:unit
```

End to end, with a real VSCodium (set `VSCODIUM_PATH` if it isn't in the default place) and a
real `remotium-host` started by the tests:

```sh
cargo build -p remotium-host
npm --prefix client run test:e2e:vsix   # install the VSIX, setup, restart, pair, connect
npm --prefix client run test:phase3     # terminal, port forwarding, debugging, host restart
npm --prefix client run test:browser    # Chromium: pair, workbench, edit a file
```

CI ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)) runs all of these on Linux, for
the VSCodium release pinned in [`compat.json`](compat.json) and the latest one (also weekly).
Releasing: [docs/releasing.md](docs/releasing.md).

## License

MIT. See [LICENSE](LICENSE) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
