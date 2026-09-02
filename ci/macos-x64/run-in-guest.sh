#!/usr/bin/env bash
# Ship the prebuilt archive into the macOS guest and execute it there.
#
# nextest propagates the real test exit code, so CI gates on it directly.
set -euo pipefail

ARCHIVE="${ARCHIVE:-$PWD/kernal-x64.tar.zst}"
PORT="${GUEST_SSH_PORT:-2222}"
USER_="${GUEST_USER:-runner}"
HOST_="${GUEST_HOST:-localhost}"
SSH_OPTS="-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null"

scp -P "$PORT" $SSH_OPTS "$ARCHIVE" "$USER_@$HOST_:~/kernal-x64.tar.zst"

# shellcheck disable=SC2029  # deliberate remote-side expansion
ssh -p "$PORT" $SSH_OPTS "$USER_@$HOST_" \
  "cargo-nextest run --archive-file ~/kernal-x64.tar.zst ${NEXTEST_FILTER:+-E '${NEXTEST_FILTER}'}"
