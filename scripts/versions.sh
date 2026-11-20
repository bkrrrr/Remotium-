#!/bin/sh
# Print the versions of both parts as shell assignments: host=… client=…
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
host=$(sed -n '/^\[workspace.package\]/,/^\[/ s/^version = "\(.*\)"/\1/p' "$root/Cargo.toml")
client=$(sed -n 's/^  "version": "\(.*\)",$/\1/p' "$root/client/package.json" | head -1)
for v in "$host" "$client"; do
  echo "$v" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$' || { echo "bad version: '$v'" >&2; exit 1; }
done
echo "host=$host"
echo "client=$client"
