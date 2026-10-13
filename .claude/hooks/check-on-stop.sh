#!/usr/bin/env bash
# Claude Code Stop hook (ADR-0013): before Claude may finish a turn, run
# `cargo xtask check --fast` if code or docs changed since the last green run.
# Exit 2 blocks the stop and feeds stderr back to Claude.
set -uo pipefail

input=$(cat)
cd "${CLAUDE_PROJECT_DIR:-$(dirname "$0")/../..}" || exit 0
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"

state_dir="target/stop-hook"
mkdir -p "$state_dir"
attempts_file="$state_dir/attempts"
green_file="$state_dir/last-green"

# Fingerprint of everything the check looks at (works without any commit).
fingerprint=$(
  git ls-files -co --exclude-standard -- crates xtask docs Cargo.toml Cargo.lock \
    CLAUDE.md deny.toml rust-toolchain.toml .cargo 2>/dev/null \
    | sort | while IFS= read -r f; do [ -f "$f" ] && { echo "$f"; cat "$f"; }; done \
    | shasum | cut -d' ' -f1
)
if [ "$fingerprint" = "$(cat "$green_file" 2>/dev/null)" ]; then
  exit 0 # nothing changed since the last green check
fi

# Avoid endless loops: give up blocking after 3 consecutive red stops.
attempts=$(cat "$attempts_file" 2>/dev/null || echo 0)
if [ "$(jq -r '.stop_hook_active // false' <<<"$input")" = "true" ] && [ "$attempts" -ge 3 ]; then
  echo 0 >"$attempts_file"
  echo '{"systemMessage":"Stop-Hook: cargo xtask check --fast ist nach 3 Versuchen weiterhin rot – bitte selbst prüfen."}'
  exit 0
fi

if output=$(cargo xtask check --fast 2>&1); then
  echo "$fingerprint" >"$green_file"
  echo 0 >"$attempts_file"
  exit 0
fi

echo $((attempts + 1)) >"$attempts_file"
{
  echo "cargo xtask check --fast ist ROT. Behebe die Ursache, bevor du die Aufgabe als erledigt meldest"
  echo "(keine Assertions abschwächen, kein #[allow]/#[ignore] ohne Begründung). Letzte Ausgabe:"
  echo "$output" | tail -n 60
} >&2
exit 2
