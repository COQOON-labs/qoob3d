# Concept "Werft" – a 3D design environment for the Mac, built for AI agents

> English translation of the user's German concept of 2026-11-22; the German original is in Git
> history (commit 91e70f2, werft/docs/konzept.md). Superseded in architecture by ADR-0053 and
> ADR-0054; kept as background and source of requirements.
>
> Translation notes: "Werft" is German for "shipyard". German tool, folder and identifier names
> that are proper names of the concept (`bauen`, `soll()`, `parts/`, `soll/` …) are kept and
> glossed in English on first use. Identifiers inside code blocks are unchanged.

As of 2026-11-22. Grown out of 37 build stages of an RC ship model
(Damen Stan Patrol 5009, 1:40) in Autodesk Fusion via the Fusion MCP server.

## 1. Summary
- Not a Fusion replacement, but an environment in which the model is program code in
  a Git folder. Agent and human work on the same, readable model.
- Do not write the geometry kernel ourselves: Open Cascade (OCCT) via the
  Python library build123d (Apache-2.0).
- There is build123d-mcp (Apache-2.0), an MCP server that executes build123d code,
  renders, measures and exports. Examine it first and build on it.
- To be built new: project structure, background jobs, cache,
  checks as tests, image measurement from photos, print analysis, Mac app.
- Priorities: (a) keep the effort small, a tool of our own, nothing unnecessary;
  (b) the human shall be able to design with the mouse (7a); (c) deriving geometry from
  photos is a core feature (6a), by computation and with as little AI as possible.
- Limits: OCCT is more delicate than Fusion with difficult fillets, shells and freeform
  surfaces. Remedy: simple, robust operations; if need be
  STEP import.

## 2. Experience with Fusion and the Fusion MCP

### 2.1 Good (adopt)
| Experience | Conclusion |
|---|---|
| A free Python script with the full API was the most powerful tool | Core tool "execute code"; few, powerful tools |
| Exact bodies, sections, distance, interference, rays | BREP kernel; measuring and collision as ready-made tools |
| Build a shape non-committally first, look at it, then adopt it | Every run is a draft until it is checked in |
| Recreate the camera from the photo, lay the outline over the photo | Photo matching as a built-in tool |
| Check script after every change | Checks as pytest, run automatically |
| Named bodies, components, colors | Named parts in assemblies, colors as data |

### 2.2 Bad (solve)
| Problem | Solution |
|---|---|
| Every call had to stay under ~80 s; long checks only via tricks | Jobs in the background with an ID, progress, cancellation |
| An analysis script blocked Fusion, the MCP server went down, unsaved work at risk | Every run in its own process with a time and memory limit; the service stays responsive |
| Saving only as a whole, no comparison, no way back | Model = text files in Git; a crash loses nothing |
| Only one agent may write | Headless runs; several agents in Git branches/worktrees |
| Timeline with 1,360 entries, early changes risky | Dependency graph of parts; cache (hash of code + parameters + inputs) |
| Target values and check script as attributes in the document | Target values as JSON/TOML, checks as Python files |
| Images only by rearranging the one view | Rendering stateless and headless; camera, visibility, section as parameters |
| Print analysis by hand via meshes, the trigger of the hang | Built-in print analysis as a background job |
| Misreadings of photos were noticed only by the user | Photo matching is part of the acceptance of every part |
| API pitfalls, unit cm | Unit mm throughout; a small tested helper library |

### 2.3 What Fusion does better (do not rebuild)
Very robust fillets/shells/lofts, freeform surfaces, CAM,
drawing derivation, photo rendering. For that, STEP in/out remains.

## 3. Goals and non-goals
Goals:
1. An agent builds a model completely as code, checks it, illustrates it and
   exports it for FDM printing, without a user interface.
2. No run can block the environment; nothing is lost through a crash.
3. The human sees the state at any time: rotate, section, measure, hide
   parts, compare states, pin annotations to places in the model.
