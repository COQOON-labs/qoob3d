//! Live view of a script project (docs/requirements/project-app.md,
//! ADR-0053): a background thread polls the status of the project service and
//! loads the meshes of parts whose cache key changed; [`Live`] is what the
//! app shows of it. HTTP never runs on the UI thread (WA-LV-01).

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use qoob3d_core::ShapeMesh;

use crate::client::{Client, Part, PartState, ProjectDir, START_TIMEOUT};

/// Status poll interval of the live view (WA-LV-01).
pub const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// What the poller hands to the UI.
#[derive(Debug)]
pub enum Update {
    /// The status changed (or answered again after being unreachable).
    Parts(Vec<Part>),
    /// A part's mesh with a key different from the one loaded before.
    Mesh { name: String, key: String, mesh: ShapeMesh },
    /// The service does not answer (WA-LV-05).
    Unreachable,
}

/// Background thread polling the service (WA-LV-01/02/05). Dropping it
/// stops the thread after its current request.
pub struct Poller {
    updates: Receiver<Update>,
    /// Dropped with the poller; the thread sees the disconnect and ends.
    _stop: Sender<()>,
}

impl Poller {
    /// Polls `client` every `interval`.
    pub fn spawn(client: Client, interval: Duration) -> Self {
        let mut client = Some(client);
        Self::start(move || client.take(), interval, || {})
    }

    /// For the app (WA-OP-02/03): connects (starting the service if needed)
    /// and requests one build of all parts in the background, then polls
    /// every [`POLL_INTERVAL`]. `notify` runs after every update (repaint).
    ///
    /// A failed first connection is retried every interval (AG-HD-05); the
    /// retries only look for a running service, they start none.
    pub fn open(project: ProjectDir, cmd: Vec<String>, notify: impl Fn() + Send + 'static) -> Self {
        let mut first = true;
        let connect = move || {
            let client = if std::mem::take(&mut first) {
                Client::connect(&project, &cmd, START_TIMEOUT)
                    .map_err(|err| tracing::error!("project service: {err:#}"))
                    .ok()?
            } else {
                Client::probe(&project)?
            };
            if let Err(err) = client.build_all() {
                tracing::warn!("build request failed: {err:#}");
            }
            Some(client)
        };
        Self::start(connect, POLL_INTERVAL, notify)
    }

    /// `connect` is called every `interval` until it returns a client.
    fn start(
        mut connect: impl FnMut() -> Option<Client> + Send + 'static,
        interval: Duration,
        notify: impl Fn() + Send + 'static,
    ) -> Self {
        let (tx, updates) = mpsc::channel();
        let (stop, stopped) = mpsc::channel();
        std::thread::Builder::new()
            .name("qoob3d-live".into())
            .spawn(move || {
                let send = |u: Update| {
                    let sent = tx.send(u).is_ok();
                    notify();
                    sent
                };
                let mut reported = false;
                let client = loop {
                    if let Some(client) = connect() {
                        break client;
                    }
                    if !std::mem::replace(&mut reported, true) && !send(Update::Unreachable) {
                        return;
                    }
                    if stopped.recv_timeout(interval) != Err(RecvTimeoutError::Timeout) {
                        return;
                    }
                };
                poll_loop(client, interval, &stopped, send);
            })
            .expect("spawning the live poller thread");
        Self { updates, _stop: stop }
    }

    pub fn updates(&self) -> &Receiver<Update> {
        &self.updates
    }
}

