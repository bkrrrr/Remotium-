# Releasing

A release is a version tag. `.github/workflows/release.yml` runs the full CI, then publishes:

| What | Where | Needs |
|---|---|---|
| `remotium-host` for linux-x86_64, linux-aarch64 (static musl), macos-x86_64, macos-aarch64, each with Remotium Client embedded | GitHub Release | nothing |
| `remotium-client-<version>.vsix`, `SHA256SUMS` | GitHub Release | nothing |
| `remotium.client` | [Open VSX](https://open-vsx.org/extension/remotium/client) | the `OVSX_PAT` secret (skipped with a notice when missing) |

After publishing to Open VSX, a **store test** installs `remotium.client@<version>` from Open
VSX into a fresh VSCodium profile and runs the first-install e2e (setup, restart, pair,
connect) against a local remotium-host.

## One-time setup

1. **GitHub repository.** Push this repository to GitHub.
2. **Open VSX** (optional, for "search Remotium in the Extensions view"):
   1. Sign in at <https://open-vsx.org> with GitHub and accept the publisher agreement on
      your profile page.
   2. Create an access token under *Settings → Access Tokens*.
   3. Create the namespace: `npx ovsx create-namespace remotium -p <token>`.
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

`scripts/check-version.sh v0.2.0` runs first in the workflow; a tag that doesn't match the
versions in the tree stops the release before anything is built. Versions with a suffix
(`0.2.0-rc.1`) become pre-releases on GitHub and Open VSX.

## After a release

- Update `compat.json` when a new VSCodium release is tested. The weekly CI run against the
  latest VSCodium shows when something breaks; `compat.json` also decides which version
  browsers get.
