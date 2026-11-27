# Requirements: workbench – many projects, agent sessions, Claude accounts (WS)

As of: 2026-11-27 · Status: **S0 implemented, S1–S3 specified** · Decision: ADR-0057 · Builds on
ADR-0053 (script projects), ADR-0054 (workspace, tasks, checks, MCP), ADR-0055 (service)

Goal: the human works the other way round. Instead of an agent driving one project from
outside, the app is the workbench: it shows every project and its state, keeps several
projects open, and starts Claude agents on them with one of the human's own Claude
accounts. Each project stays what it is today: a folder in Git with its own service, checks,
task inbox and review gate. IDs `WS-<area>-<no>` are stable.

Scope: one human on one machine with their own Claude accounts (personal use). Not in scope:
other users, shared or pooled accounts, a login of our own, cloud, several agents writing the
same project at once.

Phases, each one ends with a green `cargo xtask check`:
**S0** service prerequisites · **S1** overview · **S2** several open projects ·
**S3** accounts and agent sessions.

## 1. Service prerequisites (S0)

Findings of the Damen 5009 project that would break an overview of many projects.

| ID | Requirement |
| --- | --- |
| WS-PRE-01 | `/status` answers within 100 ms (AG-SV-01) with 120 parts, while a build and a check run. The file hashes behind the cache keys are memoised by path, size and modification time and computed outside the service lock; no request waits for hashing. |
| WS-PRE-02 | The cache key includes a fingerprint of the kernel (at build time: the sources of `qoob3d-kernel` with `cpp/`, the OCCT fork in `third_party/opencascade-sys`, the code of `qoob3d-core` the kernel runs while it builds, such as lettering and mesh import, and the kernel's dependencies locked in `Cargo.lock`, such as Rhai, OCCT and the font reader). A kernel whose results may change rebuilds every part; a rebuild of the app alone, or an update of crates the kernel does not use, keeps the cache. |
| WS-PRE-03 | A client tells a busy service from a dead one: while the process in `service.json` is alive and holds `service.lock`, the client starts no second service and reports kind `busy` after its timeout instead of "did not come up". |
| WS-PRE-04 | The service writes `.qoob3d/summary.json` atomically whenever part, check, job or load-error states change (at most twice per second): `{project, updated, parts: {current, outdated, failed, never_built}, checks: {passed, failed, stale, never_run}, jobs_running, load_errors}`. |
| WS-PRE-05 | Idle shutdown: the service exits after `[service] idle_min` (default 30) minutes without requests and without jobs; it removes `service.json` as today. A polling app keeps it alive. |
| WS-PRE-06 | Machine-wide worker budget: at most `max_busy_workers` workers (app setting from S2 with WS-MP-05, until then `budget.json` in the runtime folder; default: physical cores − 1, at least 1) run jobs at the same time across all project services of the user, coordinated by slot lock files in a per-user runtime folder. A job waiting for a slot is `queued` with the reason `waiting for a worker slot` in `/status`; a crashed service frees its slots (the OS releases the locks). |

## 2. Overview (S1)

| ID | Requirement |
| --- | --- |
| WS-OV-01 | The app keeps a project list (folder paths) in its settings. Projects are added with the folder dialog, by dropping a folder, or by opening one (`--project`, Cmd/Ctrl+O); removing one from the list never touches its files. |
| WS-OV-02 | The overview shows one card per project: name (`qoob3d.toml`), folder, part and check counts, tasks per state, Git branch and number of changed files, the agent state and its account, and when the state was last updated. A folder that is gone shows its card greyed with "folder not found". |
| WS-OV-03 | The overview reads files only: `.qoob3d/summary.json`, `tasks/*.md`, `.qoob3d/agent/session.json`, and Git through the `git` executable (`status --porcelain`, branch). It never starts a service or a worker. Reads are bounded (a summary above 64 KiB or a task file above 1 MiB counts as unreadable); anything missing or unreadable shows as "unknown", never as an error dialog. |
| WS-OV-04 | For projects whose service runs (it answers within 100 ms), the overview uses `/status` instead of the summary, refreshing visible cards every 2 s. HTTP and Git run off the UI thread. |
| WS-OV-05 | A card is marked "needs you" when a task is in `review`, a check failed, a load error exists, the agent waits for input, or the account of the project is signed out or expired. The overview can be sorted by "needs you" first. |
| WS-OV-06 | Card actions: open in a tab, start an agent (WS-AG-01), open the agent in a terminal (WS-AG-08), show the folder in the file manager, remove from the list. |
| WS-OV-07 | Every text via `tr()` in all languages; accessible names on cards and actions. |

## 3. Several open projects (S2)

| ID | Requirement |
| --- | --- |
| WS-MP-01 | Several projects are open at once, one tab each with its own poller, live state, camera and selection. The overview is a tab of its own that cannot be closed. |
| WS-MP-02 | The active tab polls as today (WA-LV-01, 0.5 s) and fetches changed meshes; other tabs poll `/status` every 2 s and fetch meshes only when they become active. |
| WS-MP-03 | Closing a tab stops its poller and frees its meshes; the service keeps running until its idle shutdown (WS-PRE-05). |
| WS-MP-04 | The open tabs are restored at the next start (from the settings); a tab whose folder is gone is dropped with a note. |
| WS-MP-05 | The status bar shows busy and waiting workers against the machine-wide budget (WS-PRE-06). |

## 4. Claude accounts (S3)

Policy (Anthropic, "Legal and compliance", "Authentication"): a user may sign in to the
unmodified Claude Code binary with their own subscription; third-party apps must not offer a
claude.ai login of their own, nor collect, store or pass on Claude credentials or tokens, nor
use a subscription on behalf of others. Several logins on one machine are supported through
one `CLAUDE_CONFIG_DIR` per account. ADR-0057 records how qoob3d stays inside that.

| ID | Requirement |
| --- | --- |
| WS-AC-01 | An account profile is `{name, config_dir}` in the app settings. `config_dir` defaults to `<app data>/claude/<slug of name>`; the human may point it at an existing folder (for example `~/.claude`). |
| WS-AC-02 | "Sign in" opens the human's terminal running `claude auth login` with `CLAUDE_CONFIG_DIR=<config_dir>`; "Sign out" runs `claude auth logout` the same way. Signing in always happens in Anthropic's own flow. |
| WS-AC-03 | The account state comes from `claude auth status --json` with that `CLAUDE_CONFIG_DIR` (off the UI thread, timeout 10 s): signed in or not, and what the command reports about the account and the expiry of the login. It is shown in the settings and on the overview. |
| WS-AC-04 | qoob3d never reads, copies, stores, logs or transfers a credential or token: not the Keychain entries, not `.credentials.json`, not `CLAUDE_CODE_OAUTH_TOKEN`. An account that bills by API key is set up inside its config folder by the human (for example `apiKeyHelper` in its settings); qoob3d stores no keys. |
| WS-AC-05 | Every `claude` process that qoob3d starts gets `CLAUDE_CONFIG_DIR` of the chosen account, and `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN` and `CLAUDE_CODE_OAUTH_TOKEN` are removed from its environment (they would silently take precedence over the chosen login). |
| WS-AC-06 | Each project has a default account, kept in the app settings by folder path (accounts are personal and never written into the project folder). The account can be changed when a session starts; a running session keeps its account. |
| WS-AC-07 | qoob3d never switches accounts on its own (not on a usage limit, not on an error). A limit or authentication error ends the session with Claude's message (WS-AG-07). |

## 5. Agent sessions (S3)

Two ways to run Claude on a project: a background session inside the app, limited to the
qoob3d tools, and an interactive Claude Code in a terminal. Both use the human's installed
`claude` (program from `QOOB3D_CLAUDE_CMD`, default `claude` on `PATH`), started in the
project folder.

| ID | Requirement |
| --- | --- |
| WS-AG-01 | A background session starts from a task of the inbox ("work on task T-0003") or from a free prompt, with the chosen account (WS-AC-05), in the project folder as working directory: `claude -p --input-format stream-json --output-format stream-json --verbose --mcp-config .qoob3d/agent/mcp.json --strict-mcp-config` plus the tool rules of WS-AG-02 and a system prompt addition that names the project, the task and the rule to work through the qoob3d tools. It never uses `--bare` (that ignores the subscription login). |
| WS-AG-02 | A background session changes the project only through the qoob3d MCP tools (confined file tools, build, check, tasks, render; AG-MC-*): those are allowed, built-in tools that write files or run commands (`Bash`, `Edit`, `Write`, `NotebookEdit`) and web tools are disallowed, and nothing waits for a permission prompt (`--permission-prompts none`). |
| WS-AG-03 | qoob3d writes `.qoob3d/agent/mcp.json` with one server `qoob3d` = its own executable with `mcp <DIR>`. `.qoob3d/` stays out of Git (`/save` never adds it). |
| WS-AG-04 | At most one background session per project at a time. The session record `.qoob3d/agent/session.json` (`{id, account, task, state, started, updated, last_message}`, `state` ∈ `running`, `waiting`, `done`, `failed`, `stopped`) is written atomically; the stream is appended to `.qoob3d/agent/<id>.jsonl`. |
| WS-AG-05 | The agent panel of a project tab shows the stream: Claude's text, each tool call with its name and a short result, errors, and the usage the stream reports. The human can stop the session (the whole process tree ends within 2 s) and continue a finished one with a follow-up message (`--resume <id>`). |
| WS-AG-06 | The review flow stays that of ADR-0054: the agent updates its task with role `agent`, the gate decides whether it may go to `review`, and only the human sets `done` or `rejected` (app buttons on the task, as `qoob3d task set`). |
| WS-AG-07 | A session that ends with an error (usage limit, authentication, network, the CLI missing or too old) is `failed` with Claude's message; there are no automatic retries. A CLI older than the oldest supported version is refused before start with a message naming both versions. |
| WS-AG-08 | "Open in terminal" starts the human's terminal (macOS: Terminal; Linux: `$TERMINAL`, else `x-terminal-emulator`) running an interactive `claude` in the project folder with the account of WS-AC-05, the MCP config of WS-AG-03 and hooks from a `--settings` file (`Notification`, `Stop`) that call `qoob3d agent-event <DIR> <event>`; the event updates `session.json`, so the overview shows `waiting` and `done`. Permissions in the terminal are Claude Code's own. |
| WS-AG-09 | Quitting the app with running background sessions asks whether to stop them; sessions are child processes of the app and end with it. Terminal sessions are not stopped. |
| WS-AG-10 | Sessions on different projects run in parallel; their builds share the worker budget (WS-PRE-06). |

## 6. Quality and the existing gates

| ID | Requirement |
| --- | --- |
| WS-QA-01 | The logic without UI (project list and overview reading, accounts, session processes, the stream parser, `agent-event`) lives in `qoob3d-project` (ADR-0036: no GPU or UI); the app draws it. No new crate, so the architecture check and the harness stay unchanged. |
| WS-QA-02 | Tests never call a real Claude: `QOOB3D_CLAUDE_CMD` points to a fake `claude` in the tests that replays recorded stream lines, exit codes and `auth status` answers. A real session runs only in an opt-in test (`QOOB3D_CLAUDE_LIVE=1`), like the acceptance test of real projects. |
| WS-QA-03 | The stream parser reads foreign bytes: unknown event types are kept as raw lines and shown collapsed, malformed lines are logged and skipped, a line above 1 MiB is cut; it never panics. |
| WS-QA-04 | UI tests (egui_kittest) for the overview, tabs and the agent panel with fake data; screenshots for `cargo xtask screenshot`; the smoke test opens the overview, a project tab and switches tabs. |
| WS-QA-05 | Everything that works today keeps working unchanged: `qoob3d --project`, CLI, MCP, service, checks, review gate, `save`, the reference project and `cargo xtask check`. Each phase ends green. |

## 7. Open questions

- Several background sessions on one project (in Git worktrees) – not now; one per project.
- Whether the overview may run `git fetch` to show ahead/behind – not now (no network from
  the overview).
- A shared "inbox of all projects" (tasks in `review` across projects) as its own view, or
  only the "needs you" marks of WS-OV-05.

## 8. Tests

- S0: service tests with 120 generated parts and a running build (`/status` latency), a
  kernel fingerprint change in a test cache, a busy service (`busy` instead of a second
  start), summary file contents, idle shutdown with a short `idle_min`, two services sharing
  one worker slot.
- S1, S2: unit tests of the overview reader against prepared folders (missing, oversized and
  broken files); kittest for cards, tabs and restore.
- S3: fake `claude` for start arguments, environment (WS-AC-05), stream parsing, stop, resume,
  errors, version refusal, `auth status`; `agent-event` updates; the terminal command line
  built per platform (without starting a terminal).