4. Every number has a source or is marked as an assumption; the data structure
   enforces this.
5. The human can perform the usual operations with the mouse (7a).
6. From photos of real objects, dimensions and geometry can be derived
   traceably (6a).

Non-goals: CAM, simulation, technical drawings, photo rendering, cloud,
multi-user, sale, a geometry kernel of our own, Windows/Linux app.

## 4. Architecture
```
Mac app "Werft" (SwiftUI): 3D view, part tree, section, measure, comparison,
                           photo matching, annotations, check traffic light,
                           mouse operation
        │ local interface (HTTP/WebSocket)
werftd (Python service):   job management (queue, worker processes,
                           limits, cancellation); build graph + cache;
                           tools build, check, measure, render, photo,
                           print, export
        │ MCP (stdio)                         │ CLI `werft …`
AI agent                                 human/scripts
        │
Project folder (Git): parts/*.py, soll/*.json, checks/*.py, fotos/, out/
```
(`soll/` = target values, `fotos/` = photos.)
- Core: Python 3.12, build123d. Every build or analysis job runs in a
  worker process of its own. The service itself never computes geometry.
- Project = folder in Git, no binary project file. Cache
  (.werft/cache) not in Git, can be regenerated at any time.
- MCP server and CLI are thin shells around the same functions.
- Rendering for the agent headless in the core (VTK or pyrender, offscreen), so that
  images are produced even when the Mac is locked or the app is closed.

## 5. Project structure and data model
```
projekt/
  werft.toml         # unit mm, scale, printer (bed, nozzle), material
  soll/*.json        # target values: {"id","wert","quelle","status"}
  fotos/             # photos + camera models (cam_*.json)
  lib/               # own helper functions (wire, peg, railing panel …)
  parts/*.py         # one function per part
  assembly.py        # assembly: position, fixed/removable, joints
  checks/*.py        # pytest
  views.toml         # named views
  print/plan.toml    # print orientation and settings per part
  out/               # exports, images, reports
```
(Target value fields: `wert` = value, `quelle` = source, `status` = status.)
```python
from werft import teil, soll, zapfen
from build123d import *

@teil("Kran_Saeule", gruppe="Ausruestung", material="PLA", farbe="weiss",
      druck=dict(lage="stehend"))
def bauen(dep):
    s = soll("kran.K1")          # the access logs the source
    saeule = Cylinder(s.d/2, s.h, align=(Align.CENTER, Align.CENTER, Align.MIN))
    return saeule + zapfen(d=4, l=2.45, richtung=-Z)
```
(Glosses: `teil` = part, `soll` = target value, `zapfen` = peg, `bauen` = build,
`Kran_Saeule`/`saeule` = crane column/column, `gruppe="Ausruestung"` = group "equipment",
`farbe="weiss"` = color "white", `druck` = print, `lage="stehend"` = orientation "upright",
`richtung` = direction.)
- Every part is a pure function: same inputs, same body.
- Stable references: faces/edges via selection by position/kind or via names
  assigned during the build, never via indices.
- Declare joints (peg/hole, clearance, magnet) once in assembly.py; the
  environment generates the peg, the matching hole and the associated check.
- Source obligation: soll() requires quelle (source) and status (measured / one source /
  assumption). A report lists all assumptions of a part.

