# Requirements: the app shows a script project live (W1)

As of: 2026-11-22 · Status: **W1 implemented** · Decision: ADR-0053

Goal: QOOB3D opens a project folder whose parts are Rhai scripts (ADR-0053,
`docs/requirements/agent.md`) and shows the parts live while an AI agent builds them.
IDs `WA-<area>-<no>` are stable; code and tests refer to them.

## 1. Service protocol used by the app

The service is `qoob3d serve <DIR>` (agent requirements AG-SV-*). It listens on 127.0.0.1;
the port is in `<DIR>/.qoob3d/service.json` as `{"port": <u16>, "pid": <u32>}`. Every JSON
answer has the envelope `{"ok": bool, "duration_s": f64, "warnings": [str], "result": …}`,
with `"error": {"kind", "message"}` when `ok` is false.

| ID | Requirement |
| --- | --- |
| WA-PR-01 | `GET /status` → `result.parts` is a list of `{"name", "file", "state", "key"}`. `state` ∈ `current`, `outdated`, `failed`, `never_built`. `key` is the cache key of the last successful build (string) or `null`. Other fields may exist and are ignored. |
| WA-PR-02 | `GET /mesh/<name>` → the mesh of the last successful build in the binary format of section 3 (`Content-Type: application/octet-stream`); HTTP 404 with the JSON envelope if the part has never been built. Response header `X-Qoob3d-Key` carries the cache key of that mesh. |
| WA-PR-03 | `POST /build` with `{"parts": null, "wait_s": 0}` starts a build of all parts and returns at once. |

## 2. App behaviour

| ID | Requirement | Prio |
| --- | --- | --- |
| WA-OP-01 | `qoob3d --project <DIR>` opens the project in `<DIR>` (a folder with `qoob3d.toml`). Without `qoob3d.toml`, or if the folder does not exist, the app exits with code 2 and a message naming the folder. | MUST |
| WA-OP-02 | If `.qoob3d/service.json` is missing or the service does not answer `GET /status` within 1 s, the app starts the service with `<program> serve <DIR>` as a detached child. `<program>` comes from the environment variable `QOOB3D_SERVICE_CMD` (may contain arguments separated by spaces); default is the app's own executable. It then waits up to 20 s for a valid `service.json` and a successful `GET /status`. | MUST |
| WA-OP-03 | The app requests one build of all parts on opening (WA-PR-03). | MUST |
| WA-LV-01 | A background thread polls `GET /status` every 0.5 s. HTTP never runs on the UI thread. | MUST |
| WA-LV-02 | For each part whose `key` differs from the key of the mesh last loaded, the thread fetches `GET /mesh/<name>` and hands the mesh to the UI. Parts with unchanged key are not fetched again. Parts that disappear from the status disappear from the view; a part that returns is fetched again. | MUST |
| WA-LV-03 | STL import: a binary STL turns into a `ShapeMesh` (`qoob3d_core::stl::read_binary`): vertices welded by exact position, normals computed from the winding, feature edges = edges whose two triangle normals differ by more than 30° plus open edges. Malformed input (shorter than the declared count) is an error, never a panic, and never allocates for an absurd declared count. | MUST |
| WA-LV-04 | The browser panel lists the project parts with name and state (English: up to date / outdated / error / never built; German: aktuell / veraltet / Fehler / nie gebaut); a part in state `failed` is marked in the error color; the eye (👁) toggles visibility. All texts via `tr()`. | MUST |
| WA-LV-05 | If the service stops answering, the panel shows "service not reachable" and the last meshes stay visible; when it answers again, polling continues. | MUST |
| WA-LV-06 | The first time meshes arrive, the camera fits all parts. Later updates do not move the camera. | SHOULD |
| WA-HL-01 | `qoob3d --project <DIR> --render <PNG>` renders headless: start/connect the service (WA-OP-02), build all parts and wait until no part is `outdated` or `never_built` (at most 120 s), load all meshes, render, exit 0. Parts in state `failed` are named on stderr and lead to exit 1 after rendering; the PNG is still written. | MUST |

## 3. Mesh transfer format (`qoob3d_core::qmesh`)

Indexed and compact, because projects have many polygons: positions are stored once, not
three times per triangle as in STL.

| ID | Requirement |
| --- | --- |
| WA-QM-01 | Layout, little endian: magic `b"QMSH"`, `u32` version = 1, `u32` vertex count V, `u32` triangle count T, `u32` edge point count E, `u32` polyline count P; then positions `f32×3×V`, normals `f32×3×V`, indices `u32×3×T`, edge points `f32×3×E`, polyline starts `u32×(P+1)` (sentinel = E). |
| WA-QM-02 | `qmesh::write(&ShapeMesh) -> Vec<u8>` and `qmesh::read(&[u8]) -> Result<ShapeMesh, QmeshError>` round-trip exactly (bit-identical floats, `face_ids` are not transferred and read back empty). |
| WA-QM-03 | `read` rejects wrong magic, unknown version, truncated data, indices ≥ V, starts that are not monotonic or do not end at E, and counts whose byte size exceeds the input – without panicking and without allocating more than the input size. |

## 4. Later (not W1)

- WA-MS-*: mouse tools write script lines into `parts/*.rhai` (concept 7a); undo via Git.
- Section, measure and print view for project parts; annotations sent to the agent.
- Photo underlay on the sketch plane (concept 6a).

## 5. Tests

- STL reader and qmesh: unit tests in `qoob3d-core`.
- Client, poller, live state: tests against a fake service (a `TcpListener` in the test), no
  real service needed.
- UI: kittest tests for the browser list, error mark, eye, "service not reachable", fit.
- Headless: `--project` with `--render` against the fake service started through
  `QOOB3D_SERVICE_CMD`.
