# Running remotium-host

```
remotium-host [--folder DIR] [--listen 0.0.0.0:8443] [--name NAME…] [--public-url URL]
              [--tls-cert PEM --tls-key PEM | --plain-http] [--data-dir DIR]
remotium-host pair [--name LABEL]     # one-time pairing link (10 minutes)
remotium-host devices                 # paired browsers and VSCodiums
remotium-host revoke ID               # remove one
```

Every option also has an environment variable (`REMOTIUM_FOLDER`, `REMOTIUM_LISTEN`, …; see
`remotium-host --help`). State lives in `~/.remotium-host` (or `--data-dir`): the certificate,
paired devices (hashed), and the downloaded VSCodium servers (about 320 MB per version).

## Certificate

- **Self-signed (default).** Created on first start for the host name, `localhost` and the
  machine's IP addresses; add names with `--name dev.example.lan`. Pairing links for VSCodium
  carry its SHA-256, and the client pins it. Browsers show a warning once; compare the
  fingerprint `remotium-host pair` prints.
- **Your own** (e.g. Let's Encrypt): `--tls-cert fullchain.pem --tls-key privkey.pem`. Renewed
  files are picked up every 6 hours. Pairing links then carry no fingerprint and clients use
  normal certificate validation.
- **Behind your own HTTPS proxy or tunnel:** `--plain-http --listen 127.0.0.1:8080 --public-url https://dev.example.com`.
  Plain HTTP on a non-loopback address also needs `--plain-http-any-address`.

## Reaching the host

- **Same network or VPN:** nothing to do. With Tailscale, `--name <machine>.<tailnet>.ts.net`
  adds the MagicDNS name to the certificate.
- **Caddy** (automatic HTTPS):
  ```
  dev.example.com {
      reverse_proxy 127.0.0.1:8080
  }
  ```
- **nginx:**
  ```nginx
  location / {
      proxy_pass http://127.0.0.1:8080;
      proxy_http_version 1.1;
      proxy_set_header Upgrade $http_upgrade;
      proxy_set_header Connection "upgrade";
      proxy_set_header Host $host;
      proxy_set_header X-Forwarded-Proto https;
      proxy_read_timeout 1h;
  }
  ```
- **Cloudflare Tunnel:** `cloudflared tunnel --url http://127.0.0.1:8080`, with
  `--public-url` set to the tunnel's hostname.

Keep a proxy's idle timeout above 30 s: the host pings every 25 s.

## As a service (systemd)

```ini
# ~/.config/systemd/user/remotium-host.service
[Unit]
Description=remotium-host

[Service]
ExecStart=%h/bin/remotium-host --folder %h/project
Restart=on-failure

[Install]
WantedBy=default.target
```

`systemctl --user enable --now remotium-host`, then `journalctl --user -u remotium-host` shows the
first pairing link. VSCodium servers keep running across a restart of the host, so editor
windows and browser tabs reconnect without losing their session.

## In a container

The Linux binaries are static, so any image works. Publish the port, mount the data dir and
your code:

```dockerfile
FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates git && rm -rf /var/lib/apt/lists/*
COPY remotium-host /usr/local/bin/
ENTRYPOINT ["remotium-host", "--data-dir", "/data", "--folder", "/workspace", "--name", "localhost"]
```

```sh
docker run -d -p 8443:8443 -v remotium:/data -v "$PWD:/workspace" my-dev-image
docker exec <container> remotium-host --data-dir /data pair
```

The VSCodium server needs glibc (Debian, Ubuntu, Fedora, …) or musl (Alpine); the host picks
the right download.
