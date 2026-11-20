#!/bin/sh
# Actions the phase 3 suite asks for, against the compose stack.
set -eu
compose="docker compose -f $(dirname "$0")/../../../deploy/docker-compose.yml"
case "$1" in
  restart-server) $compose restart server ;;
  restart-agent)  $compose exec -T devenv pkill -f /usr/local/bin/remotium-agent ;;
  *) echo "unknown action $1" >&2; exit 2 ;;
esac
