# OpenTunnel

An open-source alternative to VS Code Remote Tunnels for **VSCodium**, built so it can be
offered as a service.

Your desktop VSCodium connects to a VSCodium remote server (REH) running in a dev
container. Both the editor and the container only open **outbound** HTTPS/WSS connections to
a broker that runs on any server with a DNS name and TLS. Containers behind NAT or CGNAT
work. There is no vendor dependency.

```
VSCodium ── opentunnel.client ──WSS──► broker ◄──WSS (yamux)── agent ──► codium-server (REH)
```

The editor's own remote machinery does the work (files, terminals, debugging, extensions,
port forwarding, reconnection). OpenTunnel only sets up the connection and carries the bytes.

| Part | What it does |
|------|--------------|
| [`crates/broker`](crates/broker) | Control plane (dashboard, tickets, tokens, sessions) and data plane (stream routing). Rust. |
| [`crates/agent`](crates/agent) | Runs in the dev container, dials the broker, installs and runs one REH per editor version. Rust. |
| [`crates/protocol`](crates/protocol) | Shared framing, messages, the WebSocket ⇄ byte-stream adapter, yamux driver. |
| [`client`](client) | The VSCodium extension `opentunnel.client`: a remote authority resolver using `ManagedResolvedAuthority`. TypeScript. |
| [`deploy`](deploy) | Docker Compose, Dockerfiles, broker configs, reverse-proxy examples. |

Docs: [architecture](docs/architecture.md) · [protocol](docs/protocol.md) ·
[security](docs/security.md) · [compatibility](docs/compatibility.md) ·
[licensing](docs/licensing.md) · [feasibility](docs/feasibility.md)

Only VSCodium is supported. Microsoft VS Code needs Microsoft's server, whose license
doesn't allow running it as a service. See [licensing](docs/licensing.md).

## Quick start (everything local)

Requirements: Docker, VSCodium (the version pinned in [`compat.json`](compat.json) is
pre-baked in the image; other versions are downloaded on first connect), Node.js 20+ to
build the extension.

```sh
docker compose -f deploy/docker-compose.yml up --build -d
npm --prefix client ci && npm --prefix client run package   # → client/opentunnel-client.vsix
```

1. **Install the extension** in VSCodium: *Extensions → … → Install from VSIX…* and pick
   `client/opentunnel-client.vsix`, or run
   `codium --install-extension client/opentunnel-client.vsix`.
2. **Allow its proposed API.** The extension uses the `resolvers` API proposal. Run
   *Preferences: Configure Runtime Arguments*, which opens `~/.vscode-oss/argv.json`, add the
   line below, and restart VSCodium:
   ```jsonc
   "enable-proposed-api": ["opentunnel.client"]
   ```
3. Open <http://localhost:8080>. The `demo` environment shows as online.
4. Click **Open in VSCodium**. VSCodium asks once whether to trust the broker, then opens a
   remote window on `/workspace/demo` inside the container.

Check that it works by editing `README.md` in that window, then running:

```sh
docker compose -f deploy/docker-compose.yml exec devenv cat /workspace/demo/README.md
```

If the browser can't open `vscodium://` links, copy the link from the "Opening VSCodium" page
and run **OpenTunnel: Open Launch Link** in VSCodium.

A second environment: `docker compose -f deploy/docker-compose.yml --profile multi up -d`
starts `devenv2` as env `demo2`.

## Running the broker on a server

Any VPS with a DNS name works. The broker serves HTTPS itself, or plain HTTP behind a
reverse proxy.

1. Build or copy the broker: `docker build -f deploy/Dockerfile.broker -t opentunnel-broker .`
   (a static binary on Alpine), or `cargo build --release -p opentunnel-broker`.
2. Get a certificate, for example with certbot:
   `certbot certonly --standalone -d tunnels.example.com`.
3. Copy [`deploy/broker.example.toml`](deploy/broker.example.toml) to `broker.toml`. Set
   `public_url`, `tls_cert` and `tls_key` (the broker reloads renewed certificates every 6
   hours), and configure an auth proxy (see below).
4. For each environment, run `opentunnel-broker gen-token`. Put the hash in an `[[env]]`
   block and give the token to that environment's agent. Add `[[grant]]` blocks for users.
5. `opentunnel-broker -c broker.toml check-config`, then `opentunnel-broker -c broker.toml`.

Start a devenv anywhere (a laptop, a NAT'd box), pointing it at the server:

```sh
BROKER_URL=https://tunnels.example.com AGENT_ENV_ID=alice-dev AGENT_TOKEN=otag_... \
  docker compose -f deploy/docker-compose.yml up -d devenv
```

### Authentication

The broker doesn't log users in itself (yet). An auth proxy in front of it does and passes
the identity on. The broker trusts it only from `trusted_proxies`:

- **nginx + oauth2-proxy** (any OIDC provider): [`deploy/examples/nginx.conf`](deploy/examples/nginx.conf).
- **Signed assertions** (e.g. Cloudflare Access): `[auth.jwt]` verifies them against a JWKS.
  See [`deploy/examples/cloudflare-tunnel.md`](deploy/examples/cloudflare-tunnel.md).

Only the dashboard, `/launch`, `/api/v1/environments` and `/api/v1/launch-tickets` need
the proxy login. `/api/v1/tokens*`, `/api/v1/sessions*`, `/data/*` and `/agent/*` must
bypass it, because they use the broker's own tokens. See [security](docs/security.md).

## Agent configuration

| Variable | Default | |
|----------|---------|---|
| `BROKER_URL` | (required) | `https://…` or `http://…` |
| `AGENT_ENV_ID` | (required) | env id from `broker.toml` |
| `AGENT_TOKEN` / `AGENT_TOKEN_FILE` | (required) | the env's agent token |
| `AGENT_PROJECTS` | dirs in the workspace root | comma separated |
| `AGENT_WORKSPACE_ROOT` | `/workspace` | |
| `AGENT_DATA_DIR` | `/data` | server cache, tokens, extensions |
| `AGENT_SERVER_URL_TEMPLATE` | VSCodium GitHub releases | `${version}`, `${release}`, `${releaseSuffix}`, `${os}`, `${arch}`, `${commit}`, `${quality}` |
| `AGENT_SERVER_SEARCH_PATH` | `/opt/vscodium-server` in the image | pre-installed servers as `<dir>/<commit>` |
| `AGENT_ALLOW_UNVERIFIED_DOWNLOADS` | off | accept downloads without a `.sha256` |
| `AGENT_IDLE_TIMEOUT_SECS` | 1800 | stop a server with no connections |
| `AGENT_REAP_AFTER_DAYS` | 14 | delete unused downloaded servers |

## Development

```sh
cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
npm --prefix client run build
```

End-to-end with a real VSCodium (needs the compose stack running):

```sh
npm --prefix client run test:e2e
```

Without a broker, the M0 direct mode connects VSCodium straight to an agent. See
[feasibility](docs/feasibility.md).

## License

MIT. See [LICENSE](LICENSE) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
