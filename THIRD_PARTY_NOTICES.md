# Third-party notices

## open-remote-ssh
Parts of `client/src/productInfo.ts` and the server download template logic in
`crates/agent/src/reh.rs` are adapted from
[jeanp413/open-remote-ssh](https://github.com/jeanp413/open-remote-ssh).

    MIT License
    Copyright (c) 2022 (open-remote-ssh authors)

    Permission is hereby granted, free of charge, to any person obtaining a copy
    of this software and associated documentation files (the "Software"), to deal
    in the Software without restriction, including without limitation the rights
    to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
    copies of the Software, and to permit persons to whom the Software is
    furnished to do so, subject to the following conditions:

    The above copyright notice and this permission notice shall be included in all
    copies or substantial portions of the Software.

    THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
    IMPLIED. (full text: https://opensource.org/license/mit)

## Visual Studio Code (Code - OSS)
`client/src/vscode.proposed.resolvers.d.ts` is copied unchanged from
[microsoft/vscode](https://github.com/microsoft/vscode) tag 1.143.1.
Copyright (c) Microsoft Corporation. MIT License.

## VSCodium
The `devenv` image downloads and bundles the VSCodium remote extension host
(`vscodium-reh-*`), built from Code - OSS by the
[VSCodium](https://github.com/VSCodium/vscodium) project. MIT License.
Its own license and third-party notice files ship inside the extracted server
directory and are kept unchanged.
