#!/bin/sh
# Bump the version of one part. Pushing the bump to main publishes that part.
#   scripts/set-version.sh host 0.2.0     # remotium-host (Cargo workspace)
#   scripts/set-version.sh client 0.2.0   # Remotium Client (client/package.json)
set -eu
part="${1:?usage: scripts/set-version.sh host|client <version>}"
v="${2:?usage: scripts/set-version.sh host|client <version>}"
echo "$v" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$' || { echo "not a semver version: $v" >&2; exit 1; }
root=$(cd "$(dirname "$0")/.." && pwd)
case "$part" in
  host)
    sed -i.bak -E "/^\[workspace.package\]/,/^\[/ s/^version = \".*\"/version = \"$v\"/" "$root/Cargo.toml" && rm "$root/Cargo.toml.bak"
    (cd "$root" && cargo metadata --format-version 1 >/dev/null)   # refresh Cargo.lock
    ;;
  client)
    (cd "$root/client" && npm version "$v" --no-git-tag-version --allow-same-version >/dev/null)
    ;;
  *) echo "part must be host or client" >&2; exit 1 ;;
esac
echo "$part version set to $v; commit and push to main to publish it"
