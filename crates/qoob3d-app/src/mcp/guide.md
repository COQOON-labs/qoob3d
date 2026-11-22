# QOOB3D workflow guide

A QOOB3D project is a folder of text files in Git. You model by writing Rhai scripts;
the service builds them with OpenCascade. Units are mm, angles degrees, +Z is up.

## Project layout

- `qoob3d.toml` – project name, limits (time, memory), workers.
- `parts/*.rhai` – parts: `part("Name", #{ depends: [..] }, |deps| { … shape })`.
- `lib/*.rhai` – shared functions, used with `import "lib/name" as name;`.
- `values/*.toml` – target values: `[id]` with `value`, `source`, `status`.
- `checks/*.rhai` – checks: `check("name", || { expect_near(…); })`.
- `tasks/NNNN-slug.md` – the task inbox of the human (read with `task_get`).
- `out/` – exports and renders; `.qoob3d/` – cache. Both are not yours to edit.

## The loop

1. `task_list {state: "open"}`, then `task_get` – read what the human wants.
2. `task_update {id, state: "in_progress"}`.
3. `api_reference {search}` – read the script API before writing; do not guess functions.
4. `file_write` the values (`values/*.toml`), the part (`parts/*.rhai`) and a check
   (`checks/*.rhai`) that measures what the task asks for.
5. `build` – fix every error it reports, then build again.
6. `check` – every expectation lists actual and expected; fix the part, not the check.
7. `render` (views iso, front, top, …) and look at the image; `measure` and `run_script`
   answer geometric questions exactly without changing the project.
8. `task_update {id, state: "review", note}` – hand over. The gate needs every part of
   the task built and current and every check of the task passed after the last build.
9. `save_state {message}` commits the project when all checks are green.

Only the human sets `done` or `rejected`. You may move `open -> in_progress`,
`in_progress -> blocked | review`, and `blocked | review -> in_progress`.

## Rules

- Every dimension is a value with a `source` (drawing, measurement, task text) and a
  `status`: `measured`, `one_source` or `assumption`. Read it with `value("id")`.
- Never guess a dimension. If the task does not give it, ask: `task_update {id, state:
  "blocked", note: "<your question>"}` and wait for the human.
- Batch booleans: `union([a, b, c])` and `cut(body, [t1, t2])` are one operation and far
  faster than a chain of `+` and `-`. For many holes, cut 2D profiles before `extrude`.
- A cut must remove material and a union must not shrink; otherwise the build fails. This
  is almost always a placement mistake.
- FDM fits: `pin(d, l)` and `hole(d, l, clearance)` with 0.1–0.2 mm clearance.
- Write files only with `file_write` (under `parts/`, `lib/`, `values/`, `checks/` and
  `qoob3d.toml`); task files change only through `task_create` and `task_update`.

## Errors

Every tool answers with one JSON envelope: `{ok, duration_s, warnings, result}` and, when
`ok` is false, `error: {kind, message, file?, line?, column?}`.

- `script` – your script failed; `file`, `line` and `column` point at the place.
- `time_limit`, `memory_limit`, `crash` – the worker was stopped; the service goes on.
- `gate` – review or save refused; the message lists every part and check that is not green.
- `bad_request`, `not_found`, `load` – wrong arguments, unknown names, broken project files.

A build result with `state: "failed"` for a part carries the same `error` object per part.
