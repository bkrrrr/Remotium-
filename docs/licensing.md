# Licensing

## Why VSCodium only

Remotium is meant to be offered as a **hosted service**: a Remotium Server that many
users connect through, with dev containers that run a remote extension host
(REH) for them.

- **Microsoft VS Code Server** is distributed under the Microsoft Software
  License for VS Code Server. It does not allow hosting the server or offering
  it "for others to use" as a service. Baking it into images we redistribute is
  not allowed either.
- **Microsoft VS Code desktop** only accepts a server with exactly its own build
  commit, which means only Microsoft's server. So it can't be paired with an
  MIT-licensed server.
- **VSCodium** builds both the desktop editor and the REH
  (`vscodium-reh-<os>-<arch>-<version>.tar.gz`) from the MIT-licensed Code - OSS
  sources. We may host it, bake it into images and redistribute it, as long as
  the license notices are kept.

Extensions come from [Open VSX](https://open-vsx.org), VSCodium's default
gallery. Microsoft-only extensions (Pylance, MS C#, Live Share, …) are licensed
for Microsoft products only and aren't supported.

## What we ship

| Component | License | Notes |
|-----------|---------|-------|
| server, agent, protocol crates | MIT | this repository |
| client extension (`remotium.client`) | MIT | contains code adapted from open-remote-ssh (MIT) and a copy of the `resolvers` proposal typings from Code - OSS (MIT) |
| VSCodium REH in the `devenv` image | MIT | downloaded unchanged from VSCodium releases; its `LICENSE.txt` and `ThirdPartyNotices.txt` stay in the server directory |

See [`THIRD_PARTY_NOTICES.md`](../THIRD_PARTY_NOTICES.md) for the attributions.

## Obligations when you redistribute

- Keep `LICENSE` and `THIRD_PARTY_NOTICES.md` with the binaries and the VSIX.
- Keep the server directory's license files when you bake a REH into an image.
- Don't use the Microsoft "Visual Studio Code" trademarks or logos for your
  service.
