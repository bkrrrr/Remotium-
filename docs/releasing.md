# Releasing

A release is a version tag. `.github/workflows/release.yml` runs the full CI, then publishes:

| What | Where | Needs |
|---|---|---|
| `remotium-server` and `remotium-devenv` images (amd64 + arm64) | `ghcr.io/<owner>/remotium-server:<version>`, `…/remotium-devenv:<version>`, plus `latest` for non-pre-releases | nothing (the workflow's `GITHUB_TOKEN`) |
| GitHub Release: `remotium-client-<version>.vsix`, static `remotium-server` (musl) and `remotium-agent` (glibc) for x86_64 and aarch64, `SHA256SUMS` | the repository's Releases page | nothing |
| `remotium.client` | [Open VSX](https://open-vsx.org/extension/remotium/client) | the `OVSX_PAT` secret (skipped with a notice when missing) |

After publishing to Open VSX the workflow runs a **store test**: it installs
`remotium.client@<version>` from Open VSX into a fresh VSCodium profile and runs the
first-install e2e (setup, restart, remote window) against a local stack.

## One-time setup

1. **GitHub repository.** Push this repository to GitHub. Packages (GHCR) need no setup; after
   the first release, set the two packages to public under *Packages → Package settings* if
   users should pull without logging in.
2. **Open VSX** (optional, for "search Remotium in the Extensions view"):
   1. Sign in at <https://open-vsx.org> with GitHub and sign the Eclipse Publisher Agreement
      (profile page).
   2. Create an access token under *Settings → Access Tokens*.
   3. Create the namespace: `npx ovsx create-namespace remotium -p <token>`.
      Optionally claim ownership of it (Open VSX docs: "namespace access"), which shows the
      extension as verified.
   4. Add the token as the repository secret `OVSX_PAT` (*Settings → Secrets and variables →
      Actions*).
3. Add a `repository` field to `client/package.json` pointing at the GitHub repository, so
   Open VSX links to the source.

## Each release

```sh
scripts/set-version.sh 0.2.0       # Cargo workspace + client/package.json (+ Cargo.lock)
git commit -am "Release 0.2.0"
git tag v0.2.0
git push origin main v0.2.0
```

`scripts/check-version.sh v0.2.0` is what the workflow runs first; a tag that doesn't match
the versions in the tree fails the release before anything is built. Versions with a suffix
(`0.2.0-rc.1`) are published as pre-releases (GitHub pre-release, Open VSX pre-release, no
`latest` image tag).

## After a release

- Servers that run the image offer the new client at `/downloads/remotium-client.vsix`
  right away; users re-run the install command or update from Open VSX.
- Once the client is on Open VSX, set `client_on_open_vsx = true` in `server.toml`: the
  dashboard then shows a one-click install link (`vscodium:extension/remotium.client`).
- Update `compat.json` when a new VSCodium release is tested (the weekly CI run against the
  latest VSCodium tells you when something breaks).
