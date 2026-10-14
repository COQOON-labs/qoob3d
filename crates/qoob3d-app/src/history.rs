//! Undo/redo as immutable document snapshots (ADR-0029).

use std::sync::Arc;

use qoob3d_core::document::Document;

const MAX_ENTRIES: usize = 200;

#[derive(Debug, Clone)]
struct Entry {
    doc: Arc<Document>,
    /// Translation key of what led to this state ("" for the initial state).
    label: &'static str,
}

#[derive(Debug, Clone)]
pub struct History {
    entries: Vec<Entry>,
    /// Index of the current state.
    cursor: usize,
}

impl History {
    pub fn new(doc: Arc<Document>) -> Self {
        Self { entries: vec![Entry { doc, label: "" }], cursor: 0 }
    }

    pub fn current(&self) -> &Arc<Document> {
        &self.entries[self.cursor].doc
    }

    /// Records a new state; anything that could be redone is dropped.
    pub fn push(&mut self, doc: Arc<Document>, label: &'static str) {
        self.entries.truncate(self.cursor + 1);
        self.entries.push(Entry { doc, label });
        if self.entries.len() > MAX_ENTRIES {
            self.entries.remove(0);
        }
        self.cursor = self.entries.len() - 1;
    }

    pub fn undo(&mut self) -> Option<Arc<Document>> {
        (self.cursor > 0).then(|| {
            self.cursor -= 1;
            self.current().clone()
        })
    }

    pub fn redo(&mut self) -> Option<Arc<Document>> {
        (self.cursor + 1 < self.entries.len()).then(|| {
            self.cursor += 1;
            self.current().clone()
        })
    }

    /// Label of the step undo would revert.
    pub fn undo_label(&self) -> Option<&'static str> {
        (self.cursor > 0).then(|| self.entries[self.cursor].label)
    }

    pub fn redo_label(&self) -> Option<&'static str> {
        self.entries.get(self.cursor + 1).map(|e| e.label)
    }

    /// Position marker for [`History::collapse_since`].
    pub fn marker(&self) -> usize {
        self.cursor
    }

    /// Merges every step after `marker` into one step labelled `label`
    /// (a whole sketch session = one timeline step, SK-LC-08).
    pub fn collapse_since(&mut self, marker: usize, label: &'static str) {
        if self.cursor <= marker || marker >= self.entries.len() {
            return;
        }
        let current = self.current().clone();
        self.entries.truncate(marker + 1);
        self.entries.push(Entry { doc: current, label });
        self.cursor = self.entries.len() - 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(n: u64) -> Arc<Document> {
        Arc::new(Document { next_feature_id: n, ..Default::default() })
    }

    #[test]
    fn undo_redo_and_branching() {
        let mut h = History::new(doc(1));
        h.push(doc(2), "a");
        h.push(doc(3), "b");
        assert_eq!(h.undo().unwrap().next_feature_id, 2);
        assert_eq!(h.redo_label(), Some("b"));
        h.push(doc(4), "c"); // drops "b"
        assert!(h.redo().is_none());
        assert_eq!(h.undo().unwrap().next_feature_id, 2);
        assert_eq!(h.undo().unwrap().next_feature_id, 1);
        assert!(h.undo().is_none());
    }

    #[test]
    fn collapse_makes_one_step() {
        let mut h = History::new(doc(1));
        let m = h.marker();
        for n in 2..6 {
            h.push(doc(n), "edit");
        }
        h.collapse_since(m, "sketch");
        assert_eq!(h.current().next_feature_id, 5);
        assert_eq!(h.undo_label(), Some("sketch"));
        assert_eq!(h.undo().unwrap().next_feature_id, 1);
    }
}
