#!/bin/sh
# Set the release version everywhere it lives: the Cargo workspace and the client extension.
#   scripts/set-version.sh 0.2.0
set -eu
v="${1:?usage: scripts/set-version.sh <version>}"
echo "$v" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$' || { echo "not a semver version: $v" >&2; exit 1; }
root=$(cd "$(dirname "$0")/.." && pwd)
sed -i.bak -E "/^\[workspace.package\]/,/^\[/ s/^version = \".*\"/version = \"$v\"/" "$root/Cargo.toml" && rm "$root/Cargo.toml.bak"
(cd "$root/client" && npm version "$v" --no-git-tag-version --allow-same-version >/dev/null)
(cd "$root" && cargo metadata --format-version 1 >/dev/null)   # refresh Cargo.lock
echo "version set to $v; commit, then: git tag v$v && git push origin v$v"
