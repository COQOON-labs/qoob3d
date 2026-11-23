# Requirements: the app shows a script project live (W1)

As of: 2026-11-23 · Status: **W1 implemented** · Decision: ADR-0053

Goal: QOOB3D opens a project folder whose parts are Rhai scripts (ADR-0053,
`docs/requirements/agent.md`) and shows the parts live while an AI agent builds them. This is
the only thing the app shows; headless pictures come from `qoob3d render-project` (AG-RD-01).
IDs `WA-<area>-<no>` are stable; code and tests refer to them.

## 1. Service protocol used by the app

The service is `qoob3d serve <DIR>` (agent requirements AG-SV-*). It listens on 127.0.0.1;
port and token are in `<DIR>/.qoob3d/service.json` as `{"port": <u16>, "pid": <u32>,
"token": <32 hex characters>}`. Every JSON
answer has the envelope `{"ok": bool, "duration_s": f64, "warnings": [str], "result": …}`,
with `"error": {"kind", "message"}` when `ok` is false.

| ID | Requirement |
| --- | --- |
| WA-PR-01 | `GET /status` → `result.parts` is a list of `{"name", "file", "state", "key"}`. `state` ∈ `current`, `outdated`, `failed`, `never_built`. `key` is the cache key of the last successful build (string) or `null`. Other fields may exist and are ignored. |
| WA-PR-02 | `GET /mesh/<name>` → the mesh of the last successful build in the binary format of section 3 (`Content-Type: application/octet-stream`); HTTP 404 with the JSON envelope if the part has never been built. Response header `X-Qoob3d-Key` carries the cache key of that mesh. |
| WA-PR-03 | `POST /build` with `{"parts": null, "wait_s": 0}` starts a build of all parts and returns at once. |
| WA-PR-04 | Every request carries the token of `service.json` in the header `X-Qoob3d-Token`, a `Host` of `127.0.0.1:<port>` (or `localhost:<port>`) and no `Origin` header; anything else is answered with HTTP 403 before routing (AG-HD-03, ADR-0055). The token is new at every service start, so a client that gets no answer reads `service.json` again for port and token (AG-HD-05). Names in a path (`/mesh/<name>`) are percent-encoded as one segment (AG-HD-04). |

## 2. App behaviour

| ID | Requirement | Prio |
| --- | --- | --- |
| WA-OP-01 | `qoob3d --project <DIR>` opens the project in `<DIR>` (a folder with `qoob3d.toml`). Without `qoob3d.toml`, or if the folder does not exist, the app exits with code 2 and a message naming the folder. | MUST |
| WA-OP-02 | If `.qoob3d/service.json` is missing or the service does not answer `GET /status` within 1 s, the app starts the service with `<program> serve <DIR>` as a detached child. `<program>` comes from the environment variable `QOOB3D_SERVICE_CMD` (may contain arguments separated by spaces); default is the app's own executable. It then waits up to 20 s for a valid `service.json` and a successful `GET /status`. | MUST |
| WA-OP-03 | The app requests one build of all parts on opening (WA-PR-03). | MUST |
| WA-OP-04 | Without a project the viewport shows an empty state: what a project is and a button "Open project folder…". The button, the toolbar button "Open" and Cmd/Ctrl+O pick a folder in the system dialog and open it like WA-OP-01; a folder that is no project shows an error dialog and keeps what was open. | MUST |
| WA-LV-01 | A background thread polls `GET /status` every 0.5 s. HTTP never runs on the UI thread. | MUST |
| WA-LV-02 | For each part whose `key` differs from the key of the mesh last loaded, the thread fetches `GET /mesh/<name>` and hands the mesh to the UI. Parts with unchanged key are not fetched again. Parts that disappear from the status disappear from the view; a part that returns is fetched again. | MUST |
| WA-LV-04 | The browser panel lists the project parts with name and state (English: up to date / outdated / error / never built; German: aktuell / veraltet / Fehler / nie gebaut); a part in state `failed` is marked in the error color; the eye (👁) toggles visibility. All texts via `tr()`. | MUST |
| WA-LV-05 | If the service stops answering, the panel shows "service not reachable" and the last meshes stay visible; when it answers again, polling continues. The poller reads `service.json` again while there is no answer, so it follows a service restarted on another port; a failed first connection is retried every poll interval (AG-HD-05; the retries look for a running service, they start none). | MUST |
| WA-LV-06 | The first time meshes arrive, the camera fits all parts. Later updates do not move the camera. | SHOULD |

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

- qmesh: unit tests in `qoob3d-core`.
- Client, poller, live state: tests against a fake service (a `TcpListener` in the test), no
  real service needed.
- UI: kittest tests for the empty state, browser list, error mark, eye, "service not
  reachable", fit.
