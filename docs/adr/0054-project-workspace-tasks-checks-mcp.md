---
id: ADR-0054
title: The project folder is the shared workspace of human and agent, with a task inbox, checks as the review gate and MCP as the agent's main interface
status: accepted
date: 2026-11-23
crates: [qoob3d-core, qoob3d-kernel, qoob3d-app]
tags: [architecture, ai-first, mcp, tasks, checks]
supersedes: []
superseded_by: null
---

## Context

Models are Rhai scripts in a project folder, built in worker processes by a service
(ADR-0053). That lets an agent build, but more is needed for real work: a place for the
human's intent (tasks, requirements, ideas), a rule for when work counts as finished that does
not depend on the agent's goodwill, a way for the agent to see its result, and an interface
that every MCP client can use, including clients without their own file tools. Agents do not
know the script API from training, so it has to be served with the tools.

## Decision

### One workspace, all text in Git

The project folder is the single shared workspace: `qoob3d.toml`, `parts/`, `lib/`,
`values/`, `checks/`, `tasks/`, plus the generated `out/`. No second store, no database.
`qoob3d init <DIR>` creates it, including `lib/examples/` with the files the reference
examples import.

### Task inbox

- Every task, requirement or idea is one Markdown file `tasks/NNNN-slug.md`: a TOML header
  (title, kind `task`/`requirement`/`idea`, state, bound `parts` and `checks`, created), the
  body, and a `## Log` of dated entries by `agent` or `human`.
- States: `open`, `in_progress`, `blocked` (the agent asks the human), `review`, `done`,
  `rejected`. The agent has five transitions: `open`/`blocked`/`review` → `in_progress` and
  `in_progress` → `blocked`/`review`. The human may make any change and alone decides
  `done` and `rejected`.
- The service serves the inbox (`/tasks`, `/task/<id>`, role required) and serialises task
  writes; the CLI (`qoob3d task …`) acts as the human.

### Checks are the gate

- `checks/*.rhai` declare named checks with `check(...)` and `expect…`. `/check` builds stale
  parts first and runs each check as one worker request. A result is `passed`, `failed`,
  `never_run` or `stale` (its file changed or a part it read was rebuilt).
- **Review gate**, enforced by the service: a task or requirement moves to `review` only with
  at least one part and one check bound, every bound part `current` and every bound check
  passed; an idea needs a note saying what was done.
- `/save` (MCP `save_state`) commits with the `git` executable only when every part builds
  and every check passes, only project paths (never `.qoob3d/` or `out/`), and only when the
  project folder is the top level of its repository.

### MCP is the main interface

- `qoob3d mcp [DIR]` is a hand-written, synchronous JSON-RPC 2.0 server on stdio; stdout
  carries only protocol messages. Its 19 tools are thin wrappers over the service API and
  answer with the service envelope `{ok, duration_s, warnings, result}`; it starts the
  service on demand.
- `task_update` always acts as role `agent`, so `done` can never be set over MCP.
- **Confined file tools** (`file_list`, `file_read`, `file_write`) work on `qoob3d.toml` and
  the project folders only: `.rhai`, `.toml` and `.md`, at most 1 MiB, any symlink on the path
  refused, task files read-only (tasks change through the task tools).
- **The agent sees its result:** `render` runs `qoob3d render-project` in a child process and
  returns PNG images of named views (2 to 6 views as one labelled contact sheet); `measure`
  and `run_script` (a throwaway worker that writes nothing) answer geometric questions
  exactly.
- The script API reference, a workflow guide (at most 120 lines) and the example scripts are
  embedded in the binary and offered both as tools (`api_reference`, `guide`) and as resources
  (`qoob3d://script-api`, `qoob3d://guide`, `qoob3d://examples/…`).
- The CLI offers the same functions for humans and scripts.

## Consequences

- A new model project needs nothing beyond `qoob3d init` and one MCP entry in the client.
- Human in the loop by construction; an agent cannot hand over a red state or accept its own
  work.
- The API reference and the guide are product surface: they are tested (every example runs,
  the guide's minimal project builds) and must not contain repository paths or ADR numbers.
- **Open:** the app does not show or create tasks yet; mouse tools that write script lines,
  photo measurement and print analysis come after that.

## Rejected alternatives

- **Tasks in an external tracker or a database** – not in Git with the model, not readable by
  every agent, needs accounts.
- **The agent sets `done` itself** – removes the human acceptance.
- **MCP through an SDK with an async runtime** – a large dependency for a line-based protocol.
- **The API reference only as resources** – many clients do not show resources to the model;
  a tool always works.
- **Relying on the client's own file tools** – not every client has them, and they are not
  confined to the project.

## References

- Code: `crates/qoob3d-core/src/tasks/`, `crates/qoob3d-kernel/src/script/check.rs`,
  `crates/qoob3d-app/src/service/{checks,probe,save,tasks}.rs`, `crates/qoob3d-app/src/mcp.rs`,
  `crates/qoob3d-app/src/mcp/{tools,files,render,resources}.rs`,
  `crates/qoob3d-app/src/mcp/guide.md`, `crates/qoob3d-app/src/{init,project_render,service_cli}.rs`
- `docs/requirements/agent.md` (section 4), `docs/script-api.md`,
  `docs/concept/werft-concept.md` (sections 2, 5, 6, 7)
- Related: ADR-0053, ADR-0055
