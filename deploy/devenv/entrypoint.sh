#!/bin/sh
# Keep the agent running without restarting the container. Servers started by the agent
# outlive it and are adopted by the next agent process, so editor sessions survive an agent
# crash, restart or upgrade.
trap 'kill -TERM "$pid" 2>/dev/null; wait "$pid"; exit 0' TERM INT
while :; do
  /usr/local/bin/opentunnel-agent "$@" &
  pid=$!
  wait "$pid"
  code=$?
  echo "opentunnel-agent exited with $code; restarting in 2s" >&2
  sleep 2
done
