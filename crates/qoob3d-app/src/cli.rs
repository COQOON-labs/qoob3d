//! Command-line interface: the app (optionally with `--project`), the
//! project commands for agents and scripts (ADR-0053) and the smoke test of
//! the quality harness (ADR-0013).

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use qoob3d_project::RENDER_PROJECT;
use qoob3d_render::ViewPreset;

#[derive(Debug, Parser)]
#[command(name = "qoob3d", version, about = "QOOB3D – parametric CAD for FDM printing")]
#[command(args_conflicts_with_subcommands = true)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Open the script project in this folder (with `qoob3d.toml`) and show
    /// its parts live (WA-OP-01).
    #[arg(long, value_name = "DIR", conflicts_with = "smoke_test")]
    pub project: Option<PathBuf>,

    /// Start the real app, drive it with synthetic input, verify the effect
    /// and exit with code 0 (ok) or 1 (failure). A watchdog catches hangs.
    #[arg(long)]
    pub smoke_test: bool,
}

/// Script project commands (ADR-0053, docs/requirements/agent.md §2–3).
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the project service for DIR (HTTP on 127.0.0.1, AG-SV-01).
    Serve { dir: PathBuf },
    /// Build worker process of the service (AG-PJ-05); talks JSON lines on stdin/stdout.
    #[command(hide = true)]
    Worker,
    /// Build all parts of DIR uncached twice and print seconds, peak worker
    /// memory, triangles and mesh bytes per part (AG-PF-06).
    Bench { dir: PathBuf },
    /// Create a project skeleton in DIR (AG-SV-08).
    Init { dir: PathBuf },
    /// Render the built parts of DIR headless into a PNG (AG-RD-01).
    #[command(name = RENDER_PROJECT)]
    RenderProject {
        dir: PathBuf,
        #[arg(long, value_name = "PNG")]
        out: PathBuf,
        #[arg(long, value_enum, default_value_t = View::Iso)]
        view: View,
        /// Several views, comma separated: one contact sheet with a label
        /// per view instead of `--view` (AG-MC-09).
        #[arg(long, value_enum, value_delimiter = ',', conflicts_with = "view")]
        views: Vec<View>,
        /// Image size, e.g. 1024x768.
        #[arg(long, value_parser = parse_size, default_value = "1024x768")]
        size: [u32; 2],
        /// Parts to show, comma separated; all without it.
        #[arg(long, value_delimiter = ',')]
        parts: Vec<String>,
        /// A throwaway script whose shown shapes are rendered instead of
        /// parts (AG-MC-12).
        #[arg(long, value_name = "RHAI", conflicts_with = "parts")]
        source: Option<PathBuf>,
        /// Frame this box instead of everything: x0,y0,z0,x1,y1,z1 in mm
        /// (a zoom, AG-MC-12).
        #[arg(long, value_parser = parse_region, allow_hyphen_values = true)]
        region: Option<[f32; 6]>,
    },
    /// MCP server for the project in DIR (default: the working directory)
    /// over stdio (AG-MC-01).
    Mcp { dir: Option<PathBuf> },
    #[command(flatten)]
    Project(ProjectCommand),
}

/// Clients of the service of the project in the working directory
/// (AG-SV-02): they print the JSON envelope and exit with 1 on failure.
#[derive(Debug, Subcommand)]
pub enum ProjectCommand {
    /// Build parts (all without names) and their dependencies.
    Build {
        parts: Vec<String>,
        /// Seconds to wait for the result; afterwards the job id is printed.
        #[arg(long, default_value_t = 300.0)]
        wait: f64,
    },
    /// Parts with build state and cache key, jobs, workers.
    Status,
    /// Write out/<part>.<format> (built first if needed).
    Export {
        #[arg(required = true)]
        parts: Vec<String>,
        /// stl, 3mf (one object per part) or step.
        #[arg(long)]
        format: String,
        /// One file out/KIT.<format> with all parts, each in its print pose
        /// and count, arranged on the bed (AG-SV-09).
        #[arg(long)]
        kit: Option<String>,
        /// Bed width for the arrangement of a kit (mm, default 256).
        #[arg(long)]
        bed_mm: Option<f32>,
        /// One Bambu Studio project out/PROJECT.3mf: all parts on plates,
        /// each in the filament of its colour (AG-SV-10).
        #[arg(long, conflicts_with_all = ["kit", "bed_mm"])]
        project: Option<String>,
        /// For --project: parts with `print_with` print inside their host
        /// as multi-colour objects (AMS); without it every plate holds
        /// parts of one colour.
        #[arg(long, requires = "project")]
        multicolor: bool,
        /// For --project: printer whose bed and presets the project uses,
        /// e.g. bambu-lab-p1s (default), bambu-lab-a1, bambu-lab-x1-carbon.
        #[arg(long, requires = "project")]
        printer: Option<String>,
    },
    /// Target values with source and status, and the assumptions.
    Values,
    /// Run checks (all without names) after building stale parts; exit 1
    /// if any failed (AG-CK-03).
    Check { names: Vec<String> },
    /// The task inbox, acting as the human (AG-SV-04).
    Task {
        #[command(subcommand)]
        command: TaskCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum TaskCommand {
    /// All tasks with their states.
    List,
    /// One task with body and log.
    Show { id: String },
    /// Create a task.
    New {
        title: String,
        /// task, requirement or idea.
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        body: Option<String>,
    },
    /// Set the state of a task; every transition is allowed.
    Set {
        id: String,
        state: String,
        #[arg(long)]
        note: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum View {
    Iso,
    Front,
    Back,
    Left,
    Right,
    Top,
    Bottom,
}

impl From<View> for ViewPreset {
    fn from(v: View) -> Self {
        match v {
            View::Iso => ViewPreset::Iso,
            View::Front => ViewPreset::Front,
            View::Back => ViewPreset::Back,
            View::Left => ViewPreset::Left,
            View::Right => ViewPreset::Right,
            View::Top => ViewPreset::Top,
            View::Bottom => ViewPreset::Bottom,
        }
    }
}

fn parse_region(s: &str) -> Result<[f32; 6], String> {
    let values: Vec<f32> = s
        .split(',')
        .map(|v| v.trim().parse::<f32>().map_err(|e| format!("'{v}': {e}")))
        .collect::<Result<_, _>>()?;
    let [x0, y0, z0, x1, y1, z1] = values[..] else {
        return Err("expected six numbers x0,y0,z0,x1,y1,z1".to_owned());
    };
    if !(x0 < x1 && y0 < y1 && z0 <= z1) || values.iter().any(|v| !v.is_finite()) {
        return Err("the region needs finite numbers with x0 < x1, y0 < y1, z0 <= z1".to_owned());
    }
    Ok([x0, y0, z0, x1, y1, z1])
}

fn parse_size(s: &str) -> Result<[u32; 2], String> {
    let (w, h) = s.split_once(['x', 'X']).ok_or("expected WIDTHxHEIGHT, e.g. 1280x800")?;
    let parse = |v: &str| {
        v.trim()
            .parse::<u32>()
            .ok()
            .filter(|n| (16..=8192).contains(n))
            .ok_or_else(|| format!("'{v}' is not a size between 16 and 8192"))
    };
    Ok([parse(w)?, parse(h)?])
}

#[cfg(test)]
mod tests;
