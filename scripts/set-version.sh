#!/bin/sh
# Set the version of server and client. Pushing a new version to main releases it.
#   scripts/set-version.sh 0.2.0
set -eu
v="${1:?usage: scripts/set-version.sh <version>}"
echo "$v" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$' || { echo "not a semver version: $v" >&2; exit 1; }
root=$(cd "$(dirname "$0")/.." && pwd)
sed -i.bak -E "/^\[workspace.package\]/,/^\[/ s/^version = \".*\"/version = \"$v\"/" "$root/Cargo.toml" && rm "$root/Cargo.toml.bak"
(cd "$root" && cargo metadata --format-version 1 >/dev/null)   # refresh Cargo.lock
(cd "$root/client" && npm version "$v" --no-git-tag-version --allow-same-version >/dev/null)
echo "version set to $v; commit and push to main to release it"
