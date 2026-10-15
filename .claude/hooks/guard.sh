#!/usr/bin/env bash
# Claude Code PreToolUse guard (ADR-0037, ADR-0039, ADR-0043): the rules live in
# `cargo xtask guard` (xtask/src/guard.rs). Exit 2 blocks the tool call.
#
# The guard always runs. It reads QOOB3D_HARNESS_UNLOCK itself: the unlock that
# the human sets when starting Claude Code lifts only the harness-file
# protection; fixtures, role separation and bypass detection stay active.
set -uo pipefail
cd "${CLAUDE_PROJECT_DIR:-$(dirname "$0")/../..}" || exit 2
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"

input=$(cat)
# Call the binary directly (cargo adds ~1 s per tool call); rebuild when stale.
# It lives under `.git/`, which agents never write (ADR-0043); an inherited
# CARGO_TARGET_DIR is ignored so that nothing can point the guard elsewhere.
target=".git/qoob3d/guard-target"
bin="$target/debug/xtask"
if [ ! -x "$bin" ] || [ -n "$(find xtask Cargo.lock -newer "$bin" -print -quit)" ]; then
  if ! CARGO_TARGET_DIR="$target" cargo build --quiet --package xtask; then
    if [ ! -x "$bin" ]; then
      echo "guard: xtask does not build and no guard binary exists – call blocked." >&2
      exit 2
    fi
    # A broken xtask build during harness work must not block every tool call,
    # including the edit that fixes it.
    echo "guard: xtask does not build – using the previously built guard binary." >&2
  fi
fi
"$bin" guard <<<"$input"
status=$?
# Fail closed: if the guard cannot run, block.
if [ "$status" -ne 0 ] && [ "$status" -ne 2 ]; then
  echo "guard: cargo xtask guard failed (exit $status) – call blocked." >&2
  exit 2
fi
exit "$status"
