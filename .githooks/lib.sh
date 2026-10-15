# Shared setup of the git hooks, sourced by each hook. Not a hook itself.
set -euo pipefail
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"
cd "$(git rev-parse --show-toplevel)"

# Builds xtask before the hook runs it. GUI git clients such as Sourcetree set
# `com.apple.quarantine` on every file their child processes create, so a freshly
# linked xtask is quarantined and Gatekeeper refuses to start it, which aborts the
# commit, merge or push. On macOS the attribute is removed from the built binary;
# `cargo xtask` then finds it fresh and runs it (ADR-0051).
cargo build --quiet --package xtask
if [ "$(uname)" = Darwin ]; then
  xattr -d com.apple.quarantine "${CARGO_TARGET_DIR:-target}/debug/xtask" 2>/dev/null || true
fi
