# Licensing

## Why VSCodium only

remotium-host runs a VSCodium remote server on a machine and lets browsers and desktop
editors use it.

- **Microsoft VS Code Server** is under the Microsoft Software License for VS Code Server,
  which restricts hosting it and offering it for others to use, and redistributing it.
- **Microsoft VS Code desktop** only accepts a server with exactly its own build commit, which
  means only Microsoft's server, so it can't be paired with an MIT-licensed one.
- **VSCodium** builds the desktop editor and the remote server
  (`vscodium-reh-web-<os>-<arch>-<version>.tar.gz`) from the MIT-licensed Code - OSS sources.
  Running, hosting and redistributing it is fine as long as the license notices are kept.

Extensions come from [Open VSX](https://open-vsx.org), VSCodium's default gallery.
Microsoft-only extensions (Pylance, C# Dev Kit, Live Share, …) are licensed for Microsoft
products only.

## What we ship

| Component | License | Notes |
|-----------|---------|-------|
| remotium-host | MIT | this repository; release binaries embed the client package |
| Remotium Client (`remotium.client`) | MIT | contains code adapted from open-remote-ssh (MIT) and a copy of the `resolvers` proposal typings from Code - OSS (MIT) |
| VSCodium remote server | MIT | downloaded by the host from VSCodium's releases at run time, unchanged; its `LICENSE.txt` and `ThirdPartyNotices.txt` stay in the server directory |

See [`THIRD_PARTY_NOTICES.md`](../THIRD_PARTY_NOTICES.md).

## When you redistribute

- Keep `LICENSE` and `THIRD_PARTY_NOTICES.md` with the binaries and the VSIX.
- If you bake a VSCodium server into an image, keep its license files.
- Don't use Microsoft's "Visual Studio Code" trademarks or logos.
