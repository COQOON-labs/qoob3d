# Shared setup of the git hooks, sourced by each hook. Not a hook itself.
set -euo pipefail
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"
cd "$(git rev-parse --show-toplevel)"

xtask_binary="${CARGO_TARGET_DIR:-target}/debug/xtask"

# Builds xtask before a hook runs "$xtask_binary". GUI git clients such as
# Sourcetree set `com.apple.quarantine` on every file their child processes
# create, so a freshly linked xtask is quarantined and Gatekeeper refuses to
# start it. On macOS the attribute is removed from the built binary; the hooks
# then run it directly, because `cargo xtask` would re-link and re-quarantine
# it.
build_xtask() {
  cargo build --quiet --package xtask
  if [ "$(uname)" = Darwin ]; then
    if [ -f "$xtask_binary" ]; then
      xattr -d com.apple.quarantine "$xtask_binary" 2>/dev/null || true
    else
      echo "hooks: $xtask_binary not found, quarantine not removed" >&2
    fi
  fi
}
