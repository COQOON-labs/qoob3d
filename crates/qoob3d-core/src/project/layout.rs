//! Names of the files and folders of a project (AG-PJ-01): the one place that
//! spells them, for the service, its clients, `init` and the MCP file tools.

use std::path::{Path, PathBuf};

/// The project file in the project root; its presence makes a folder a project.
pub const CONFIG_FILE: &str = "qoob3d.toml";
/// Folders an agent works in, in the order `qoob3d init` creates them.
pub const FOLDERS: [&str; 5] = ["parts", "lib", "values", "checks", "tasks"];
/// Service data (cache, lock, `service.json`); never saved, never an agent file.
pub const DATA_DIR: &str = ".qoob3d";
/// Generated exports; never saved.
pub const OUT_DIR: &str = "out";
/// Port, pid and token of the running service, inside [`DATA_DIR`].
pub const SERVICE_JSON: &str = "service.json";
/// Lock that keeps one service per project, inside [`DATA_DIR`].
pub const SERVICE_LOCK: &str = "service.lock";
/// Build results per part, inside [`DATA_DIR`].
pub const CACHE_DIR: &str = "cache";

/// `<dir>/.qoob3d`.
pub fn data_dir(dir: &Path) -> PathBuf {
    dir.join(DATA_DIR)
}

/// `<dir>/.qoob3d/service.json`.
pub fn service_json(dir: &Path) -> PathBuf {
    data_dir(dir).join(SERVICE_JSON)
}

/// `<dir>/.qoob3d/cache/<part>`.
pub fn cache_dir(dir: &Path, part: &str) -> PathBuf {
    data_dir(dir).join(CACHE_DIR).join(part)
}
