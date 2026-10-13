//! Command-line interface. Besides the normal GUI start, the app offers
//! machine-checkable modes for the quality harness (ADR-0013).

use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use qoob3d_render::ViewPreset;

#[derive(Debug, Parser)]
#[command(name = "qoob3d", version, about = "QOOB3D – parametrisches CAD für den FDM-Druck")]
pub struct Cli {
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

    /// Start the real app, drive it with synthetic input, verify the effect
    /// and exit with code 0 (ok) or 1 (failure). A watchdog catches hangs.
    #[arg(long, conflicts_with = "render")]
    pub smoke_test: bool,
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
mod tests {
    use super::*;

    #[test]
    fn size_parsing() {
        assert_eq!(parse_size("1280x800"), Ok([1280, 800]));
        assert_eq!(parse_size("64X64"), Ok([64, 64]));
        assert!(parse_size("1280").is_err());
        assert!(parse_size("0x10").is_err());
        assert!(parse_size("99999x10").is_err());
    }

    #[test]
    fn cli_definition_is_valid() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }
}