## 6. Tools for the agent (MCP)
| Tool | Purpose |
|---|---|
| projekt_status (project status) | Parts, cache, last check, jobs, Git state |
| bauen(teile, warten_s) (build(parts, wait_s)) | builds what has changed; result or job ID |
| auftrag(id, status/ergebnis/abbrechen) (job(id, status/result/cancel)) | Progress, result, cancellation |
| ausfuehren(code, teile) (execute(code, parts)) | free code in a throwaway session; does not change the project |
| messen(abstand/durchdringung/strahl/masse/huelle) (measure(distance/interference/ray/dimensions/bounding box)) | ready-made measurements |
| rendern(ansicht/kamera, teile, schnitt, stil, groesse) (render(view/camera, parts, section, style, size)) | PNG headless |
| foto_abgleich(foto, teile, modus) (photo_match(photo, parts, mode)) | Photo rectified, outline on top, deviation in px/mm |
| pruefen(auswahl) (check(selection)) | pytest; list OK/HINWEIS/FEHLER (OK/NOTE/ERROR) with measured value |
| druck_analyse(teil, lage) (print_analysis(part, orientation)) | Overhangs, bridges, wall thickness, cross-sections, contact area, bed |
| vergleich(stand_a, stand_b, teile) (compare(state_a, state_b, parts)) | Volume, bounding box, largest deviation, difference image |
| export(teile, step/stl/3mf, drucklage) (export(parts, step/stl/3mf, print orientation)) | Files to out/ |
| anmerkungen(lesen/erledigen) (annotations(read/resolve)) | Annotations of the human from the app |
| stand_sichern(nachricht) (save_state(message)) | Git commit after a passed check |

The agent edits files with its normal file tools. Answers
always as JSON with ok, dauer_s (duration_s), warnungen (warnings), result; numbers in mm;
errors with file/line; lists truncated with a counter.

## 6a. Geometry from photos ("image measurement") – core feature
Principle: classical photogrammetry, every number with an error statement; AI only as a
suggestion, never as a measured value. Implementation with OpenCV/NumPy/SciPy.

| Building block | What it does | Result |
|---|---|---|
| Calibrate camera | from ≥ 6 control points: pose, focal length, distortion (PnP + adjustment) | Camera model with residual error in px |
| Camera without control points | Vanishing points from parallel edges; a known distance as scale; or orient photos relative to each other (optionally COLMAP) | Orientation |
| Triangulate point | the same point in ≥ 2 photos; epipolar line in the second photo as an aid | 3D point with uncertainty in mm, intersection-angle warning |
| Point on a known surface | Intersect the viewing ray with a plane/model face | 3D point from one photo |
| Rectify plane | Homography → true-to-scale ortho image | Sketch underlay with mm scale |
| Outline and edges | Silhouette in the photo against the model silhouette from the same camera | Deviation in px and mm |
| Profile body | Outline of a turned part → profile curve; ellipse → diameter, axis | Profile as a sketch |
| Dense reconstruction (optional) | many photos → point cloud/mesh (COLMAP/OpenMVS) as a reference | Mesh to compare against |

Rules:
- Every measured value carries its origin (photo, pixel, method), residual error and status
  and lands automatically in the target value files.
- Report bad geometry instead of concealing it: flat intersection angle,
  control points almost in one plane, large residuals.
- AI may help to find places; the value always comes from the computation, and
  human or agent confirms the point in the image.
- In the app: photos side by side, click a point in the first, epipolar line in the
  second, second click, the 3D point appears in the model.
Tools: kamera_einmessen (calibrate_camera), kamera_aus_fluchtpunkten
(camera_from_vanishing_points), triangulieren (triangulate),
strahl_auf_flaeche (ray_onto_face), entzerren (rectify), umriss_vergleich (outline_comparison),
profil_aus_foto (profile_from_photo),
rekonstruktion (reconstruction; background job).
Limits: Points, edges and profiles result, no finished bodies.
Hidden sides remain an assumption. Empirical value: ±1–2 mm at 1:40 from
drone photos.

## 7. The Mac app (view and intervene)
- 3D view with part tree (on/off, transparent), section plane,
  exploded view, tape measure.
- Updates itself as soon as the agent builds; shows new parts.
- Comparison of two states; photo matching with a cross-fade slider.
- Annotations: click on a place + text → lands at the agent with part,
  point and image detail.
- Check traffic light, job list with cancel, print view with colored
  overhangs.
- Technology: SwiftUI + SceneKit/RealityKit, meshes as glTF/USDZ from the service. As a
  fast first stage a local web view (three.js) in the WKWebView is enough.

