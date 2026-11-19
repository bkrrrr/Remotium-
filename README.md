# OpenTunnel

An open-source alternative to VS Code Remote Tunnels for **VSCodium**, built so it
can be offered as a service.

Your desktop VSCodium connects to a VSCodium remote server (REH) running in a dev
container. Both the editor and the container only open **outbound** HTTPS/WSS
connections to a broker that runs on any server with a DNS name and TLS. There is
no vendor dependency.

```
VSCodium ── opentunnel.client ──WSS──► broker ◄──WSS (yamux)── agent ──► codium-server (REH)
```

- `crates/broker`: control plane (tickets, tokens, sessions, dashboard) and data plane (stream routing).
- `crates/agent`: runs in the dev container, dials the broker, manages REH installs per editor commit.
- `crates/protocol`: shared framing and the WebSocket ⇄ byte-stream adapter.
- `client/`: the VSCodium extension (`opentunnel.client`), a remote authority resolver using `ManagedResolvedAuthority`.
- `deploy/`: Docker Compose, the broker config, example reverse-proxy setups.

Docs: [architecture](docs/architecture.md) · [protocol](docs/protocol.md) ·
[security](docs/security.md) · [compatibility](docs/compatibility.md) ·
[licensing](docs/licensing.md) · [feasibility](docs/feasibility.md)

## Build

```sh
cargo build --workspace
npm --prefix client ci && npm --prefix client run build
```

## License

MIT. See [LICENSE](LICENSE) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
