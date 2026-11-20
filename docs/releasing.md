# Releasing

Server (remotium-host) and client (Remotium Client) are one product with one version,
in `Cargo.toml` and `client/package.json`. `scripts/version.sh` prints it and fails if the two
differ.

## How it works

Everything is one workflow, `.github/workflows/ci.yml`:

1. **Server · unit tests from source** and **Client · unit tests + package VSIX**: Rust tests
   and clippy (compiled from source by the job itself), client unit tests, the VSIX.
2. **Server · release binary**: the release binaries (Linux x86_64/aarch64 static musl, macOS
   x86_64/aarch64), each with the client embedded.
3. **End-to-end on release files**: real VSCodium and Chromium against **those** files: the Linux binary and
   the VSIX that a release would publish.
4. **Release v&lt;version&gt;** (pushes to `main` only, and only if tag `v<version>` doesn't exist
   yet; otherwise the job shows as skipped). Its steps:
   - **Publish to GitHub Releases**: the tested binaries, `remotium-client-<version>.vsix`,
     `SHA256SUMS`;
   - **Publish client to Open VSX** and **Check the client from Open VSX installs and
     connects** (both only with the `OVSX_PAT` secret, otherwise skipped).

Nothing is built or tested twice, and ordinary pushes publish nothing.

## Release

```sh
scripts/set-version.sh 0.2.0
git commit -am "Release 0.2.0"
git push origin main
```

Versions with a suffix (`0.2.0-rc.1`) become pre-releases.

## One-time setup for Open VSX (optional)

1. Sign in at <https://open-vsx.org> with GitHub and accept the publisher agreement on your
   profile page.
2. Create an access token under *Settings → Access Tokens*.
3. Create the namespace: `npx ovsx create-namespace remotium -p <token>`.
4. Add the token as the repository secret `OVSX_PAT` (*Settings → Secrets and variables →
   Actions*).

## Keeping up with VSCodium

Update `compat.json` when a new VSCodium release is tested. The weekly CI run against the
latest VSCodium shows when something breaks; `compat.json` also decides which version browsers
get.
