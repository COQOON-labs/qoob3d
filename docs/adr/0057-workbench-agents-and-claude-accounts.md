---
id: ADR-0057
title: The app is a workbench over many projects and runs Claude agents through the human's own installed Claude Code, one config folder per account, without ever handling credentials
status: accepted
date: 2026-11-27
crates: [qoob3d-project, qoob3d-app]
tags: [agent, workspace, accounts, ui]
supersedes: []
superseded_by: null
---

## Context

So far an agent drives one project from outside: Claude Code runs in a terminal and talks to
`qoob3d mcp` (ADR-0054); the app shows one project live (`docs/requirements/project-app.md`).
The human wants it the other way round: an overview of all projects and their state, several
projects open at once, and agents started from the app with one of their own Claude
subscriptions, switching between several (`docs/requirements/workspace.md`).

Facts that bound the decision (Claude Code documentation, read 2026-11-27):

- "Legal and compliance": third-party developers may not offer a claude.ai login in their
  own applications, may not route requests through Free, Pro, Max or Ultra plan credentials
  on behalf of their users, and may not collect, store or intermediate Claude.ai credentials
  or session tokens; sign-in must complete through Anthropic's own flow. It does not prevent
  an end user from signing in to the unmodified Claude Code binary with their own
  subscription.
- "Authentication": several logins on one machine are supported with one `CLAUDE_CONFIG_DIR`
  per account (own settings, history and login); `ANTHROPIC_API_KEY` and
  `CLAUDE_CODE_OAUTH_TOKEN` take precedence over a `/login` session.
- There is no official Rust SDK. The Agent SDKs (TypeScript, Python) wrap the CLI; other
  languages run the CLI as a subprocess (`-p`, `--output-format stream-json`). `--bare` reads
  no subscription login.
- Each project already has its own service, port, token, cache and workers (ADR-0055), so
  several open projects need no change of that design, but their workers compete for memory
  (the Damen project allows 8 GB per worker), and the Damen run showed `/status` blocking
  under load and cache keys that ignore kernel changes.

Use is personal: one human, their own accounts, their own machine.

## Decision

- **Workbench.** The app keeps a list of project folders and shows them on an overview that
  reads files only (`.qoob3d/summary.json` written by each service, `tasks/`, the agent's
  session record, Git); it never starts a service for it. Several projects are open as tabs,
  each with its own poller. Services stop when idle, and all services of the user share one
  machine-wide budget of busy workers (slot lock files). Before that, `/status` gets its
  hashing out of the service lock and the cache key a kernel fingerprint.
- **Agents through the installed CLI.** qoob3d starts the human's own `claude` binary,
  unmodified, in the project folder: either as a background session (`-p`, stream-json in
  and out, only the qoob3d MCP tools may change the project, nothing waits for a permission
  prompt), or interactively in the human's terminal with hooks that report "waiting" and
  "done" back. The review flow of ADR-0054 is unchanged: the gate decides what may go to
  review, the human accepts.
- **Accounts as config folders.** An account is a name and a `CLAUDE_CONFIG_DIR`. Signing in
  and out runs `claude auth login` / `logout` in a terminal with that folder, so it is always
  Anthropic's flow. qoob3d sets `CLAUDE_CONFIG_DIR` for every process it starts, removes
  `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN` and `CLAUDE_CODE_OAUTH_TOKEN` from their
  environment, and never reads, stores or passes on a credential or token. It never switches
  accounts by itself.
- **Where the code lives.** Everything without UI (project list, overview reader, accounts,
  sessions, stream parser) goes into `qoob3d-project` (ADR-0036); tests use a fake `claude`
  (`QOOB3D_CLAUDE_CMD`), a real session only in an opt-in test.

## Consequences

- The app becomes the place where the human sees all work and what needs them; agents still
  only touch a project through MCP and the gate, so the existing checks keep their meaning.
- qoob3d depends on the CLI's stream format and flags, which are not a stable, fully
  documented protocol (the input stream schema is not documented). The parser must be
  tolerant, the oldest supported CLI version is checked before a session starts, and the
  recorded streams of the fake `claude` are updated when the CLI changes.
- Usage limits of a subscription end a session with Claude's message; parallel sessions on
  several projects share those limits. There are no retries and no automatic account switch.
- Distributing qoob3d to other people with this feature would need a different auth path
  (their own API keys, or Anthropic's approval); this ADR covers personal use only.
- No credentials ever enter qoob3d's settings, logs or project folders.

## Rejected alternatives

- **Agent SDK in a TypeScript or Python sidecar** – a second runtime to ship and update for
  what the CLI already offers in stream-json; the SDK runs the same CLI underneath.
- **Own agent loop on the Messages API** – needs API keys for everything and would rebuild
  tools, permissions and session handling that Claude Code has.
- **A login screen in the app, or storing a `claude setup-token` token** – exactly what the
  policy forbids (collecting or intermediating credentials).
- **Community Rust crates for the CLI protocol** – not published by Anthropic, not vetted;
  the subset we need (start, stream, stop, resume) is small.
- **One service for all projects** – would give up the isolation of ADR-0055 (lock, token,
  limits per folder) for a saving the idle shutdown and the worker budget already give.
- **Several agents per project now** – they would write the same files; Git worktrees can
  come later.

## References

- Requirements: `docs/requirements/workspace.md` (WS-*)
- Related: ADR-0036, ADR-0053, ADR-0054, ADR-0055
- Claude Code docs: legal-and-compliance, authentication, headless, cli-reference,
  agent-sdk/overview (code.claude.com/docs)
