# Optional: Cloudflare Tunnel and Cloudflare Access

Nothing in Remotium depends on Cloudflare. If you have no public server, any HTTP
tunnel or ingress can expose the server. This page shows Cloudflare Tunnel as one example,
with Cloudflare Access as the auth proxy.

## 1. Expose the server

```sh
cloudflared tunnel create remotium
cloudflared tunnel route dns remotium tunnels.example.com
```

`~/.cloudflared/config.yml`:

```yaml
tunnel: remotium
credentials-file: /home/me/.cloudflared/<tunnel-id>.json
ingress:
  - hostname: tunnels.example.com
    service: http://127.0.0.1:8080
  - service: http_status:404
```

WebSockets work through Cloudflare Tunnel without extra settings. The server's 25 s pings
keep the connections well under Cloudflare's idle timeout.

## 2. Protect the dashboard with Access

Create a self-hosted Access application for `tunnels.example.com` with your identity
provider and policy. Then add **bypass** policies (or a second application with a Bypass
action) for the paths that use the server's own tokens:

- `tunnels.example.com/api/v1/tokens*`
- `tunnels.example.com/api/v1/sessions*`
- `tunnels.example.com/api/v1/status`
- `tunnels.example.com/data/*`
- `tunnels.example.com/agent/*`
- `tunnels.example.com/healthz`

## 3. Verify the Access assertion in the server

`cloudflared` connects from loopback, so trust that address:

```toml
[server]
listen = "127.0.0.1:8080"
public_url = "https://tunnels.example.com"

[auth]
trusted_proxies = ["127.0.0.1/32", "::1/128"]

[auth.jwt]
header = "Cf-Access-Jwt-Assertion"
jwks_url = "https://<team>.cloudflareaccess.com/cdn-cgi/access/certs"
audience = "<the application's AUD tag>"
issuer = "https://<team>.cloudflareaccess.com"
email_claim = "email"
```

The server checks the signature, audience, issuer and expiry of every assertion and uses its
`email` claim as the user. Grants in `server.toml` work as usual.
