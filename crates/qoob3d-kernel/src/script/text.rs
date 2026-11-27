//! `text` of the script API (AG-SC-38): a line of lettering as a profile.
//! The font is a file of the project, read like an import and recorded in
//! `RunOutput::files_read` for the cache key (AG-SC-24), once per run.
//! `qoob3d_core::font` sets the glyphs as polygons with holes; here they
//! become faces.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use rhai::{Engine, Map};

use qoob3d_core::font::{Align, FontError, MAX_FONT_BYTES, Setting, lettering};

use super::api::{Rr, finite, options, positive, runtime};
use super::profile::{self, Profile};
use super::{RunOutput, project_file};

/// The bytes of the fonts a run has read.
type Fonts = Rc<RefCell<HashMap<PathBuf, Rc<[u8]>>>>;

/// The font file `path` of the project.
fn font(
    root: Option<&PathBuf>,
    path: &str,
    output: &RefCell<RunOutput>,
    fonts: &Fonts,
) -> Rr<Rc<[u8]>> {
    let Some(root) = root else {
        return Err(runtime("text needs a project folder"));
    };
    let file = project_file(root, path, output).map_err(|e| runtime(format!("text: font {e}")))?;
    if let Some(read) = fonts.borrow().get(&file) {
        return Ok(Rc::clone(read));
    }
    let bytes = read(&file).map_err(|e| runtime(format!("text: font {path}: {e}")))?;
    fonts.borrow_mut().insert(file, Rc::clone(&bytes));
    Ok(bytes)
}

/// The file, unless it is larger than a font may be: the size is checked
/// before anything is read.
fn read(file: &Path) -> Result<Rc<[u8]>, String> {
    let size = std::fs::metadata(file).map_err(|e| e.to_string())?.len();
    if size > MAX_FONT_BYTES as u64 {
        let size = usize::try_from(size).unwrap_or(usize::MAX);
        return Err(FontError::TooLarge(size).to_string());
    }
    std::fs::read(file).map(Rc::from).map_err(|e| e.to_string())
}

/// The options `font`, `height`, `spacing` and `align`.
fn setting(map: &Map) -> Rr<(String, Setting)> {
    let map = options(map, "text", &["font", "height", "spacing", "align"])?;
    let Some(font) = map.get("font") else {
        return Err(runtime(
            "text: the option font is missing (a .ttf or .otf file of the project)",
        ));
    };
    let font = font
        .clone()
        .into_string()
        .map_err(|t| runtime(format!("text: font must be the path of a font file, got {t}")))?;
    let Some(height) = map.get("height") else {
        return Err(runtime("text: the option height is missing (the cap height in mm)"));
    };
    let height = positive(height, "text: height")?;
    let spacing = map.get("spacing").map_or(Ok(0.0), |v| finite(v, "text: spacing"))?;
    let align = match map.get("align").map(|v| v.clone().into_string()) {
        None => Align::Left,
        Some(Ok(name)) if name == "left" => Align::Left,
        Some(Ok(name)) if name == "center" => Align::Center,
        Some(Ok(name)) if name == "right" => Align::Right,
        Some(Ok(other)) => {
            return Err(runtime(format!(
                "text: align must be \"left\", \"center\" or \"right\", got {other:?}"
            )));
        }
        Some(Err(t)) => {
            return Err(runtime(format!(
                "text: align must be \"left\", \"center\" or \"right\", got {t}"
            )));
        }
    };
    Ok((font, Setting { height, spacing, align }))
}

pub(super) fn register(
    engine: &mut Engine,
    output: &Rc<RefCell<RunOutput>>,
    project: Option<PathBuf>,
) {
    engine.register_fn("text", |_: &str| -> Rr<Profile> {
        Err(runtime("text needs its options: text(string, #{ font: path, height: mm })"))
    });
    let fonts = Fonts::default();
    let files = Rc::clone(output);
    engine.register_fn("text", move |string: &str, options: Map| -> Rr<Profile> {
        let (path, setting) = setting(&options)?;
        let bytes = font(project.as_ref(), &path, &files, &fonts)?;
        let set = lettering(&bytes, string, &setting).map_err(|e| match e {
            FontError::NotAFont(_) | FontError::NoHeight => runtime(format!("text: {path}: {e}")),
            _ => runtime(format!("text: {e}")),
        })?;
        profile::regions("text", &set.regions, set.overlapping)
    });
}
