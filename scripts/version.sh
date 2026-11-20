#!/bin/sh
# Print the version. Server (Cargo.toml) and client (client/package.json) share it; fail if not.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
server=$(sed -n '/^\[workspace.package\]/,/^\[/ s/^version = "\(.*\)"/\1/p' "$root/Cargo.toml")
client=$(sed -n 's/^  "version": "\(.*\)",$/\1/p' "$root/client/package.json" | head -1)
if [ "$server" != "$client" ]; then
  echo "version mismatch: Cargo.toml has '$server', client/package.json has '$client' (use scripts/set-version.sh)" >&2
  exit 1
fi
echo "$server" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$' || { echo "not a semver version: '$server'" >&2; exit 1; }
echo "$server"
