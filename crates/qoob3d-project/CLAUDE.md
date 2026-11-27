# qoob3d-project

Script projects without GPU or UI (ADR-0053, ADR-0054, ADR-0055): the project service
`qoob3d serve`, its worker processes `qoob3d worker`, the service client, the MCP server
`qoob3d mcp` and `qoob3d init`. The binary lives in `qoob3d-app`, which dispatches its
subcommands here.

## Rules

- **No GPU or UI, not even transitively** (ADR-0036, `cargo xtask arch`). Rendering
  (`render-project`) stays in the app; MCP starts it as a child process.
- **The service never runs OCCT and never evaluates a user script** (ADR-0055): everything
  that touches the kernel runs in a worker process.
- Errors: `anyhow` with context is fine here (process code); types that callers match on
  (`worker::ErrorKind`, envelopes) stay plain data.
- **Every client of the service goes through `client::Client`**, so it sends the token.
- **Never read or hash a file while holding the service lock** (`Service::lock`, WS-PRE-01):
  copy what you need out, work, write back. File digests come from `Service::hashes`.
- **Every worker request of a job goes through `Service::work`** (it takes a slot of the
  machine-wide budget, WS-PRE-06); only declaration discovery and `bench` call `Pool::run`.
- Tests that start a service or the CLI give it `support::isolated` (its own
  `QOOB3D_RUNTIME_DIR`), so they never take worker slots of the user's services.
- stdout of `qoob3d worker` and `qoob3d mcp` carries only protocol messages; logs go to stderr.
- Project file and folder names come from `qoob3d_core::project::layout`.
- Integration tests run the real binary, so they live in `crates/qoob3d-app/tests/`
  (`service.rs`, `agent.rs`, `mcp.rs`, `mcp_guide.rs`, `perf.rs`, `project.rs`).

## Files

- `src/lib.rs`: modules, `RENDER_PROJECT` (name of the subcommand MCP starts).
- `src/client.rs`: `ProjectDir` and the minimal std::net HTTP `Client` of the service (start via
  `QOOB3D_SERVICE_CMD`; port and token from `service.json`, read again when the service does not
  answer; `segment` percent-encodes names in paths). A service whose process is alive and holds
  `service.lock` but does not answer is never started twice: `connect` waits and fails with
  `Busy`, `error_kind` gives the envelope kind `busy` (CLI, MCP; WS-PRE-03). **Every client
  of the service goes through `client::Client`**, so it sends the token. Used by the app, the
  CLI, MCP and `render-project`.
