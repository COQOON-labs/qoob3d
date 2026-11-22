---
id: ADR-0054
title: The project folder is the shared workspace of human and agent – task inbox, checks as a gate, MCP as the main interface
status: accepted
date: 2026-11-22
crates: [qoob3d-core, qoob3d-kernel, qoob3d-app]
tags: [architecture, ai-first, mcp, tasks, checks]
supersedes: []
superseded_by: null
---

## Context

ADR-0053 made models Rhai scripts in a project folder, built in worker processes by a
service. That lets an agent build. The user wants more: "put tasks, requirements and ideas
in, and it runs really well over MCP" for all future 3D models.

Review of the state on 2026-11-22 (all measured in this repository):

| Piece | State | Evidence |
| --- | --- | --- |
| Script kernel | works | 74 tests, analytic volumes to 1e-6; stair 161 ms (build123d: 675 ms) |
| Project logic (config, values with sources, graph, cache key) | works | 35 tests |
| Live view in the app | works | 43 tests |
| Workers, service, CLI | in progress | 18 tests written |
| MCP server, tasks, checks, render for the agent | missing | – |

What is good: one binary, no Python, OCCT failures cannot take down service or app, values
carry sources, the cache key is exact. What is weak: agents do not know the script API from
training; there is no place for the human's intent (tasks) and no rule for when work counts
as finished; an agent cannot see its result; MCP clients without file tools cannot work.

## Decision

- **The project folder is the single shared workspace** of human and agent, all text in Git:
  `qoob3d.toml`, `parts/*.rhai`, `lib/*.rhai`, `values/*.toml`, `checks/*.rhai`,
  `tasks/*.md`, `out/`. No second store, no database.
- **Task inbox:** every task, requirement or idea is one Markdown file `tasks/NNNN-slug.md`
  with a TOML header (title, kind, state, parts, checks). States: `open`, `in_progress`,
  `blocked` (agent asks the human a question), `review`, `done`, `rejected`. The human creates
  tasks (file, CLI, later the app) and alone decides `done`/`rejected`; the agent takes
  `open` tasks, works, and hands over with `review`.
- **Checks are the gate:** `checks/*.rhai` hold named checks with measured expectations. A
  task can only move to `review` when its parts are built and current and its checks pass;
  the service enforces this, not the agent's goodwill. `save_state` (Git commit) needs all
  checks green.
- **MCP is the main interface** (`qoob3d mcp`, stdio, hand-written JSON-RPC, no async
  runtime): few, powerful tools that return the JSON envelope, images for renders, the script
  API reference and a workflow guide as tools and resources, and confined file tools so that
  every MCP client can work. The CLI offers the same functions for humans and scripts.
- **The agent sees:** `render` returns PNG images of named views from the headless renderer
  (ADR-0013) of the same binary; `measure` and `run_script` (throwaway worker, cannot change
  the project) answer geometric questions exactly.
- **Order of work:** R2/R3 (workers, service, CLI) → R4 (tasks, checks, MCP, render) →
  app shows and creates tasks and annotations → mouse tools write script lines (concept 7a)
  → photo measurement (concept 6a) → print analysis.

## Consequences

- A new model project needs no setup beyond `qoob3d init <DIR>` and one MCP entry.
- Human in the loop by construction: `done` is never set over MCP.
- The gate makes "finished" checkable; an agent cannot hand over a red state.
- The script API reference is product surface: it is tested (every example runs) and served
  over MCP.
- More surface in `qoob3d-app`; when the workspace manifest is unlocked, service and MCP move
  into their own crate.

## Rejected alternatives

- **Tasks in an external tracker or a database** – not in Git with the model, not readable by
  every agent, needs accounts.
- **Agent sets `done` itself** – removes the human acceptance the concept demands.
- **MCP via an SDK with an async runtime** – large dependency for a line-based protocol.
- **Only resources for the API reference** – many clients do not show resources to the
  model; a tool always works.

## References

- `docs/requirements/agent.md` (R4), `docs/requirements/project-app.md`
- `werft/docs/konzept.md` sections 2, 5, 6, 7
- Related: ADR-0013, ADR-0053
