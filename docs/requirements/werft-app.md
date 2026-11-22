# Requirements: QOOB3D as the app of Werft

As of: 2026-11-22 · Status: **W1 specified** · Decision: ADR-0052

Goal: QOOB3D shows a Werft project (models are build123d code, `werft/docs/konzept.md`) live
while an AI agent builds it, and later lets the human model with the mouse by writing code.
IDs `WA-<area>-<no>` are stable; code and tests refer to them.

## 1. Protocol of `werftd` used by the app (W1)

`werftd` is the Python service of a Werft project (`werft/docs/m1_spec.md`). It listens on
127.0.0.1; the port is in `<project>/.werft/dienst.json` as `{"port": <u16>, "pid": <u32>}`.
Every JSON answer has the envelope `{"ok": bool, "dauer_s": f64, "warnungen": [str],
"ergebnis": …}`, with `"fehler": {"art", "meldung"}` when `ok` is false.

| ID | Requirement |
| --- | --- |
| WA-PR-01 | `GET /status` → `ergebnis.teile` is a list of `{"name", "datei", "zustand", "schluessel"}`. `zustand` ∈ `aktuell`, `veraltet`, `fehler`, `nie_gebaut`. `schluessel` is the cache key of the last successful build (string) or `null`. Other fields may exist and are ignored. |
| WA-PR-02 | `GET /netz/<name>` → binary STL (little endian, 80-byte header, u32 count, 50 bytes per triangle) of the last successful build of the part, tessellated with 0.01 mm linear tolerance; HTTP 404 with the JSON envelope if the part has never been built. Response header `X-Werft-Schluessel` carries the cache key of that mesh. |
| WA-PR-03 | `POST /bauen` with `{"teile": null, "warten_s": 0}` starts a build of all parts and returns at once. |

## 2. App behaviour (W1)

| ID | Requirement | Prio |
| --- | --- | --- |
| WA-OP-01 | `qoob3d --werft <DIR>` opens the Werft project in `<DIR>` (a folder with `werft.toml`). Without `werft.toml` the app exits with code 2 and a message naming the folder. | MUST |
| WA-OP-02 | If `.werft/dienst.json` is missing or the service does not answer `GET /status` within 1 s, the app starts it with the command `werft -C <DIR> dienst start`. The program name comes from the environment variable `QOOB3D_WERFT_CMD` (default `werft`, may contain arguments separated by spaces, e.g. `uv run --directory /x werft`). It then waits up to 20 s for a valid `dienst.json` and a successful `GET /status`. | MUST |
| WA-OP-03 | The app requests one build of all parts on opening (WA-PR-03). | MUST |
| WA-LV-01 | A background thread polls `GET /status` every 0.5 s. HTTP never runs on the UI thread. | MUST |
| WA-LV-02 | For each part whose `schluessel` differs from the one last loaded, the thread fetches `GET /netz/<name>` and hands the mesh to the UI. Parts with unchanged key are not fetched again. Parts that disappear from the status disappear from the view. | MUST |
| WA-LV-03 | The STL reader turns a binary STL into a `ShapeMesh`: vertices welded by exact position, flat shading per triangle is acceptable, feature edges = mesh edges whose two triangles enclose more than 30° (and open edges). Malformed input (shorter than the declared count) is an error, never a panic. | MUST |
| WA-LV-04 | The browser panel lists the Werft parts with their name and state (aktuell / veraltet / fehler / nie gebaut); a part in state `fehler` is marked in the error color; the eye toggles visibility. All texts via `tr()`, English and German. | MUST |
| WA-LV-05 | If the service stops answering, the panel shows "service not reachable" and the last meshes stay visible; when it answers again, polling continues. | MUST |
| WA-LV-06 | The first time meshes arrive, the camera fits all parts (Fit). Later updates do not move the camera. | SHOULD |
| WA-HL-01 | `qoob3d --werft <DIR> --render <PNG>` renders the project headless: start/connect the service (WA-OP-02), build all parts and wait until no part is `veraltet` or `nie_gebaut` (at most 120 s), load all meshes, render, exit 0. Parts in state `fehler` are reported on stderr and lead to exit 1 after rendering. | MUST |

## 3. Later (not W1)

- WA-MS-*: mouse tools write build123d code into `parts/*.py` (concept 7a); undo via Git.
- Section, measure and print view for Werft parts; annotations sent to the agent.
- Photo underlay on the sketch plane (concept 6a).

## 4. Tests (W1)

- STL reader: unit tests in `qoob3d-core` (valid file, welding, feature edges of a cube = 12
  edges, truncated input → error).
- Client and polling: tests against a fake `werftd` (a `TcpListener` in the test that serves
  `/status` and `/netz/<name>`), no Python needed.
- UI: kittest test that the browser lists the parts with their state and the error mark.
- Headless: `--werft` with `--render` against the fake service.
