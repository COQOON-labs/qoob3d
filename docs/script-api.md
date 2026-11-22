# Script API reference

As of: 2026-11-22 · Decision: ADR-0053 · Requirements: `docs/requirements/agent.md` §1
(AG-SC-01…16) · Implementation: `crates/qoob3d-kernel/src/script/`

A QOOB3D model is a [Rhai](https://rhai.rs) script. The script is glue: every geometric
operation below is one call into OpenCascade. Every ```` ```rhai ```` example in this file is
run by `crates/qoob3d-kernel/tests/script.rs` with the project folder `docs/`.

## Conventions

- **Units:** lengths in mm, angles in degrees, volumes in mm³, areas in mm². +Z is up.
- **Numbers:** ints and floats are accepted wherever a number is expected (`box(10, 20, 30)`
  and `box(10.0, 20.0, 30.0)` are the same). Invalid dimensions (≤ 0, NaN, infinite) are
  `Runtime` errors at the call.
- **Points and vectors** are arrays `[x, y, z]`; 2D points are `[x, y]`.
- **Shapes are values:** every operation returns a new shape, inputs never change.
- **Types:** `Shape` (solid or compound of solids), `Profile` (2D region: an outline, or a
  combination of outlines with holes),
  `Selection` (edges or faces of one shape).

Everything Rhai offers works as usual: `let`, `const`, `if`, `for x in [..]`,
`for i in range(1, n)`, `fn name(a, b) { … }`, arrays, maps, string concatenation, and the
math functions `PI()`, `sin`, `cos`, `tan`, `sqrt`, `abs`, `floor`, `to_float()` … (trigonometry
of Rhai works in **radians**; our API takes degrees).

```rhai
let d = 10;                       // int
let h = 2.5 * d;                  // float
let rad = 30.0 * PI() / 180.0;    // Rhai's sin/cos/tan take radians
print(sin(rad));
fn plate(w, t) { box(w, w, t) }   // own functions are fine
show(plate(d, h), "plate");
```

## Output

### `show(shape, name)`

Registers `shape` as an output of the script under `name` (string). Outputs keep the call
order; shapes that are not shown are not output. The first argument must be a shape.

```rhai
let b = box(10, 20, 30);
show(b, "block");
show(b.move(0, 0, 40), "block_above");
```

### `print(value)`

Adds one line to the printed output (any value; floats print round-trip exact).

```rhai
print("volume:");
print(box(10, 20, 30).volume());   // 6000.0
```

## Solids

All solids have their axis along Z.

### `box(x, y, z) -> Shape`

Cuboid with edge lengths `x`, `y`, `z` (mm), centred in X and Y, bottom at z = 0.

```rhai
show(box(40, 20, 5), "plate");   // x ∈ [−20, 20], y ∈ [−10, 10], z ∈ [0, 5]
```

### `cylinder(d, h) -> Shape`

Cylinder of diameter `d` and height `h` (mm), axis Z, centred, bottom at z = 0.

```rhai
show(cylinder(10, 20), "rod");   // V = π · 5² · 20
```

### `sphere(d) -> Shape`

Sphere of diameter `d` (mm), centre at the origin.

```rhai
show(sphere(10), "ball");   // z ∈ [−5, 5]
```

### `cone(d1, d2, h) -> Shape`

Cone or frustum: diameter `d1` at z = 0, `d2` at z = `h` (mm). One diameter may be 0 (tip);
equal diameters are an error (use `cylinder`).

```rhai
show(cone(10, 4, 6), "frustum");
show(cone(10, 0, 8), "tip");
```

### `torus(d_ring, d_tube) -> Shape`

Ring around the Z axis: `d_ring` is the diameter of the tube's centre line, `d_tube` the
tube diameter (mm), `d_tube < d_ring`. Centre at the origin, z ∈ [−d_tube/2, d_tube/2].

```rhai
show(torus(20, 4), "ring");   // outer diameter 24, height 4
```

## Booleans

Every boolean is one OpenCascade operation and is checked by the plausibility rule (see
[Errors](#errors-and-limits)).

Booleans run OpenCascade in parallel mode on all cores. For plates with many holes, pockets
or cut-outs of constant depth, combine **profiles** instead and extrude once (see
[Profile algebra](#profile-algebra)): that is far faster than cutting solids.

### `a + b`, `union(shapes) -> Shape`

Union. `union([s1, s2, …])` fuses any number of shapes in **one** operation, much faster
than chained `+`. Disjoint shapes are allowed (the result has several solids). An empty
array is an error.

```rhai
let a = box(20, 20, 10);
let b = cylinder(10, 30).move(0, 0, -10);
show(a + b, "plus");
let parts = [];
for i in 0..5 { parts.push(box(10, 10, 10).move(i * 5, 0, 0)); }
show(union(parts), "bar");
```

### `a - b`, `cut(a, tools) -> Shape`

Difference. `cut(a, [t1, t2, …])` removes all tools in **one** operation; `cut(a, tool)`
takes a single shape. The cut must remove material.

```rhai
let plate = box(40, 40, 5);
let holes = [];
for x in [-10, 10] { for y in [-10, 10] { holes.push(cylinder(4, 20).move(x, y, -5)); } }
show(cut(plate, holes), "plate");
show(box(20, 20, 10) - sphere(8).move(0, 0, 10), "dimple");
```

### `a & b`

Intersection (common part). The result must have volume.

```rhai
show(box(20, 20, 10) & cylinder(10, 30).move(0, 0, -10), "common");
```

## Transforms and patterns

Transforms return a moved copy and keep the shape exact.

### `s.move(x, y, z) -> Shape`

Translation by `[x, y, z]` mm.

```rhai
show(box(10, 10, 10).move(5, 0, 0), "moved");
```

### `s.rotate(axis, deg) -> Shape`

Rotation by `deg` degrees (right hand rule) about an axis through the origin: `"x"`, `"y"`,
`"z"` or a vector `[x, y, z]`.

```rhai
let a = box(10, 20, 30);
show(a.rotate("z", 90), "turned");      // x ∈ [−10, 10], y ∈ [−5, 5]
show(a.rotate([1, 1, 0], 45), "tilted");
```

### `s.mirror(plane) -> Shape`

Mirror image at an origin plane: `"xy"` (z → −z), `"xz"` (y → −y), `"yz"` (x → −x).

```rhai
show(box(10, 20, 30).move(5, 10, 0).mirror("yz"), "left");
```

### `s.scale(f) -> Shape`

Uniform scale about the origin by `f > 0`.

```rhai
show(box(10, 20, 30).scale(2), "double");   // V × 8
```

### `linear(s, n, [dx, dy, dz]) -> Shape`

`n` copies at offsets `k · [dx, dy, dz]`, k = 0 … n−1, fused into one shape (one union).

```rhai
show(linear(cylinder(3, 2), 5, [8, 0, 0]), "studs");
```

### `polar(s, n, deg_total) -> Shape`

`n` copies rotated about Z, fused into one shape. A full turn (`deg_total` = 360) spreads
them evenly (step 360/n); a smaller arc puts the first copy at 0° and the last at
`deg_total` (step `deg_total`/(n−1)).

```rhai
show(polar(box(2, 2, 2).move(10, 0, 0), 6, 360), "ring");
show(polar(cylinder(2, 3).move(15, 0, 0), 3, 90), "quarter");
```

## Profiles

Profiles are closed outlines in the XY plane (until placed with `on` or `at_z`). They become
solids with `extrude`, `revolve` or `loft`. Outlines can be moved in their plane and combined
into profiles with holes and several regions (see [Profile algebra](#profile-algebra)).

### `rect(w, h) -> Profile`

Rectangle `w` × `h` mm, centred on the origin.

### `circle(d) -> Profile`

Circle of diameter `d` mm around the origin.

### `polygon([[x, y], …]) -> Profile`

Closed polygon through at least 3 points (closed automatically, must not cross itself).

### `slot(l, w) -> Profile`

Stadium (oblong hole) of overall length `l` and width `w` (`l > w`), long axis along X,
centred.

```rhai
show(extrude(rect(10, 20), 5), "rect");
show(extrude(circle(10), 4), "disc");
show(extrude(polygon([[0, 0], [10, 0], [0, 10]]), 3), "triangle");
show(extrude(slot(20, 6), 2), "slot");
```

### `profile.move(x, y) -> Profile`

Shifts a profile by `[x, y]` mm in its own plane (before or after `on`: the shift is in the
profile's local axes).

```rhai
show(extrude(circle(4).move(10, 5), 3), "pin");   // centre (10, 5)
```

### Profile algebra

`p1 + p2` (union), `p1 - p2` (difference), `p1 & p2` (intersection),
`profile_union(profiles) -> Profile` and `profile_cut(p, tools) -> Profile` combine profiles
in 2D. The array forms are **one** operation for any number of profiles. The result is a
profile again: it may have holes and several separate regions (each region becomes its own
solid in `extrude`), and it can be moved, placed, combined further, extruded and revolved.
`loft` takes plain outlines only.

All operands must lie in the same plane: combine first, then place with `on` / `at_z`. An
empty result, an empty array and a mix of profiles and shapes are errors.

A plate with holes built this way is one planar operation and one extrusion. Use it instead
of `cut(box, cylinders)` whenever the features go straight through: 1000 holes take a
fraction of a second instead of seconds.

```rhai
let holes = [];
for i in 0..10 { for j in 0..5 { holes.push(circle(2).move(-18 + 4 * i, -8 + 4 * j)); } }
show(extrude(profile_cut(rect(44, 24), holes), 3), "perforated");

let ring = circle(20) - circle(12);
show(extrude(ring, 5), "washer");
show(extrude(rect(30, 6) + rect(6, 30), 2), "cross");
show(extrude(rect(20, 20) & circle(24), 2), "rounded_square");
show(extrude(profile_union([circle(10), circle(10).move(6, 0), circle(10).move(12, 0)]), 2), "chain");
show(extrude(rect(20, 10) - rect(4, 20), 1), "two_halves");   // two solids
show(extrude(on("xz", ring), 4), "upright_washer");
```

### `extrude(profile, h) -> Shape`

Extrudes the profile by `h` mm along the normal of its plane (`h` ≠ 0; negative goes the
other way).

```rhai
show(extrude(slot(30, 8), 3), "tab");
```

### `on(plane, profile) -> Profile`

Places a profile on an origin plane. Local axes (SK-PL-01): `"xy"`: u = +X, v = +Y,
normal +Z; `"xz"`: u = +X, v = +Z, normal −Y; `"yz"`: u = +Y, v = +Z, normal +X. `extrude`
follows the normal.

```rhai
show(extrude(on("xz", rect(10, 20)), 5), "front");   // y ∈ [−5, 0], z ∈ [−10, 10]
show(extrude(on("yz", circle(8)), 2), "side");
```

### `profile.at_z(z) -> Profile`

Moves a profile by `z` mm along Z (for lofts and raised extrusions).

```rhai
show(extrude(rect(10, 10).at_z(5), 2), "floating");   // z ∈ [5, 7]
```

### `revolve(profile, deg) -> Shape`

Turns a profile about the Z axis by `deg` ∈ (0, 360]. The profile is read as a half section:
its x is the radius, its y the height (it is drawn into the XZ plane; `on` and `at_z` are
ignored). Edges on the axis (x = 0) are allowed.

```rhai
let p = polygon([[4, 0], [10, 0], [10, 10], [4, 10]]);
show(revolve(p, 360), "tube");        // V = π (10² − 4²) · 10
show(revolve(p, 90), "quarter_tube");
```

### `loft(profiles) -> Shape`

Ruled solid through at least two profiles in order (place them with `at_z`). The profiles
should have the same number of edges (rectangle to rectangle, circle to circle). Combined
profiles (results of the profile algebra) are not accepted.

```rhai
show(loft([rect(20, 20).at_z(0), rect(10, 10).at_z(15)]), "pyramid_frustum");
show(loft([circle(10).at_z(0), circle(4).at_z(6)]), "cone_frustum");
```

## Wire and pipe

### `pipe(points, d, bend_radius) -> Shape`

Solid round wire of diameter `d` mm along the polyline `points` (`[[x, y, z], …]`, at least
two). Every corner is rounded by OpenCascade with `bend_radius` mm, which must be larger than
`d/2` (smaller radii would self-intersect, and OpenCascade does not notice). The ends are
flat and perpendicular to the first and last segment. Volume = π (d/2)² · centre line length.

```rhai
show(pipe([[0, 0, 0], [0, 0, 20], [30, 0, 20]], 2, 5), "bent_wire");
show(pipe([[0, 0, 0], [0, 0, 10]], 0.8, 1.5), "post");
```

## Edges, fillets and chamfers

### `edges(s) -> Selection`, `all_edges(s) -> Selection`

All edges of a shape (seams of round faces are not edges). `all_edges(s)` reads as "every
edge" in fillet calls; filters narrow `edges(s)` down. Filters chain and return a new
selection:

| Filter | Keeps |
| --- | --- |
| `.parallel(axis)` | straight edges parallel to `"x"`, `"y"`, `"z"` or `[x, y, z]` |
| `.lines()` | straight edges |
| `.circles()` | circles and arcs |
| `.top()` | edges lying in the highest z of the shape |
| `.bottom()` | edges lying in the lowest z of the shape |
| `.at_z(z, tol)` | edges lying in the plane z ± `tol` mm |
| `.len()` | number of selected elements (an int, not a filter) |

```rhai
let b = box(20, 30, 40);
print(all_edges(b).len());                     // 12
print(edges(b).parallel("z").len());           // 4
print(edges(b).parallel("x").top().len());     // 2
print(edges(b).lines().at_z(40, 0.001).len()); // 4
print(edges(b).bottom().len());                // 4
print(edges(cylinder(10, 5)).circles().len()); // 2
```

### `fillet(s, edges, r) -> Shape`

Rounds the selected edges of `s` with radius `r` mm. The selection must come from the same
shape. If OpenCascade fails, the error names the first edge that fails on its own.

```rhai
let b = box(20, 30, 40);
show(fillet(b, all_edges(b), 2), "rounded");
show(fillet(b, edges(b).parallel("z"), 3), "rounded_corners");
let c = cylinder(20, 10);
show(fillet(c, edges(c).top(), 2), "rounded_top");
```

### `chamfer(s, edges, d) -> Shape`

Bevels the selected edges with the distance `d` mm on both faces (45°).

```rhai
let b = box(20, 30, 40);
show(chamfer(b, edges(b).parallel("x").top(), 3), "beveled");
let c = cylinder(20, 10);
show(chamfer(c, edges(c).circles(), 1), "lead_in");
```

## Shell

### `faces(s) -> Selection`

All faces of a shape, with the filters `.top()`, `.bottom()`, `.at_z(z, tol)` (faces lying
in that plane) and `.len()`.

### `shell(s, thickness, open_faces) -> Shape`

Hollows `s` with walls of `thickness` mm, growing inward (outer size kept), and removes the
selected faces (the openings). The result must have less volume than `s` and more than 0,
otherwise it is a `Geometry` error (OpenCascade may return "valid" empty shells).

```rhai
let b = box(20, 30, 40);
show(shell(b, 2, faces(b).top()), "cup");      // V = 24000 − 16 · 26 · 38
show(shell(b, 2, faces(b).bottom()), "hood");
print(faces(b).len());                          // 6
```

## Measure

### `s.volume() -> float`, `s.area() -> float`

Volume in mm³ and surface area in mm².

```rhai
let b = box(10, 20, 30);
print(b.volume());   // 6000
print(b.area());     // 2200
```

### `s.bbox() -> map`

Tight axis-aligned bounding box (no tolerance added): a map with `min`, `max` and `size`,
each `[x, y, z]` in mm.

```rhai
let bb = sphere(10).move(0, 0, 5).bbox();
print(bb.min[2]);    // 0
print(bb.size[0]);   // 10
```

### `s.center() -> [x, y, z]`

Centre of mass (uniform density) in mm.

```rhai
let c = box(10, 20, 30).move(5, 0, 0).center();
print(c[0]);   // 5
print(c[2]);   // 15
```

### `distance(a, b) -> float`

Smallest distance between two shapes in mm; 0 if they touch or overlap.

```rhai
print(distance(box(10, 10, 10), box(10, 10, 10).move(30, 0, 0)));   // 20
```

### `ray(s, origin, direction) -> [[x, y, z], …]`

Points where the half-line from `origin` along `direction` (any length, not 0) crosses the
faces of `s`, sorted by distance. Hits behind the origin do not count.

```rhai
let b = box(20, 20, 10) - cylinder(4, 10);
for h in ray(b, [-50, 0, 5], [1, 0, 0]) { print(h[0]); }   // −10, −2, 2, 10
print(ray(b, [-50, 50, 5], [1, 0, 0]).len());              // 0: missed
```

### `s.is_valid() -> bool`, `s.solids() -> int`

`is_valid` checks the B-rep with OpenCascade's analyser; `solids` counts the solids.

```rhai
let two = box(2, 2, 2) + box(2, 2, 2).move(10, 0, 0);
print(two.is_valid());   // true
print(two.solids());     // 2
```

## FDM helpers

### `pin(d, l) -> Shape`

Cylindrical pin of diameter `d` mm pointing **down** from z = 0 to z = −`l`. Fuse it to the
bottom of a part.

### `hole(d, l, clearance) -> Shape`

The matching hole as a cutting tool: diameter `d + 2·clearance`, depth `l + clearance`, from
z = 0 down to z = −(`l` + `clearance`). `clearance` ≥ 0 (FDM: 0.1–0.2 mm). Move it to the top
face of the receiving part and cut.

```rhai
let base = box(20, 20, 10);
let top = box(20, 20, 5) + pin(4, 3);
show(base - hole(4, 3, 0.2).move(0, 0, 10), "base");
show(top.move(0, 0, 15), "lid");
```

## Files

Scripts can only read files inside the project folder; absolute paths and `..` are errors,
and without a project there is no file access at all.

### `import_step(path) -> Shape`

Reads a STEP file (path relative to the project folder) as one shape in mm.

```rhai
let block = import_step("script-api/block.step");
print(block.solids());   // 1
show(block - cylinder(4, 30), "drilled");
```

### `import "path" as name;`

Loads the Rhai module `<project>/path.rhai`; its functions and `export`ed constants are used
as `name::item`.

```rhai
import "script-api/fdm" as fdm;
show(fdm::plate_with_hole(30, 4, 0.2), "plate");
print(fdm::CLEARANCE);
```

## Project parts and values

In a project folder (`docs/requirements/agent.md` §2) the files `parts/*.rhai` declare parts;
`qoob3d serve` builds each part in a worker process and caches it.

### `part(name, options, |deps| shape)`

Declares the part `name` (unique in the project). `options` is a map; `depends` lists the
parts this one needs (other keys such as `material` and `color` are free). The closure gets
`deps`, a map from each dependency name to its built shape, and returns the part's shape.
Several parts per file are fine. The service finds the declarations without running the
closures and without any geometry: code outside the closures must not call CAD functions.
Run as a plain script, every part is built in order and shown under its name.

```rhai
part("Base", #{ material: "PLA" }, |deps| box(40, 40, 5));
part("Peg", #{ depends: ["Base"] }, |deps| {
    let top = deps["Base"].bbox().max[2];
    cylinder(6, 10).move(0, 0, top)
});
```

### `value(id) -> value`

The target value `id` from `values/*.toml` (int, float, string, bool or array). Every read is
recorded: changing a value rebuilds exactly the parts that read it. An unknown id is a
`Runtime` error naming it. Outside a project there are no values.

```toml
# values/dims.toml
[wall]
value = 1.2
source = "caliper, 2026-11-20"
status = "measured"   # measured | one_source | assumption
```

`part("Shell", #{}, |deps| box(20, 20, 10) - box(20 - 2 * value("wall"), 20 - 2 * value("wall"), 10).move(0, 0, value("wall")))`

## Checks

The files `checks/*.rhai` of a project hold named checks (`docs/requirements/agent.md` §4.2,
ADR-0054). `qoob3d check` (or the MCP tool `check`) builds stale parts and runs every check in
a worker process under the project limits. A task can only go to `review` and a state can only
be saved when its checks pass.

### `check(name, || { … })`

Declares the check `name` (unique in the project) and runs the closure. Inside it, all
measure functions, `value(id)` and the functions below are available. A check passes when
the closure ends without error and every expectation holds. A script error ends the check
and is reported with file, line and column; other checks still run.

### `part(name) -> Shape`

With one argument, `part` returns the built shape of the project part `name`. It is an error
if the part is not built and current. Run as a plain script, it returns a part declared
earlier in the same script. Also available in throwaway scripts (`run_script`).

### `expect(cond, text)`, `expect_near(actual, expected, tol, text)`, `expect_between(actual, min, max, text)`

Record one expectation with its `text`, the actual and the expected value. A failed
expectation does not stop the check, so one run shows everything that is wrong.
`expect_near` holds if |`actual` − `expected`| ≤ `tol`, `expect_between` if
`min` ≤ `actual` ≤ `max` (both borders inclusive). NaN never passes. Outside `check` the three
functions are `Runtime` errors.

```rhai
part("Plate", #{}, |deps| box(20, 10, 2));
check("plate_size", || {
    let plate = part("Plate");
    expect_near(plate.volume(), 400, 0.001, "volume of the plate");
    expect_between(plate.bbox().size[0], 19.9, 20.1, "width of the plate");
    expect(plate.is_valid(), "the plate is valid");
});
```

In a project the part is declared in `parts/*.rhai` and the check file contains only the
`check(…)` call.

## Errors and limits

A failed run returns one error with line and column of the script position:

| Kind | When |
| --- | --- |
| `Syntax` | the script does not parse |
| `Runtime` | wrong arguments (≤ 0, NaN, wrong type), unknown function, file outside the project |
| `Geometry` | an OpenCascade operation failed or its result is implausible; names the API function |
| `Limit` | more than `max_operations` Rhai operations (default 50 000 000) or too deep recursion |

**Plausibility rule (AG-SC-12)** after every boolean, `shell`, `fillet` and `chamfer`: the
result volume must be > 1e-6 mm³; `cut`/`-` and `shell` must reduce the volume; a union must
not be smaller than its largest input. A cut that misses the body is therefore an error,
because it is almost always a placement mistake.

Scripts have no file, network or process access beyond the functions above.

From Rust, `qoob3d_kernel::script::{export_stl, export_3mf, export_step}` write the shown
shapes (STL and 3MF meshed with 0.01 mm chordal tolerance, ADR-0034).
