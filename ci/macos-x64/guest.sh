#!/usr/bin/env bash
# Lifecycle for the macOS guest.
#
# Two sources, in priority order:
#   1. $GUEST_IMAGE -- a prebaked GHCR image (what CI uses; no install needed)
#   2. $GUEST_STORAGE -- a local prepared volume (what a dev box uses)
set -euo pipefail

NAME="${GUEST_NAME:-kernal-macos-x86}"
IMAGE="${GUEST_IMAGE:-}"
STORAGE="${GUEST_STORAGE:-$HOME/.clud/docker-mac-x86/storage}"
SSH_PORT="${SSH_PORT:-2222}"
READY_TIMEOUT="${GUEST_READY_TIMEOUT:-1800}"

start() {
  local -a mount=()
  local image="$IMAGE"

  if [ -n "$IMAGE" ]; then
    docker pull "$IMAGE"
  elif [ -d "$STORAGE" ]; then
    image="dockurr/macos:latest"
    mount=(-v "$STORAGE:/storage")
  else
    echo "no prebaked \$GUEST_IMAGE and no prepared guest at $STORAGE" >&2
    exit 1
  fi

  docker run -d --name "$NAME" \
    --device=/dev/kvm --device=/dev/net/tun --cap-add NET_ADMIN \
    -p 8006:8006 -p "${SSH_PORT}:22" \
    -e VERSION=ventura -e RAM_SIZE=8G -e CPU_CORES=1 -e DISK_SIZE=128G \
    "${mount[@]}" --stop-timeout 120 "$image" >/dev/null

  # macOS boots slowly under one core; wait on sshd rather than a fixed sleep,
  # and surface guest logs if it never comes up.
  local deadline=$(( $(date +%s) + READY_TIMEOUT ))
  until nc -z localhost "$SSH_PORT" 2>/dev/null; do
    if [ "$(date +%s)" -ge "$deadline" ]; then
      echo "guest sshd unreachable on :${SSH_PORT} after ${READY_TIMEOUT}s" >&2
      docker logs --tail 60 "$NAME" >&2 || true
      exit 1
    fi
    sleep 10
  done
  echo "guest ready on :${SSH_PORT}"
}

stop() { docker stop "$NAME" >/dev/null 2>&1 || true; echo "guest stopped"; }

case "${1:-}" in
  start) start ;;
  stop) stop ;;
  *) echo "usage: $0 {start|stop}" >&2; exit 2 ;;
esac
