# QOOB3D workflow guide

A QOOB3D project is a folder of text files in Git. You model by writing Rhai scripts; the
service builds them. Units are mm, angles degrees, +Z is up, the print bed is z = 0.

## Project layout

- `qoob3d.toml` – project name, limits (time, memory), workers.
- `parts/*.rhai` – parts: `part("Name", #{ depends: [..] }, |deps| { … shape })`.
- `lib/*.rhai` – shared functions, used with `import "lib/name" as name;`. Reference files
  (`.stl`, `.3mf`, `.step`) come from the human into `lib/`; you cannot write them. Load them
  with `import_mesh`, `import_step` (`step_names` + `import_step(path, name)` pick bodies).
- `values/*.toml` – target values, read in scripts with `value("id")`.
- `checks/*.rhai` – checks: `check("name", || { expect_near(…); })`.
- `tasks/NNNN-slug.md` – the human's tasks; `out/`, `.qoob3d/` (cache) are not yours to edit.

## The loop

1. `task_list {state: "open"}`, then `task_get {id}` – the human's words, plus `project`:
   the parts and checks that exist with their states.
2. `task_update {id, state: "in_progress"}`.
3. `api_reference {search}` – read the script API before writing; do not guess functions.
4. `file_write` the values, the part and a check that measures what the task asks for;
   for part, check and lib files it reports declarations or load errors immediately.
5. `build` – fix every error it reports, then build again.
6. `check` – every expectation lists actual and expected; fix the part, not the check.
7. `render {views: ["iso", "front", "top"]}` and look at the image (`region` zooms, `source`
   renders a throwaway script); `measure`, `run_script` answer questions exactly.
8. `task_update {id, parts: [..], checks: [..]}` – bind what you built and checked (while the
   task is `open` or `in_progress`; the lists replace the old ones).
9. `task_update {id, state: "review", note}` – hand over. The gate needs at least one part
   and one check bound, every bound part built and current, every bound check passed after
   the last build. A task of kind `idea` needs the note only.
10. `save_state {message}` commits the project when all checks are green.

Only the human sets `done` or `rejected`. You may move `open -> in_progress`,
`in_progress -> blocked | review`, and `blocked | review -> in_progress`.

## Dimensions

- A dimension fixed by a measurement is `measured`, by one statement (the task) `one_source`.
- A free design dimension is a value with status `assumption`; list them in the review note.
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

- Print orientation: model a part where it sits, give it `pose: #{ rotate: ["x", 180],
  count: 2 }`; export writes it so, `export {kit}` puts a whole kit into one file.
- Colours: `color: "#C0392B"` per part (`render` shows it); `export {project}` writes a slicer
  project with plates and a filament per colour (`print_with`, `multicolor`); lettering: `text`.
- `fillet` the top edges; on the bed face use `chamfer` (a fillet there starts as an overhang).
- Overhangs flatter than 45° need support: `overhangs(s, 45)`; prefer a chamfer or a pose.
- Holes: vertical where possible; fits with `pin(d, l)`, `hole(d, l, clearance)` (0.1–0.2).

## Rules and errors

- Batch booleans: `union([a, b, c])` and `cut(body, [t1, t2])` are one operation and far
  faster than a chain of `+` and `-`. For many holes, cut 2D profiles before `extrude`.
- Imported (freeform) bodies: booleans take seconds, raise `[limits] time_s` in `qoob3d.toml`;
  an invalid part: `defects`, `heal`. A cut must remove material, a union must not shrink.

Every tool answers with one JSON envelope `{ok, duration_s, warnings, result}` and, when
`ok` is false, `error: {kind, message, file?, line?, column?, printed?}`: `script` – your script
failed at that place; `time_limit`, `memory_limit`, `crash` – the worker was stopped, the
service goes on; `gate` – review or save refused, the message lists what is not green;
`bad_request`, `not_found`, `load` – wrong arguments, unknown names, broken project files.
A failed part of a build result carries the same `error` object.