## 7a. Designing with the mouse
Guiding idea: every operation with the mouse writes one line of code into the part. There
is no second data store.
First stage:
- Place primitives (box, cylinder, sphere, tube), drag with handles, type in
  dimensions.
- Sketch on a plane or face: line, rectangle, circle, arc, type in dimensions,
  snap to points/midpoints/grid. Initially without a constraint solver.
- Extrude, revolve, cut out; union/subtract/intersect;
  move/rotate/mirror/pattern.
- Fillet, chamfer, wall thickness, hole, peg-hole joint with clearance.
- Measure, section, undo/redo via the code history.
- Photo underlay on the sketch plane (from 6a).
Later, only when needed: constraint solver, loft/sweep, pulling faces, threads, text.

## 8. Milestones and acceptance
M0 – Feasibility:
- Install build123d and build123d-mcp, try them out, assess them.
- Three sample parts: crane column (body of revolution with chamfers and peg), stair (two
  stringers, 11 treads with slots), railing section made of wire (tubes with
  bends). In addition a large STEP body: import it, measure distance,
  interference and a ray onto a face.
- Acceptance: all three parts are produced; the volume deviates < 1 % from the values in
  section 10, as far as the dimensions are given there; build time per part < 5 s;
  STEP import < 60 s and from the cache < 2 s.
M1 – Core and CLI: project structure, @teil, build graph, cache,
  worker processes with limits. Acceptance: an endless part is terminated after the time
  limit, the service answers in the meantime; a changed part rebuilds only
  itself and its dependents.
M2 – MCP server: tools from section 6 without photo/print. Acceptance: a
  fresh agent builds, via MCP only, a box with a lid and a peg joint
  including a check.
M3 – Rendering and photo matching; in parallel the image measurement (6a) begins.
M4 – Print analysis and export (3MF with print orientation).
M5 – Mac app for viewing; start of mouse operation (7a). Acceptance 7a: I build,
  without help and with the mouse, a box with a lid, pegs and rounded edges.
M6 – Annotations and comparison.
M7 – Practical trial with a new small project.
The milestones are an order, not monthly plans. M0–M2 are hours to
a few days of agent work.

## 9. Risks
| Risk | Countermeasure |
|---|---|
| OCCT fails on fillets/shells | M0 trial; robust construction in lib/; STEP import; report errors cleanly |
| Boolean operations on large bodies slow | Cache; pre-filter with bounding bodies; background jobs |
| Duplicate work relative to build123d-mcp | M0 decision; rather extend |
| Mac app becomes a time sink | first a web view in the window |
| Scope grows | Non-goals apply; extension only with a reason from practice |
| Agent code runs on the Mac | Worker process only in the project folder, without network |

## 10. Comparison values from the ship project (Fusion)
- Stair: printed part 2.4285 cm³; 11 rises of 5.91 mm, inclination 52.4°, two
  stringers 1.6 × 5 mm, 20 mm wide outside, height 65 mm, treads with 2 slots each of
  0.6 mm, 6 stanchion eyes with hole Ø 0.9.
- Crane column: column Ø 13 mm, peg Ø 4 mm at the bottom, pocket 4.4 × 5.4 × 4.2 mm at the top.
  Volume not logged individually; here only check build time and validity.
- Railing: stanchions and handrail wire Ø 0.8, two mid rails Ø 0.5 at +8 and +15,
  handrail at +22 mm, stanchion spacing ~24 mm, bend radius ≥ 1.5 mm. Eight
  wire bodies together 1.841 cm³ at around 2.8 m Ø 0.8 and 2.4 m Ø 0.5.
- Check script in Fusion: 47 checks, run time 103–185 s.

## 11. Sources
- build123d: https://github.com/gumyr/build123d , https://build123d.readthedocs.io/
- build123d-mcp: https://github.com/pzfreo/build123d-mcp , https://pypi.org/project/build123d-mcp/
