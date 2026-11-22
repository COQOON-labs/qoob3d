//! Command-line interface. Besides the normal GUI start, the app offers
//! machine-checkable modes for the quality harness (ADR-0013).

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use qoob3d_render::ViewPreset;

#[derive(Debug, Parser)]
#[command(name = "qoob3d", version, about = "QOOB3D – parametric CAD for FDM printing")]
#[command(args_conflicts_with_subcommands = true)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// `.qbd` file to open.
    pub file: Option<PathBuf>,

    /// Open the script project in this folder (with `qoob3d.toml`) and show
    /// its parts live; with --render, build and render them headless
    /// (WA-OP-01, WA-HL-01).
    #[arg(long, value_name = "DIR", conflicts_with_all = ["file", "export", "report", "smoke_test"])]
    pub project: Option<PathBuf>,

    /// Show the demo body (block with a hole) in addition to the document.
    #[arg(long)]
    pub demo: bool,

    /// Render headless (no window) into this PNG file and exit.
    #[arg(long, value_name = "PNG")]
    pub render: Option<PathBuf>,

    /// View for --render.
    #[arg(long, value_enum, default_value_t = View::Iso)]
    pub view: View,

    /// Image size for --render, e.g. 1280x800.
    #[arg(long, value_parser = parse_size, default_value = "1280x800")]
    pub size: [u32; 2],

    /// Orthographic instead of perspective projection for --render.
    #[arg(long)]
    pub ortho: bool,

    /// Camera distance factor for --render after fitting (>1 zooms out).
    #[arg(long, default_value_t = 1.0)]
    pub zoom: f32,

    /// Printer whose build plate --render shows (id, e.g. bambu-lab-h2d).
    #[arg(long)]
    pub printer: Option<String>,

    /// Show the build volume wireframe in --render.
    #[arg(long)]
    pub volume: bool,

    /// Export the bodies of FILE headless to this .stl or .3mf and exit.
    #[arg(long, value_name = "STL|3MF", requires = "file")]
    pub export: Option<PathBuf>,

    /// Rebuild FILE headless and print a JSON report (bodies, volumes,
    /// validity, feature errors). Exit code 1 if anything is not printable.
    #[arg(long, requires = "file")]
    pub report: bool,

    /// Start the real app, drive it with synthetic input, verify the effect
    /// and exit with code 0 (ok) or 1 (failure). A watchdog catches hangs.
    #[arg(long, conflicts_with = "render")]
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
    },
    /// Target values with source and status, and the assumptions.
    Values,
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