- `src/service.rs` + `src/service/`: `qoob3d serve` (AG-SV-01, ADR-0053): routes and envelope,
  single instance via `.qoob3d/service.lock`; **every request needs the token of this start**
  (`service.json`, header `X-Qoob3d-Token`), a local `Host` and no `Origin`, else 403 before
  routing (`Service::allowed`, AG-HD-03, ADR-0055); `http.rs` minimal server side HTTP; `jobs.rs` job
  queue and parallel part scheduling, cache hits; `pool.rs` warm workers, time/memory/crash
  limits, replacement, recycling above `[workers] recycle_mb` (AG-PF-03); `discovery.rs`
  declaration discovery (ADR-0055): every changed part or check file goes to a worker
  (`Probe::Declare`) on a thread of its own, answers and load errors cached by file and
  literal-import bytes; `/status` and `/mesh` never wait for discovery and show the last
  known declarations (parts of a file under rediscovery `outdated`, of a broken file `failed`,
  a never discovered file no parts yet, `discovering` lists files not known yet), builds,
  checks and the gate wait for all files, `POST /discover` waits for chosen files (MCP
  `file_write` answers `declared`/`load_error` with it, `qoob3d status` settles up to 10 s);
  the cache lock is never held while a worker runs; `snapshot.rs` the project from those
  declarations (per-file `load_errors`, a broken file never hides the others), keys, states,
  cache eviction (`evict`, AG-PF-04);
  `hashes.rs` the digest memo (WS-PRE-01): digests and literal imports by path, size and
  modification time, nothing remembered for a file younger than 1 s when it was read (racy),
  `known` for `/status` and `/mesh` (a file above 1 MiB goes to the `qoob3d-hash` thread,
  `pending` → `/status` `hashing`), `digest` for builds, checks and the gate, `key` the cache
  key of every part with its inputs (hashed again only when one changed; `views` also asks
  each file once per call);
  `snapshot.rs` `Service::views` takes the lock only to copy records and to store found cache
  entries, `Snapshot::key` gets digests through a function (`None` while one is hashed);
  `summary.rs` `.qoob3d/summary.json` every 0.5 s when a count changed, and once more at
  shutdown (WS-PRE-04); `Service::watch_idle` stops the service after `[service] idle_min`
  without allowed requests and unfinished jobs (WS-PRE-05); `slots.rs` the machine-wide worker
  budget (WS-PRE-06): runtime folder, `budget.json`, `slot-<i>.lock` file locks, default
  physical cores − 1; `Service::work` holds a slot around `Pool::run`, `Service::shown` shows a
  running job that only waits as `queued` with `reason`.
  `bench.rs` `qoob3d bench <DIR>` (AG-PF-06): a fresh worker per build, entries in
  `.qoob3d/bench-<pid>/` (removed afterwards), never the project cache; it discovers in its own
  process. The service never runs OCCT and never evaluates a script. The `[mesh]` settings
  and the kernel fingerprint enter the cache key through `worker::key_version` (AG-PF-05,
  WS-PRE-02); files enter it as their SHA-256 (`file_digest`).
  R4 (ADR-0054): `jobs.rs` `Work` (build, check, run) so `/check` and `/run` answer with a job
  like `/build`; `checks.rs` checks of the discovered `checks/*.rhai`, `/check`
  (one worker request per check), states passed/failed/stale/never_run (stale = check file or
  the build key of a part the check read changed), `gate_input` for the review gate; `probe.rs`
  `/run` and `/measure`; `save.rs` `/save` (`git` executable, never adds `.qoob3d/`, `out/`);
  `tasks.rs` `/tasks`, `/task/<id>` on `qoob3d_core::tasks` (`role` is required). `/status`
  stays lock-short: check states come from records, never from a run.
  R5 hardening (agent.md §6.1): job threads run inside `jobs::caught` (a panic is a `crash`
  of that job or part) and hold workers through `pool::Lease` (dropped without `give`, the
  worker is replaced); locks recover from poisoning; `/status` lists the 20 newest jobs with
  `kind`; task create/update/bind run under `Service::task_writes`; `POST /task/<id>` takes
  `parts`/`checks` (binding) and reports load errors of part and check files as the gate
  error; `/save` needs the project folder to be the Git top level and commits with the
  pathspec of the add. The cache key hashes the modules the engine resolved in the last
  build (`modules_read` in the cache `.json`, `snapshot::Reads`), the literal scan only
  before the first build (AG-HD-06), and after them the files `import_step`/`import_mesh`
  read (`files_read`, AG-SC-24; a file that is gone hashes as empty). Declaration results are cached per file **including
  load errors**, so a file that hits the discovery limits is not evaluated on every poll.
  `/status` reports `load_errors: [{kind, message, file, line?, column?}]` (kind `load`,
  `time_limit`, `memory_limit`, `crash`) and the first one also as `error`; a build of all
  parts builds what loads and then fails with those errors.
