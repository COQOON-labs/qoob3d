//! The open project: document + undo history + file + derived data from
//! the kernel (profiles). All document changes go through here so that
//! undo, the dirty flag and rebuilds stay consistent (ADR-0030, ADR-0029).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use qoob3d_core::document::{Document, FeatureId, LoadError, MAX_FILE_BYTES, Sketch};
use qoob3d_core::profile::Profile;
use qoob3d_kernel::{BodyResult, Kernel, RebuildResult, Request, RequestId};

use crate::history::History;

#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Load(#[from] LoadError),
}

pub struct Project {
    doc: Arc<Document>,
    history: History,
    /// Snapshot at the last save/load; dirty = different snapshot.
    saved: Arc<Document>,
    pub path: Option<PathBuf>,
    profiles: BTreeMap<FeatureId, Vec<Profile>>,
    errors: BTreeMap<FeatureId, String>,
    bodies: Vec<BodyResult>,
    /// Increments whenever `bodies` change (the viewport re-uploads then).
    bodies_version: u64,
    pending: Option<RequestId>,
    needs_rebuild: bool,
}

impl Default for Project {
    fn default() -> Self {
        Self::with_document(Document::default(), None)
    }
}

impl Project {
    pub fn with_document(doc: Document, path: Option<PathBuf>) -> Self {
        let doc = Arc::new(doc);
        Self {
            history: History::new(doc.clone()),
            saved: doc.clone(),
            doc,
            path,
            profiles: BTreeMap::new(),
            errors: BTreeMap::new(),
            bodies: Vec::new(),
            bodies_version: 0,
            pending: None,
            needs_rebuild: true,
        }
    }

    pub fn doc(&self) -> &Arc<Document> {
        &self.doc
    }

    pub fn history(&self) -> &History {
        &self.history
    }

    pub fn is_dirty(&self) -> bool {
        !Arc::ptr_eq(&self.doc, &self.saved)
    }

