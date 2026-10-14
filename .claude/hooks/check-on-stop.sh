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
  git ls-files -co --exclude-standard -- crates xtask docs harness models Cargo.toml \
    Cargo.lock CLAUDE.md clippy.toml deny.toml rust-toolchain.toml rustfmt.toml .cargo \
    .claude .github .githooks third_party 2>/dev/null \
    | sort | while IFS= read -r f; do [ -f "$f" ] && { echo "$f"; cat "$f"; }; done \
    | shasum | cut -d' ' -f1
)
if [ "$fingerprint" = "$(cat "$green_file" 2>/dev/null)" ]; then
  exit 0 # nothing changed since the last green check
fi

# Avoid endless loops: stop blocking after 3 consecutive red stops. This hook is
# only a brake; pre-push and CI still reject the red state (ADR-0037).
attempts=$(cat "$attempts_file" 2>/dev/null || echo 0)
if [ "$(jq -r '.stop_hook_active // false' <<<"$input")" = "true" ] && [ "$attempts" -ge 3 ]; then
  echo 0 >"$attempts_file"
  echo '{"systemMessage":"Stop hook: cargo xtask check --fast is still RED after 3 attempts. The task is NOT done; pre-push and CI reject this state."}'
  exit 0
fi

if output=$(cargo xtask check --fast 2>&1); then
  echo "$fingerprint" >"$green_file"
  echo 0 >"$attempts_file"
  exit 0
fi

echo $((attempts + 1)) >"$attempts_file"
{
  echo "cargo xtask check --fast is RED. Fix the cause before you report the task as done"
  echo "(do not weaken assertions, no #[allow]/#[ignore] without a reason). Last output:"
  echo "$output" | tail -n 60
} >&2
exit 2