- `build.rs`: `QOOB3D_KERNEL_FINGERPRINT` (WS-PRE-02), SHA-256 over the files of
  `qoob3d-kernel` (`Cargo.toml`, `build.rs`, `cpp/`, `src/`), of `third_party/opencascade-sys`
  and of the `qoob3d-core` files the kernel runs (`CORE`), and over the kernel's packages in
  `Cargo.lock` (of a path package only its normal and build dependencies, `PATH_PACKAGES`);
  `worker/tests.rs` recomputes it with a second lock reader, so a stale fingerprint fails,
  derives `CORE` from the `qoob3d_core::` paths of the kernel (a new path there may ask for
  a file in the list), and checks it covers every package `cargo metadata` resolves for the
  kernel.
- `src/worker.rs`: `qoob3d worker` (AG-PJ-05; lowers its own priority by nice 10 at start,
  `rustix`, WS-PRE-01): request/answer types of the JSON-line protocol
  (answers start with `{"job":`, foreign stdout lines from OCCT are skipped), builds one part and
  writes its cache entry, writes STEP, declares a file (`declaration`, `PartMode::Declare` with
  the AG-HD-02 limits). Exits when stdin closes. `worker/probe.rs`: `Probe` requests that write
  nothing: one check, a throwaway script, measures of one or two parts, declarations.
- `src/mcp.rs` + `src/mcp/`: `qoob3d mcp [DIR]` (AG-MC-01..06): JSON-RPC 2.0 over stdio, one
  line per message, synchronous; **stdout carries only protocol messages** (never `println!`
  there, logs go to stderr). `tools.rs` tool table (schemas) and dispatch: service tools hand
  the service envelope through (service started on demand, `task_update` always as role
  agent, also binds `parts`/`checks`; `task_get` adds `project` from `/status`; `render` adds
  the `bbox` from the build results); `files.rs` confined file tools (AG-MC-03: allowed folders and extensions, 1 MiB, any
  symlink on the path refused, task files read-only; AG-MC-11: `file_list` adds `imports`
  `{path, size}` for `.step`/`.stp`/`.stl`/`.3mf`, `file_read` on them names `import_step`
  or `import_mesh`); `render.rs` runs `render-project` as a
  child process and returns the PNG as base64 (`views`: 2 to 6 views as one contact sheet;
  the view table there must match `guide.md`); `resources.rs` embeds `docs/script-api.md`,
  `mcp/guide.md` (at most 120 lines; its minimal project is built by `tests/mcp_guide.rs`)
  and the kernel example scripts.
- Damen 5009 findings (agent.md §6.6): `/export` applies `PrintPose` from the snapshot's
  declarations and writes kits (`kit`, `bed_mm`) with `details` per file (AG-SV-09);
  `/run {meshes: true}` makes the worker write the shown shapes as `.qoob3d/run/<job>/<i>.qmsh`
  for `render-project --source` (AG-MC-12); `PartError::printed` carries the lines printed
  before a script error (AG-SC-35); a meshing failure is kind `script` naming the part
  (AG-SC-36); the cache `.json` has `mesh` (export report) and `printed` (entry version 4).
  MCP `render` takes `source` and `region` and hashes long file names; `file_write` of
  `qoob3d.toml` answers when its sections apply (limits, mesh, cache: next job; workers: next
  start).
- Colours and slicer projects (AG-PJ-07, AG-SV-10, ADR-0034): declarations carry `color` and
  `print_with` to `/status`; `service/slicer.rs` writes `/export {project, multicolor,
  printer}` as a Bambu Studio project (plates grouped by the filaments their objects need,
  purge corner of the bed kept empty). **Check a change there with Bambu Studio's command
  line** (`ag_sv_10_bambu_studio_…` in `tests/service.rs`, no window): our own 3MF reader
  accepted three files the slicer got wrong.
- `src/init.rs`: `qoob3d init <DIR>` (AG-SV-08); writes `lib/examples/` from the master
  copies in `docs/script-api/lib/examples/` (`EXAMPLES`: a new asset of the reference needs
  an entry there).

## Relevant ADRs

ADR-0034, ADR-0036, ADR-0053, ADR-0054, ADR-0055, ADR-0057
