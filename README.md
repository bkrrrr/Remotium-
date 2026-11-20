# Remotium

An open-source alternative to VS Code Remote Tunnels for **VSCodium**, built so it can be
offered as a service.

Your desktop VSCodium connects to a VSCodium remote server (REH) running in a dev
container. Both the editor and the container only open **outbound** HTTPS/WSS connections to
**Remotium Server**, which runs on any machine with a DNS name and TLS. Containers behind NAT or CGNAT
work. There is no vendor dependency.

```
VSCodium ── remotium.client ──WSS──► server ◄──WSS (yamux)── agent ──► codium-server (REH)
```

The editor's own remote machinery does the work (files, terminals, debugging, extensions,
port forwarding, reconnection). Remotium only sets up the connection and carries the bytes.

| Part | What it does |
|------|--------------|
| [`crates/server`](crates/server) | Control plane (dashboard, tickets, tokens, sessions) and data plane (stream routing). Rust. |
| [`crates/agent`](crates/agent) | Runs in the dev container, dials the server, installs and runs one REH per editor version. Rust. |
| [`crates/protocol`](crates/protocol) | Shared framing, messages, the WebSocket ⇄ byte-stream adapter, yamux driver. |
| [`client`](client) | The VSCodium extension `remotium.client`: a remote authority resolver using `ManagedResolvedAuthority`. TypeScript. |
| [`deploy`](deploy) | Docker Compose, Dockerfiles, server configs, reverse-proxy examples. |

Docs: [architecture](docs/architecture.md) · [protocol](docs/protocol.md) ·
[security](docs/security.md) · [compatibility](docs/compatibility.md) ·
[licensing](docs/licensing.md) · [feasibility](docs/feasibility.md)

Only VSCodium is supported. Microsoft VS Code needs Microsoft's server, whose license
doesn't allow running it as a service. See [licensing](docs/licensing.md).

## Installing Remotium Client

Users install the client once per computer. Any of these works:

| Way | How |
|---|---|
| From your server's dashboard | The dashboard shows a one-line command for the visitor's OS, e.g. `curl -fsSL https://<server>/install.sh \| sh` or `irm https://<server>/install.ps1 \| iex`. It downloads the client from that server, checks its SHA-256 and installs it with the `codium` CLI. A **download** link is there too: drag the file onto VSCodium's Extensions view. |
| Open VSX (once published) | Search **Remotium** in VSCodium's Extensions view, or `codium --install-extension remotium.client`. |
| GitHub Releases | Download `remotium-client-<version>.vsix` from the release page and drag it onto the Extensions view. |

On first start, Remotium asks once to finish its setup: VSCodium has to allow its remote
connection API (the `resolvers` proposal). Accept, then restart VSCodium. Under the hood this
adds `"enable-proposed-api": ["remotium.client"]` to `~/.vscode-oss/argv.json`, keeping the
rest of the file; you can also add that line yourself via *Preferences: Configure Runtime
Arguments*.

Microsoft VS Code can install the file but can't connect: it only accepts Microsoft's own
server, which may not be offered as a service.

## Quick start (everything local)

Requirements: Docker and VSCodium (the version pinned in [`compat.json`](compat.json) is
pre-baked in the image; other versions are downloaded on first connect).

```sh
docker compose -f deploy/docker-compose.yml up --build -d
```

1. Open <http://localhost:8080>. The `demo` environment shows as online.
2. Install the client using the command shown under **Get the client**, start VSCodium and
   accept the one-time setup, then restart VSCodium.
3. Click **Open in VSCodium**. VSCodium asks once whether to trust the server, then opens a
   remote window on `/workspace/demo` inside the container.

Check that it works by editing `README.md` in that window, then running:

```sh
docker compose -f deploy/docker-compose.yml exec devenv cat /workspace/demo/README.md
```

If the browser can't open `vscodium://` links, copy the link from the "Opening VSCodium" page
and run **Remotium: Open Launch Link** in VSCodium.

A second environment: `docker compose -f deploy/docker-compose.yml --profile multi up -d`
starts `devenv2` as env `demo2`.

## Running Remotium Server on a VPS

