//! `qoob3d init <DIR>` (AG-SV-08, ADR-0054): the skeleton of a script
//! project. A folder that already is a project is refused; anything else
//! in the folder stays as it is. `lib/examples/` gets the files the
//! examples of the script API reference read, so every example runs as
//! written (AG-MC-08).

use std::path::Path;

use anyhow::{Context as _, bail};
use qoob3d_core::project::layout;

/// What Git must not track: service data and generated output.
const IGNORED: [&str; 2] = [layout::DATA_DIR, layout::OUT_DIR];
/// `(path, content)` of the files the reference examples import; the master
/// copies live beside the reference.
const EXAMPLES: [(&str, &[u8]); 6] = [
    ("lib/examples/fdm.rhai", include_bytes!("../../../docs/script-api/lib/examples/fdm.rhai")),
    ("lib/examples/block.step", include_bytes!("../../../docs/script-api/lib/examples/block.step")),
    (
        "lib/examples/bracket.step",
        include_bytes!("../../../docs/script-api/lib/examples/bracket.step"),
    ),
    ("lib/examples/angle.3mf", include_bytes!("../../../docs/script-api/lib/examples/angle.3mf")),
    // The font of the `text` examples and its licence, which travels with it.
    (
        "lib/examples/DejaVuSans-Bold.ttf",
        include_bytes!("../../../docs/script-api/lib/examples/DejaVuSans-Bold.ttf"),
    ),
    (
        "lib/examples/DejaVuSans-Bold.LICENSE.txt",
        include_bytes!("../../../docs/script-api/lib/examples/DejaVuSans-Bold.LICENSE.txt"),
    ),
];

/// Creates the skeleton in `dir`.
///
/// # Errors
/// `dir` already has `qoob3d.toml`, or a folder or file cannot be written.
pub fn run(dir: &Path) -> anyhow::Result<()> {
    let config = dir.join(layout::CONFIG_FILE);
    if config.exists() {
        bail!("{} already is a QOOB3D project (qoob3d.toml exists)", dir.display());
    }
    for folder in layout::FOLDERS {
        let path = dir.join(folder);
        std::fs::create_dir_all(&path).with_context(|| format!("creating {}", path.display()))?;
    }
    for (file, content) in EXAMPLES {
        let path = dir.join(file);
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)
                .with_context(|| format!("creating {}", folder.display()))?;
        }
        // A file of the same name that is already there is the user's.
        if !path.exists() {
            std::fs::write(&path, content)
                .with_context(|| format!("writing {}", path.display()))?;
        }
    }
    let full = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let name = full.file_name().map_or_else(|| "project".into(), |n| n.to_string_lossy());
    // A JSON string is a valid TOML basic string.
    let text = format!("[project]\nname = {}\nunits = \"mm\"\n", serde_json::json!(name));
    std::fs::write(&config, text).with_context(|| format!("writing {}", config.display()))?;

    let ignore = dir.join(".gitignore");
    let mut lines = std::fs::read_to_string(&ignore).unwrap_or_default();
    for entry in IGNORED.map(|folder| format!("{folder}/")) {
        if !lines.lines().any(|l| l.trim().trim_start_matches('/') == entry) {
            if !lines.is_empty() && !lines.ends_with('\n') {
                lines.push('\n');
            }
            lines.push_str(&entry);
            lines.push('\n');
        }
    }
    std::fs::write(&ignore, lines).with_context(|| format!("writing {}", ignore.display()))?;
    println!("created a QOOB3D project in {}", dir.display());
    Ok(())
}
