#!/bin/sh
# Fail unless the Cargo workspace, the client and (optionally) a tag agree on the version.
#   scripts/check-version.sh [v0.2.0]
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
cargo_v=$(sed -n '/^\[workspace.package\]/,/^\[/ s/^version = "\(.*\)"/\1/p' "$root/Cargo.toml")
client_v=$(node -p "require('$root/client/package.json').version")
echo "Cargo: $cargo_v  client: $client_v${1:+  tag: $1}"
[ "$cargo_v" = "$client_v" ] || { echo "Cargo and client versions differ" >&2; exit 1; }
if [ -n "${1:-}" ] && [ "${1#v}" != "$cargo_v" ]; then echo "tag $1 doesn't match version $cargo_v" >&2; exit 1; fi
echo "$cargo_v"
