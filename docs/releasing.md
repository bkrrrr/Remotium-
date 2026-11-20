# Releasing

Server and client are released separately, each when its version changes.

| Part | Version lives in | Tag | Published to |
|---|---|---|---|
| **Server**: remotium-host | `Cargo.toml` (`[workspace.package] version`) | `host-v<version>` | GitHub Release: binaries for linux-x86_64, linux-aarch64 (static musl), macos-x86_64, macos-aarch64 (each with the client embedded), `SHA256SUMS` |
| **Client**: Remotium Client | `client/package.json` (`version`) | `client-v<version>` | GitHub Release: `remotium-client-<version>.vsix`, `SHA256SUMS`; Open VSX if `OVSX_PAT` is set |

## How it works

`.github/workflows/release.yml` runs on every push to `main`:

1. **Which parts to publish:** reads both versions and looks for their tags. A part whose
   tag already exists is skipped. If both exist, the workflow stops here; ordinary pushes
   publish nothing.
2. **CI:** the full CI runs before anything is published.
3. **Server** (if bumped): builds the binaries and creates `host-v<version>`, marked as the
   latest release.
4. **Client** (if bumped): creates `client-v<version>` with the VSIX, then publishes to Open VSX
   and runs a **store test** (installs `remotium.client@<version>` from Open VSX into a fresh
   VSCodium and connects to a local remotium-host).

"Run workflow" on the Release workflow re-checks by hand (e.g. after a failed run: delete the
half-made tag/release first). Versions with a suffix (`0.2.0-rc.1`) become pre-releases.

## Release a part

```sh
scripts/set-version.sh host 0.2.0      # or: scripts/set-version.sh client 0.2.0
git commit -am "Release remotium-host 0.2.0"
git push origin main
```

`scripts/versions.sh` prints both current versions.

Server and client talk over the versioned `/remotium/v1` API, so any client works with any
host of the same API version. Release the client together with the server when you change
that API.

## One-time setup for Open VSX (optional)

1. Sign in at <https://open-vsx.org> with GitHub and accept the publisher agreement on your
   profile page.
2. Create an access token under *Settings → Access Tokens*.
3. Create the namespace: `npx ovsx create-namespace remotium -p <token>`.
4. Add the token as the repository secret `OVSX_PAT` (*Settings → Secrets and variables →
   Actions*).

Without the secret, client releases still go to GitHub and the Open VSX step is skipped with a
notice.

## Keeping up with VSCodium

Update `compat.json` when a new VSCodium release is tested. The weekly CI run against the
latest VSCodium shows when something breaks; `compat.json` also decides which version browsers
get. Changing it doesn't publish anything by itself; bump the server version to ship it.