Any VPS with a DNS name works. The server serves HTTPS itself, or plain HTTP behind a
reverse proxy.

1. Build or copy the server: `docker build -f deploy/Dockerfile.server -t remotium-server .`
   (a static binary on Alpine), or `cargo build --release -p remotium-server`.
2. Get a certificate, for example with certbot:
   `certbot certonly --standalone -d tunnels.example.com`.
3. Copy [`deploy/server.example.toml`](deploy/server.example.toml) to `server.toml`. Set
   `public_url`, `tls_cert` and `tls_key` (the server reloads renewed certificates every 6
   hours), and configure an auth proxy (see below).
4. For each environment, run `remotium-server gen-token`. Put the hash in an `[[env]]`
   block and give the token to that environment's agent. Add `[[grant]]` blocks for users.
5. `remotium-server -c server.toml check-config`, then `remotium-server -c server.toml`.

Start a devenv anywhere (a laptop, a NAT'd box), pointing it at the server:

```sh
REMOTIUM_SERVER_URL=https://tunnels.example.com AGENT_ENV_ID=alice-dev AGENT_TOKEN=rmag_... \
  docker compose -f deploy/docker-compose.yml up -d devenv
```

### Authentication

The server doesn't log users in itself (yet). An auth proxy in front of it does and passes
the identity on. The server trusts it only from `trusted_proxies`:

- **nginx + oauth2-proxy** (any OIDC provider): [`deploy/examples/nginx.conf`](deploy/examples/nginx.conf).
- **Signed assertions** (e.g. Cloudflare Access): `[auth.jwt]` verifies them against a JWKS.
  See [`deploy/examples/cloudflare-tunnel.md`](deploy/examples/cloudflare-tunnel.md).

Only the dashboard, `/launch`, `/api/v1/environments` and `/api/v1/launch-tickets` need
the proxy login. `/api/v1/tokens*`, `/api/v1/sessions*`, `/data/*` and `/agent/*` must
bypass it, because they use the server's own tokens. See [security](docs/security.md).

## Agent configuration

| Variable | Default | |
|----------|---------|---|
| `REMOTIUM_SERVER_URL` | (required) | `https://…` or `http://…` |
| `AGENT_ENV_ID` | (required) | env id from `server.toml` |
| `AGENT_TOKEN` / `AGENT_TOKEN_FILE` | (required) | the env's agent token |
| `AGENT_PROJECTS` | dirs in the workspace root | comma separated |
| `AGENT_WORKSPACE_ROOT` | `/workspace` | |
| `AGENT_DATA_DIR` | `/data` | REH cache, tokens, extensions |
| `AGENT_REH_URL_TEMPLATE` | VSCodium GitHub releases | `${version}`, `${release}`, `${releaseSuffix}`, `${os}`, `${arch}`, `${commit}`, `${quality}` |
| `AGENT_REH_SEARCH_PATH` | `/opt/vscodium-server` in the image | pre-installed servers as `<dir>/<commit>` |
| `AGENT_ALLOW_UNVERIFIED_DOWNLOADS` | off | accept downloads without a `.sha256` |
| `AGENT_IDLE_TIMEOUT_SECS` | 1800 | stop an REH with no connections |
| `AGENT_REAP_AFTER_DAYS` | 14 | delete unused downloaded REHs |

## Development

```sh
cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
npm --prefix client run build
```

End-to-end with a real VSCodium (needs the compose stack running):

```sh
npm --prefix client run test:e2e
```

CI ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)) runs the Rust tests and clippy,
the client unit tests, and packages the VSIX. Against the compose stack, in VSCodium under
Xvfb (pinned and latest release, also weekly), it runs the first-install test with the VSIX
the server serves, `install.sh`, and the phase 3 suite. `install.sh` and `install.ps1` are
also tested on macOS and Windows.

```sh
npm --prefix client run test:unit       # argv.json handling
npm --prefix client run test:e2e:vsix   # install the served VSIX, setup, restart, connect
npm --prefix client run test:phase3     # terminals, ports, debugging, Open VSX, restarts
```

Releasing: see [docs/releasing.md](docs/releasing.md).

## License

MIT. See [LICENSE](LICENSE) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
