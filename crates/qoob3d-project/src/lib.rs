//! QOOB3D script projects (ADR-0053, ADR-0054, ADR-0055): everything that
//! builds and serves a project folder, without GPU or UI.
//!
//! - [`service`]: `qoob3d serve <DIR>`, the local HTTP service with jobs,
//!   part states, checks, tasks and the worker pool.
//! - [`worker`]: `qoob3d worker`, the only process that runs OCCT for a
//!   project build.
//! - [`client`]: the project folder and the HTTP client every caller of the
//!   service uses (app, CLI, MCP, `render-project`).
//! - [`mcp`]: `qoob3d mcp [DIR]`, the MCP server for AI agents.
//! - [`init`]: `qoob3d init <DIR>`, the skeleton of a new project.
//!
//! The binary `qoob3d` (crate `qoob3d-app`) dispatches its subcommands here.

pub mod client;
pub mod init;
pub mod mcp;
pub mod service;
pub mod worker;

/// Name of the `qoob3d render-project` subcommand, for callers that start it
/// as a child process (the MCP tool `render`).
pub const RENDER_PROJECT: &str = "render-project";
