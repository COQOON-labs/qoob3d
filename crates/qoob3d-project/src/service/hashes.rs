//! Digests of the files behind cache keys and declarations, remembered by
//! path, size and modification time (WS-PRE-01, ADR-0055). Without it every
//! `/status` read and hashed every module and imported file of every part,
//! inside the service lock: with 108 parts and 66 MB of imported STEP files
//! (the Damen 5009 project) it stopped answering for minutes.
//!
//! Callers never hold the service lock while they ask. A digest is
//! remembered only when the file was older than [`RACY`] when its reading
//! began: a file written again within the resolution of its timestamp
//! keeps size and modification time, so a younger one is read again next
//! time (the "racy" case of Git's index).
//!
//! [`Hashes::known`] never waits for a large file: `/status` and `/mesh`
//! use it, and a file above [`INLINE_BYTES`] that is not remembered goes to
//! one background thread. Builds, checks and the review gate use
//! [`Hashes::digest`], which hashes what it needs.
//!
//! [`Hashes::key`] remembers the cache key of every part with its inputs:
//! `/status` asks for 100 and more keys every poll, and a key is hashed
//! again only when an input changed.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use qoob3d_core::project::{
    self, FileDigest, KeyInputs, ProjectError, cache_key, file_digest, read_digest,
};

/// A changed file up to this size is hashed by the asking request itself.
const INLINE_BYTES: u64 = 1 << 20;
/// How long before its reading began a file must have been written for its
/// digest to be remembered: above the timestamp resolution of the file
/// systems qoob3d runs on (APFS, ext4, btrfs).
const RACY: Duration = Duration::from_secs(1);

/// Size and modification time of a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    size: u64,
    modified: SystemTime,
}

impl Stamp {
    fn of(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        Some(Self { size: meta.len(), modified: meta.modified().ok()? })
    }

    /// Unchanged since before `start` by more than [`RACY`].
    fn settled(self, start: SystemTime) -> bool {
        self.modified.checked_add(RACY).is_some_and(|t| t <= start)
    }
}

/// The modules a file imports literally, with the stamps of the file and of
/// those modules when they were scanned.
struct Imports {
    stamps: Vec<(PathBuf, Stamp)>,
    modules: Vec<PathBuf>,
}

#[derive(Default)]
struct Queue {
    files: VecDeque<PathBuf>,
    queued: HashSet<PathBuf>,
    started: bool,
}

#[derive(Default)]
pub(crate) struct Hashes {
    /// By absolute path.
    digests: Mutex<HashMap<PathBuf, (Stamp, FileDigest)>>,
    /// By path relative to the project.
    imports: Mutex<HashMap<PathBuf, Imports>>,
    queue: Mutex<Queue>,
    wake: Condvar,
    /// The last key of every part, with its inputs.
    keys: Mutex<HashMap<String, (KeyInputs, String)>>,
}

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Hashes {
    fn remembered(&self, path: &Path, stamp: Stamp) -> Option<FileDigest> {
        guard(&self.digests).get(path).filter(|(s, _)| *s == stamp).map(|(_, d)| *d)
    }

    /// The digest of `path` (absolute), hashed now if it is not remembered.
    /// A file that cannot be read hashes as empty: the key changes and the
    /// build reports the missing file.
    pub(crate) fn digest(&self, path: &Path) -> FileDigest {
        match Stamp::of(path) {
            Some(stamp) => self.remembered(path, stamp).unwrap_or_else(|| self.hash(path)),
            None => file_digest(&[]),
        }
    }

    /// The digest of `path` without waiting for a large file: `None` while
    /// it is hashed in the background.
    pub(crate) fn known(self: &Arc<Self>, path: &Path) -> Option<FileDigest> {
        let Some(stamp) = Stamp::of(path) else {
            return Some(file_digest(&[]));
        };
        if let Some(digest) = self.remembered(path, stamp) {
            return Some(digest);
        }
        if stamp.size <= INLINE_BYTES {
            return Some(self.hash(path));
        }
        self.schedule(path);
        None
    }

    fn hash(&self, path: &Path) -> FileDigest {
        let start = SystemTime::now();
        let Ok(digest) = read_digest(path) else {
            return file_digest(&[]);
        };
        // The stamp after reading: a write during the read makes it younger
        // than `start`, and the digest is not remembered.
        if let Some(stamp) = Stamp::of(path).filter(|s| s.settled(start)) {
            guard(&self.digests).insert(path.to_owned(), (stamp, digest));
        }
        digest
    }

    fn schedule(self: &Arc<Self>, path: &Path) {
        let mut queue = guard(&self.queue);
        if queue.queued.insert(path.to_owned()) {
            queue.files.push_back(path.to_owned());
        }
        if !queue.started {
            let hashes = Arc::clone(self);
            let spawned =
                std::thread::Builder::new().name("qoob3d-hash".into()).spawn(move || hashes.work());
            match spawned {
                Ok(_) => queue.started = true,
                Err(e) => tracing::warn!("starting the hash thread: {e}"),
            }
        }
        drop(queue);
        self.wake.notify_one();
    }

    /// The cache key of `inputs` ([`cache_key`]), hashed only when the
    /// inputs of `part` changed since its last key.
    pub(crate) fn key(&self, part: &str, inputs: &KeyInputs) -> String {
        if let Some((known, key)) = guard(&self.keys).get(part)
            && known == inputs
        {
            return key.clone();
        }
        let key = cache_key(inputs);
        guard(&self.keys).insert(part.to_owned(), (inputs.clone(), key.clone()));
        key
    }

    /// The files waiting for the background thread or being hashed by it.
    pub(crate) fn pending(&self) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = guard(&self.queue).queued.iter().cloned().collect();
        files.sort();
        files
    }

    /// The background thread: hashes queued files one after another, for
    /// the life of the service.
    fn work(&self) {
        let mut queue = guard(&self.queue);
        loop {
            let Some(path) = queue.files.pop_front() else {
                queue = self.wake.wait(queue).unwrap_or_else(PoisonError::into_inner);
                continue;
            };
            drop(queue);
            self.hash(&path);
            queue = guard(&self.queue);
            queue.queued.remove(&path);
        }
    }

    /// The modules `file` (relative to `dir`) imports literally,
    /// transitively ([`project::resolve_imports`]), scanned again only when
    /// the file or one of those modules changed.
    ///
    /// # Errors
    /// As [`project::resolve_imports`].
    pub(crate) fn imports(&self, dir: &Path, file: &Path) -> Result<Vec<PathBuf>, ProjectError> {
        if let Some(known) = guard(&self.imports).get(file)
            && known.stamps.iter().all(|(p, s)| Stamp::of(&dir.join(p)) == Some(*s))
        {
            return Ok(known.modules.clone());
        }
        let start = SystemTime::now();
        let modules = project::resolve_imports(dir, file)?;
        let stamps: Option<Vec<(PathBuf, Stamp)>> = std::iter::once(file)
            .chain(modules.iter().map(PathBuf::as_path))
            .map(|p| Some((p.to_owned(), Stamp::of(&dir.join(p)).filter(|s| s.settled(start))?)))
            .collect();
        if let Some(stamps) = stamps {
            guard(&self.imports)
                .insert(file.to_owned(), Imports { stamps, modules: modules.clone() });
        }
        Ok(modules)
    }
}

#[cfg(test)]
mod tests;
