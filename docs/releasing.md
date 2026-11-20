# Releasing

Server (remotium-host) and client (Remotium Client) are one product with one version,
in `Cargo.toml` and `client/package.json`. `scripts/version.sh` prints it and fails if the two
differ.

## How it works

Everything is one workflow, `.github/workflows/ci.yml`:

1. **Server · tests** and **Client · tests and package**: Rust tests and clippy, client unit
   tests, the VSIX.
2. **Server · build**: the release binaries (Linux x86_64/aarch64 static musl, macOS
   x86_64/aarch64), each with the client embedded.
3. **End-to-end**: real VSCodium and Chromium against **those** files: the Linux binary and
   the VSIX that a release would publish.
4. **Release** (pushes to `main` only): if there's no tag `v<version>` yet, it publishes the
   tested files as GitHub Release `v<version>` (binaries, `remotium-client-<version>.vsix`,
   `SHA256SUMS`). Otherwise it does nothing.
5. **Release · client to Open VSX** (only after a release, and only with the `OVSX_PAT`
   secret): publishes `remotium.client`, then installs it from Open VSX and connects to the
   released server binary.

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
