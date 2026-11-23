#!/usr/bin/env bash
# Claude Code PreToolUse guard (ADR-0037): the edit tools must not change
# harness files unless the human started Claude Code with
# QOOB3D_HARNESS_UNLOCK=1. Everything else passes. Exit 2 blocks the tool call
# and shows stderr to the agent.
#
# This is a write protection against accidental edits, not a sandbox: shell
# commands are not inspected. The list mirrors HARNESS in xtask/src/paths.rs.
set -uo pipefail
export PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"

[ "${QOOB3D_HARNESS_UNLOCK:-}" = "1" ] && exit 0

root=$(cd "${CLAUDE_PROJECT_DIR:-$(dirname "$0")/../..}" && pwd) || exit 2

# Prints the repository-relative path if the call edits a harness file. Paths
# are collapsed (`.`, `..`) and compared ignoring case, as macOS file systems do.
if ! hit=$(jq -r --arg root "$root" '
  def collapse:
    split("/")
    | reduce .[] as $c ([];
        if $c == "" or $c == "." then . elif $c == ".." then .[:-1] else . + [$c] end)
    | join("/");
  [".claude/", ".githooks/", ".github/", ".cargo/", "xtask/", "harness/",
   "cargo.toml", "clippy.toml", "deny.toml", "rustfmt.toml", "rust-toolchain.toml"] as $harness
  | ($root | collapse | ascii_downcase) as $root
  | (.tool_input.file_path // .tool_input.notebook_path // "") as $file
  | (if ($file | startswith("/")) then $file else $root + "/" + $file end
     | collapse | ascii_downcase) as $path
  | select($file != "" and ($path | startswith($root + "/")))
  | $path[($root | length) + 1:]
  | select(. as $rel
      | any($harness[]; . as $h
          | if ($h | endswith("/")) then ($rel | startswith($h)) else $rel == $h end))
'); then
  echo "guard: cannot read the hook input (is jq installed?) – call blocked." >&2
  exit 2
fi

if [ -n "$hit" ]; then
  echo "guard: $hit is a harness file (ADR-0037). Propose the change; the human" >&2
  echo "unlocks harness edits by starting Claude Code with QOOB3D_HARNESS_UNLOCK=1." >&2
  exit 2
fi
exit 0
