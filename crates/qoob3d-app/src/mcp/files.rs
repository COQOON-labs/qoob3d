//! The confined file tools (AG-MC-03): paths relative to the project, only
//! `qoob3d.toml` and files under the five project folders, only text
//! formats, at most 1 MiB. Any symbolic link on the way is refused, so a
//! path means the same file for the check and for the access. Task files
//! are read-only here: they change through the task tools and their role
//! rules.

use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};

use qoob3d_core::fs::write_atomic;
use qoob3d_core::project::layout;

const EXTENSIONS: [&str; 3] = ["rhai", "toml", "md"];
const MAX_BYTES: u64 = 1 << 20;

/// The plain components of a project-relative path.
fn components(relative: &str) -> Result<Vec<&str>, String> {
    let path = Path::new(relative);
    let parts: Option<Vec<&str>> = path
        .components()
        .map(|c| match c {
            Component::Normal(name) => name.to_str().filter(|n| *n != layout::DATA_DIR),
            _ => None,
        })
        .collect();
    parts.filter(|p| !p.is_empty()).ok_or_else(|| {
        format!("{relative:?} must be a path inside the project without `..` and `.qoob3d`")
    })
}

/// `root` joined with `parts`; no existing component may be a symlink.
fn without_links(root: &Path, parts: &[&str]) -> Result<PathBuf, String> {
    let mut path = root.to_owned();
    for part in parts {
        path.push(part);
        if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(format!("{} is a symbolic link", parts.join("/")));
        }
    }
    Ok(path)
}

fn allowed_extension(name: &str) -> bool {
    Path::new(name).extension().is_some_and(|e| EXTENSIONS.iter().any(|x| e == *x))
}

/// The file `relative` if the file tools may touch it.
fn file(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let parts = components(relative)?;
    let placed = match parts.as_slice() {
        [name] => *name == layout::CONFIG_FILE,
        [folder, ..] => layout::FOLDERS.contains(folder),
        [] => false,
    };
    if !placed {
        return Err(format!(
            "{relative:?}: only {} and files under parts/, lib/, values/, checks/ and tasks/",
            layout::CONFIG_FILE
        ));
    }
    if !parts.last().is_some_and(|name| allowed_extension(name)) {
        return Err(format!("{relative:?}: only .rhai, .toml and .md files"));
    }
    without_links(root, &parts)
}

fn too_large(relative: &str, bytes: u64) -> String {
    format!("{relative:?} has {bytes} bytes; the limit is {MAX_BYTES} (1 MiB)")
}

/// `file_read {path}` → `{path, content}`.
pub(super) fn read(root: &Path, relative: &str) -> Result<Value, String> {
    let path = file(root, relative)?;
    let meta = std::fs::metadata(&path).map_err(|e| format!("{relative:?}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{relative:?} is not a file"));
    }
    if meta.len() > MAX_BYTES {
        return Err(too_large(relative, meta.len()));
    }
    let content = std::fs::read_to_string(&path).map_err(|e| format!("{relative:?}: {e}"))?;
    Ok(json!({"path": relative, "content": content}))
}

/// `file_write {path, content}` → `{path, bytes}`; folders are created.
pub(super) fn write(root: &Path, relative: &str, content: &str) -> Result<Value, String> {
    let path = file(root, relative)?;
    if components(relative)?.first() == Some(&"tasks") {
        return Err(format!(
            "{relative:?}: task files change only through task_create and task_update"
        ));
    }
    if content.len() as u64 > MAX_BYTES {
        return Err(too_large(relative, content.len() as u64));
    }
    if path.is_dir() {
        return Err(format!("{relative:?} is a folder"));
    }
    let io = |e: std::io::Error| format!("{relative:?}: {e}");
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(io)?;
    }
    write_atomic(&path, content.as_bytes()).map_err(io)?;
    Ok(json!({"path": relative, "bytes": content.len()}))
}

/// The allowed files below `folder` (project-relative `prefix`), without
/// following links.
fn walk(folder: &Path, prefix: &str, files: &mut Vec<String>) {
    for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else { continue };
        let relative = format!("{prefix}/{name}");
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => walk(&entry.path(), &relative, files),
            Ok(kind) if kind.is_file() && allowed_extension(&name) => files.push(relative),
            _ => {}
        }
    }
}

/// `file_list {dir?}` → `{files}`: sorted project-relative paths.
pub(super) fn list(root: &Path, dir: Option<&str>) -> Result<Value, String> {
    let mut files = Vec::new();
    if let Some(dir) = dir {
        let parts = components(dir)?;
        if !parts.first().is_some_and(|folder| layout::FOLDERS.contains(folder)) {
            return Err(format!("{dir:?}: only parts/, lib/, values/, checks/ and tasks/"));
        }
        walk(&without_links(root, &parts)?, &parts.join("/"), &mut files);
    } else {
        if std::fs::symlink_metadata(root.join(layout::CONFIG_FILE)).is_ok_and(|m| m.is_file()) {
            files.push(layout::CONFIG_FILE.to_owned());
        }
        for folder in layout::FOLDERS {
            if let Ok(path) = without_links(root, &[folder]) {
                walk(&path, folder, &mut files);
            }
        }
    }
    files.sort();
    Ok(json!({"files": files}))
}
