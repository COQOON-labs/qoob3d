# QOOB3D workflow guide

A QOOB3D project is a folder of text files in Git. You model by writing Rhai scripts; the
service builds them. Units are mm, angles degrees, +Z is up, the print bed is z = 0.

## Project layout

- `qoob3d.toml` – project name, limits (time, memory), workers.
- `parts/*.rhai` – parts: `part("Name", #{ depends: [..] }, |deps| { … shape })`.
- `lib/*.rhai` – shared functions, used with `import "lib/name" as name;`.
- `values/*.toml` – target values, read in scripts with `value("id")`.
- `checks/*.rhai` – checks: `check("name", || { expect_near(…); })`.
- `tasks/NNNN-slug.md` – the task inbox of the human (read with `task_get`).
- `out/` – exports and renders; `.qoob3d/` – cache. Both are not yours to edit.

## The loop

1. `task_list {state: "open"}`, then `task_get {id}` – the human's words, plus `project`:
   the parts and checks that exist with their states.
2. `task_update {id, state: "in_progress"}`.
3. `api_reference {search}` – read the script API before writing; do not guess functions.
4. `file_write` the values, the part and a check that measures what the task asks for.
5. `build` – fix every error it reports, then build again.
6. `check` – every expectation lists actual and expected; fix the part, not the check.
7. `render {views: ["iso", "front", "top"]}` and look at the image; `measure` and
   `run_script` answer geometric questions exactly without changing the project.
8. `task_update {id, parts: [..], checks: [..]}` – bind what you built and checked (while the
   task is `open` or `in_progress`; the lists replace the old ones).
9. `task_update {id, state: "review", note}` – hand over. The gate needs at least one part
   and one check bound, every bound part built and current, every bound check passed after
   the last build. A task of kind `idea` needs the note only.
10. `save_state {message}` commits the project when all checks are green.

Only the human sets `done` or `rejected`. You may move `open -> in_progress`,
`in_progress -> blocked | review`, and `blocked | review -> in_progress`.

## Dimensions

- A dimension the task text or a measurement fixes is a value with status `measured`
  (measured on the real object) or `one_source` (one statement, e.g. the task text).
- A free design dimension (wall thickness, a radius you chose) is a value with status
  `assumption`; list every assumption in the note of the hand-over to review.
- A missing **interface** dimension (something the part must fit to: a screw, a rail, another
  part) or contradictory text is a question, never a guess:
  `task_update {id, state: "blocked", note: "<your question>"}`, then wait for the human.

Rules for `values/*.toml`: one table per value; its id is the table name, dots make
namespaces (`[plate.width]` is read with `value("plate.width")`); ids are unique in the
project. Keys: `value` (number, string, bool or array), `source` (non-empty text), `status`,
and the optional `unit` (documentation only, scripts always get mm). `values {}` lists them.

## A minimal project

```toml
# values/plate.toml
[plate.width]
value = 40
unit = "mm"
source = "task text: a 40 mm square plate"
status = "one_source"

[plate.thickness]
value = 3
source = "free design dimension"
status = "assumption"
```

```rhai
// parts/plate.rhai
part("Plate", #{ material: "PLA" }, |deps| {
    let w = value("plate.width");
    let t = value("plate.thickness");
    box(w, w, t) - hole(4, t, 0.2).move(0, 0, t)
});
```

```rhai
// checks/plate.rhai
check("plate_size", || {
    let size = part("Plate").bbox().size;
    expect_near(size[0], value("plate.width"), 0.001, "width of the plate");
    expect_near(size[2], value("plate.thickness"), 0.001, "thickness of the plate");
});
```

## Render views

| view | camera looks along | up in the image |
| --- | --- | --- |
| `iso` | −X +Y −Z, from the front right top corner | +Z |
| `front` | +Y | +Z |
| `back` | −Y | +Z |
| `left` | +X | +Z |
| `right` | −X | +Z |
| `top` | −Z (down onto the part) | +Y |
| `bottom` | +Z (up from below the bed) | −Y |

## FDM printing

- Model the part in its print orientation, standing on z = 0, or say in the review note how
  it has to be turned.
- `fillet` the top edges; on the bed face use `chamfer` (a fillet there starts as an overhang).
- Overhang rule: downward faces flatter than 45 degrees from the horizontal need support;
  measure with `overhangs(s, 45)`, then prefer a chamfer or another orientation.
- Holes: vertical axis where possible; fits with `pin(d, l)` and `hole(d, l, clearance)` and
  0.1–0.2 mm clearance; horizontal holes come out smaller.

## Rules and errors

- Batch booleans: `union([a, b, c])` and `cut(body, [t1, t2])` are one operation and far
  faster than a chain of `+` and `-`. For many holes, cut 2D profiles before `extrude`.
- A cut must remove material and a union must not shrink, otherwise the build fails: almost
  always a placement mistake. Task files change only through the task tools.

Every tool answers with one JSON envelope `{ok, duration_s, warnings, result}` and, when
`ok` is false, `error: {kind, message, file?, line?, column?}`: `script` – your script
failed at that place; `time_limit`, `memory_limit`, `crash` – the worker was stopped, the
service goes on; `gate` – review or save refused, the message lists what is not green;
`bad_request`, `not_found`, `load` – wrong arguments, unknown names, broken project files.
A failed part of a build result carries the same `error` object.