    /// File name for the title bar ("Untitled" is translated by the caller).
    pub fn file_name(&self) -> Option<String> {
        self.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned())
    }

    /// Applies a change as one undo step.
    pub fn edit(&mut self, label: &'static str, change: impl FnOnce(&mut Document)) {
        let mut doc = (*self.doc).clone();
        change(&mut doc);
        self.set(Arc::new(doc));
        self.history.push(self.doc.clone(), label);
    }

    pub fn replace_sketch(&mut self, feature: FeatureId, sketch: Sketch, label: &'static str) {
        self.edit(label, |d| {
            if let Some(s) = d.sketch_mut(feature) {
                *s = sketch;
            }
        });
    }

    /// Live change without an undo step (e.g. while dragging); finish with
    /// [`Project::commit_preview`].
    pub fn preview_sketch(&mut self, feature: FeatureId, sketch: Sketch) {
        let mut doc = (*self.doc).clone();
        if let Some(s) = doc.sketch_mut(feature) {
            *s = sketch;
        }
        self.set(Arc::new(doc));
    }

    /// Live document change without an undo step (feature dialogs).
    pub fn preview_doc(&mut self, doc: Document) {
        if *self.doc != doc {
            self.set(Arc::new(doc));
        }
    }

    /// Drops a preview: back to the last undo snapshot.
    pub fn cancel_preview(&mut self) {
        if !Arc::ptr_eq(&self.doc, self.history.current()) {
            let current = self.history.current().clone();
            self.set(current);
        }
    }

    pub fn commit_preview(&mut self, label: &'static str) {
        if !Arc::ptr_eq(&self.doc, self.history.current()) {
            self.history.push(self.doc.clone(), label);
        }
    }

    pub fn undo(&mut self) -> bool {
        self.commit_preview("edit-move");
        match self.history.undo() {
            Some(doc) => {
                self.set(doc);
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self) -> bool {
        match self.history.redo() {
            Some(doc) => {
                self.set(doc);
                true
            }
            None => false,
        }
    }

    pub fn history_marker(&self) -> usize {
        self.history.marker()
    }

    pub fn collapse_history(&mut self, marker: usize, label: &'static str) {
        self.history.collapse_since(marker, label);
    }

    fn set(&mut self, doc: Arc<Document>) {
        self.doc = doc;
        self.needs_rebuild = true;
    }

    // --- kernel ------------------------------------------------------------

    /// Submits a rebuild if the document changed since the last one.
    pub fn sync(&mut self, kernel: &mut Kernel) {
        if self.needs_rebuild {
            self.needs_rebuild = false;
            match kernel.submit(Request::Rebuild { document: self.doc.clone() }) {
                Ok(id) => self.pending = Some(id),
                Err(err) => tracing::error!(%err, "kernel unavailable"),
            }
        }
    }

    /// Takes a rebuild result; stale results are ignored (ADR-0005).
    pub fn accept_rebuild(&mut self, id: RequestId, result: RebuildResult) {
        if self.pending != Some(id) {
            return;
        }
        self.pending = None;
        self.profiles.clear();
        self.errors = result.errors.into_iter().collect();
        for sketch in result.sketches {
            if let Ok(p) = sketch.profiles {
                self.profiles.insert(sketch.feature, p);
            }
        }
        let changed = self.bodies.len() != result.bodies.len()
            || self.bodies.iter().zip(&result.bodies).any(|(a, b)| !Arc::ptr_eq(&a.mesh, &b.mesh));
        self.bodies = result.bodies;
        if changed {
            self.bodies_version += 1;
        }
    }

    pub fn bodies(&self) -> &[BodyResult] {
        &self.bodies
    }

    pub fn bodies_version(&self) -> u64 {
        self.bodies_version
    }

    pub fn is_rebuilding(&self) -> bool {
        self.pending.is_some() || self.needs_rebuild
    }

    pub fn profiles(&self, feature: FeatureId) -> &[Profile] {
        self.profiles.get(&feature).map_or(&[], Vec::as_slice)
    }

    pub fn error(&self, feature: FeatureId) -> Option<&str> {
        self.errors.get(&feature).map(String::as_str)
    }

    // --- files ---------------------------------------------------------------

    pub fn open(path: &Path) -> Result<Project, FileError> {
        let size = std::fs::metadata(path)?.len();
        if size > MAX_FILE_BYTES as u64 {
            return Err(LoadError::TooLarge.into());
        }
        let text = std::fs::read_to_string(path)?;
        let doc = Document::from_json(&text)?;
        Ok(Project::with_document(doc, Some(path.to_path_buf())))
    }

    /// Writes atomically (temp file + rename) and marks the state as saved.
    pub fn save_to(&mut self, path: &Path) -> Result<(), FileError> {
        let tmp = path.with_extension("qbd.tmp");
        std::fs::write(&tmp, self.doc.to_json())?;
        std::fs::rename(&tmp, path)?;
        self.path = Some(path.to_path_buf());
        self.saved = self.doc.clone();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qoob3d_core::document::{FeatureKind, OriginPlane, PlaneRef};

    fn add_sketch(p: &mut Project) -> FeatureId {
        let mut id = FeatureId(0);
        p.edit("sketch-create", |d| {
            id = d.add_feature(
                "Sketch1".into(),
                FeatureKind::Sketch(Sketch::new(PlaneRef::Origin(OriginPlane::XY))),
            );
        });
        id
    }

    #[test]
    fn dirty_tracking_follows_undo() {
        let mut p = Project::default();
        assert!(!p.is_dirty());
        add_sketch(&mut p);
        assert!(p.is_dirty());
        assert!(p.undo());
        assert!(!p.is_dirty(), "back at the saved snapshot");
        assert!(p.redo());
        assert!(p.is_dirty());
    }

    #[test]
    fn save_and_open_round_trip() {
        let dir = std::env::temp_dir().join(format!("qoob3d-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("part.qbd");
        let mut p = Project::default();
        let id = add_sketch(&mut p);
        p.save_to(&path).unwrap();
        assert!(!p.is_dirty());
        let q = Project::open(&path).unwrap();
        assert_eq!(**q.doc(), **p.doc());
        assert!(q.doc().sketch(id).is_some());
        assert!(!path.with_extension("qbd.tmp").exists(), "temp file renamed");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn preview_becomes_one_undo_step() {
        let mut p = Project::default();
        let id = add_sketch(&mut p);
        let mut s = p.doc().sketch(id).unwrap().clone();
        for x in 0..5 {
            s.add_point([f64::from(x), 0.0]);
            p.preview_sketch(id, s.clone());
        }
        p.commit_preview("edit-move");
        assert_eq!(p.doc().sketch(id).unwrap().points.len(), 5);
        assert!(p.undo());
        assert!(p.doc().sketch(id).unwrap().points.is_empty());
    }
}
