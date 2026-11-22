//! `qoob3d init <DIR>` (AG-SV-08, ADR-0054): the skeleton of a script
//! project. A folder that already is a project is refused; anything else
//! in the folder stays as it is.

use std::path::Path;

use anyhow::{Context as _, bail};

/// Folders of a project (AG-PJ-01, ADR-0054).
const FOLDERS: [&str; 5] = ["parts", "lib", "values", "checks", "tasks"];
/// What Git must not track: service data and generated output.
const IGNORED: [&str; 2] = [".qoob3d/", "out/"];

/// Creates the skeleton in `dir`.
///
/// # Errors
/// `dir` already has `qoob3d.toml`, or a folder or file cannot be written.
pub fn run(dir: &Path) -> anyhow::Result<()> {
    let config = dir.join("qoob3d.toml");
    if config.exists() {
        bail!("{} already is a QOOB3D project (qoob3d.toml exists)", dir.display());
    }
    for folder in FOLDERS {
        let path = dir.join(folder);
        std::fs::create_dir_all(&path).with_context(|| format!("creating {}", path.display()))?;
    }
    let full = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let name = full.file_name().map_or_else(|| "project".into(), |n| n.to_string_lossy());
    // A JSON string is a valid TOML basic string.
    let text = format!("[project]\nname = {}\nunits = \"mm\"\n", serde_json::json!(name));
    std::fs::write(&config, text).with_context(|| format!("writing {}", config.display()))?;

    let ignore = dir.join(".gitignore");
    let mut lines = std::fs::read_to_string(&ignore).unwrap_or_default();
    for entry in IGNORED {
        if !lines.lines().any(|l| l.trim().trim_start_matches('/') == entry) {
            if !lines.is_empty() && !lines.ends_with('\n') {
                lines.push('\n');
            }
            lines.push_str(entry);
            lines.push('\n');
        }
    }
    std::fs::write(&ignore, lines).with_context(|| format!("writing {}", ignore.display()))?;
    println!("created a QOOB3D project in {}", dir.display());
    Ok(())
}