/// One status request per `interval`; meshes only for parts whose key
/// differs from the key of the mesh last loaded (WA-LV-02). When the service
/// does not answer, `service.json` is read again (AG-HD-05).
fn poll_loop(
    mut client: Client,
    interval: Duration,
    stopped: &Receiver<()>,
    send: impl Fn(Update) -> bool,
) {
    let mut loaded: HashMap<String, String> = HashMap::new();
    // Last parts sent; `None` after an outage, so recovery is always reported.
    let mut last: Option<Vec<Part>> = None;
    let mut reachable = true;
    let stop_requested = || !matches!(stopped.try_recv(), Err(mpsc::TryRecvError::Empty));
    loop {
        match client.status() {
            Ok(parts) => {
                reachable = true;
                // A vanished part is forgotten, so it is fetched again on return.
                loaded.retain(|name, _| parts.iter().any(|p| p.name == *name));
                if last.as_ref() != Some(&parts) {
                    if !send(Update::Parts(parts.clone())) {
                        return;
                    }
                    last = Some(parts.clone());
                }
                for part in &parts {
                    let Some(key) = &part.key else { continue };
                    if loaded.get(&part.name) == Some(key) || stop_requested() {
                        continue;
                    }
                    match client.mesh(&part.name) {
                        Ok(m) => {
                            loaded.insert(part.name.clone(), m.key.clone());
                            let update =
                                Update::Mesh { name: part.name.clone(), key: m.key, mesh: m.mesh };
                            if !send(update) {
                                return;
                            }
                        }
                        Err(err) => tracing::warn!("loading {}: {err:#}", part.name),
                    }
                }
            }
            Err(err) => {
                client.reload();
                if reachable {
                    tracing::warn!("project service not reachable: {err:#}");
                    reachable = false;
                    last = None;
                    if !send(Update::Unreachable) {
                        return;
                    }
                }
            }
        }
        if stopped.recv_timeout(interval) != Err(RecvTimeoutError::Timeout) {
            return;
        }
    }
}

// ------------------------------------------------------------------ view model

/// A part as the app shows it.
#[derive(Debug)]
pub struct LivePart {
    pub name: String,
    pub file: String,
    pub state: PartState,
    /// Cache key from the latest status.
    pub key: Option<String>,
    /// Mesh of the last successful build; stays while a rebuild fails.
    pub mesh: Option<ShapeMesh>,
    pub visible: bool,
}

/// What the app shows of a project: parts in status order (WA-LV-02/04/05).
#[derive(Debug, Default)]
pub struct Live {
    parts: Vec<LivePart>,
    unreachable: bool,
    /// Bumped whenever the drawn geometry changes (mesh, visibility, parts).
    version: u64,
}

impl Live {
    pub fn apply(&mut self, update: Update) {
        match update {
            Update::Parts(parts) => {
                self.unreachable = false;
                let mut old: HashMap<String, LivePart> =
                    self.parts.drain(..).map(|p| (p.name.clone(), p)).collect();
                self.parts = parts
                    .into_iter()
                    .map(|p| {
                        let prev = old.remove(&p.name);
                        LivePart {
                            visible: prev.as_ref().is_none_or(|q| q.visible),
                            mesh: prev.and_then(|q| q.mesh),
                            name: p.name,
                            file: p.file,
                            state: p.state,
                            key: p.key,
                        }
                    })
                    .collect();
                if old.values().any(|p| p.mesh.is_some()) {
                    self.version += 1;
                }
            }
            Update::Mesh { name, mesh, .. } => {
                if let Some(part) = self.parts.iter_mut().find(|p| p.name == name) {
                    part.mesh = Some(mesh);
                    self.version += 1;
                }
            }
            Update::Unreachable => self.unreachable = true,
        }
    }

    pub fn parts(&self) -> &[LivePart] {
        &self.parts
    }

    /// False while the service does not answer.
    pub fn reachable(&self) -> bool {
        !self.unreachable
    }

    pub fn set_visible(&mut self, name: &str, visible: bool) {
        if let Some(part) = self.parts.iter_mut().find(|p| p.name == name)
            && part.visible != visible
        {
            part.visible = visible;
            self.version += 1;
        }
    }

    /// Changes whenever [`Live::scene`] would change.
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn has_meshes(&self) -> bool {
        self.parts.iter().any(|p| p.mesh.is_some())
    }

    /// The visible meshes merged for the renderer and picking.
    pub fn scene(&self) -> ShapeMesh {
        ShapeMesh::merge(self.parts.iter().filter(|p| p.visible).filter_map(|p| p.mesh.as_ref()))
    }
}
