//! The kernel worker thread and its message protocol (ADR-0005).

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use qoob3d_core::ShapeMesh;
use qoob3d_core::document::Document;

use crate::model::{ExportBody, Model, RebuildResult};
use crate::tessellate::{demo_part, tessellate};
use crate::{KernelError, VIEW_TESSELLATION_TOLERANCE_MM};

/// Monotonically increasing id of a submitted request. Responses carry the id
/// of the request they answer so callers can discard stale results.
pub type RequestId = u64;

/// Work the kernel can be asked to do.
#[derive(Debug, Clone)]
pub enum Request {
    /// Demo geometry for tests and `--demo` (a block with a hole).
    DemoPart,
    /// Recompute everything derived from the document. Only the newest
    /// queued rebuild is processed (ADR-0005).
    Rebuild { document: Arc<Document> },
    /// Fine meshes of all bodies for STL/3MF (tolerance in mm).
    Export { document: Arc<Document>, tolerance: f64 },
}

/// Results sent back from the worker thread.
#[derive(Debug, Clone)]
pub enum Response {
    Mesh { id: RequestId, result: Result<ShapeMesh, KernelError> },
    Rebuilt { id: RequestId, result: RebuildResult },
    Exported { id: RequestId, result: Result<Vec<ExportBody>, KernelError> },
}

/// Handle to the kernel worker thread. Dropping it shuts the worker down.
pub struct Kernel {
    requests: Option<Sender<(RequestId, Request)>>,
    responses: Receiver<Response>,
    next_id: RequestId,
    thread: Option<JoinHandle<()>>,
}

impl Kernel {
    /// Starts the worker. `notify` is called (on the worker thread) after each
    /// response is sent, e.g. to wake up the UI event loop.
    pub fn spawn(notify: impl Fn() + Send + 'static) -> Self {
        let (req_tx, req_rx) = mpsc::channel::<(RequestId, Request)>();
        let (resp_tx, resp_rx) = mpsc::channel();

        let thread = std::thread::Builder::new()
            .name("qoob3d-kernel".into())
            .spawn(move || {
                tracing::debug!("kernel worker started");
                let mut model = Model::default();
                // Ends when the request sender (the `Kernel` handle) is dropped.
                while let Ok(first) = req_rx.recv() {
                    // Skip rebuilds that a newer queued rebuild supersedes.
                    let mut queue = vec![first];
                    queue.extend(req_rx.try_iter());
                    let newest_rebuild =
                        queue.iter().rposition(|(_, r)| matches!(r, Request::Rebuild { .. }));
                    for (index, (id, request)) in queue.into_iter().enumerate() {
                        let superseded = matches!(request, Request::Rebuild { .. })
                            && Some(index) != newest_rebuild;
                        if superseded {
                            continue;
                        }
                        let response = handle(&mut model, id, request);
                        if resp_tx.send(response).is_err() {
                            return;
                        }
                        notify();
                    }
                }
                tracing::debug!("kernel worker stopped");
            })
            .expect("failed to spawn kernel thread");

        Self { requests: Some(req_tx), responses: resp_rx, next_id: 0, thread: Some(thread) }
    }

    /// Queues a request and returns its id.
    pub fn submit(&mut self, request: Request) -> Result<RequestId, KernelError> {
        self.next_id += 1;
        let id = self.next_id;
        self.requests
            .as_ref()
            .ok_or(KernelError::WorkerGone)?
            .send((id, request))
            .map_err(|_| KernelError::WorkerGone)?;
        Ok(id)
    }

    /// Returns the next finished response without blocking.
    pub fn try_recv(&self) -> Option<Response> {
        self.responses.try_recv().ok()
    }

    /// Blocks up to `timeout` for the next response. For CLI and tests only –
    /// the UI must never block on the kernel (ADR-0005).
    pub fn recv_timeout(&self, timeout: std::time::Duration) -> Option<Response> {
        self.responses.recv_timeout(timeout).ok()
    }

    /// Rebuilds and waits for the result. CLI, tests and model checks only.
    pub fn rebuild_blocking(
        &mut self,
        document: Arc<Document>,
        timeout: std::time::Duration,
    ) -> Result<RebuildResult, KernelError> {
        let want = self.submit(Request::Rebuild { document })?;
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match self.recv_timeout(left) {
                Some(Response::Rebuilt { id, result }) if id == want => return Ok(result),
                Some(_) => {}
                None => return Err(KernelError::Model("rebuild timed out".into())),
            }
        }
    }

    /// Export meshes, waiting for the result. CLI and tests only.
    pub fn export_blocking(
        &mut self,
        document: Arc<Document>,
        tolerance: f64,
        timeout: std::time::Duration,
    ) -> Result<Vec<ExportBody>, KernelError> {
        let want = self.submit(Request::Export { document, tolerance })?;
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match self.recv_timeout(left) {
                Some(Response::Exported { id, result }) if id == want => return result,
                Some(_) => {}
                None => return Err(KernelError::Model("export timed out".into())),
            }
        }
    }
}

impl Drop for Kernel {
    fn drop(&mut self) {
        self.requests.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn handle(model: &mut Model, id: RequestId, request: Request) -> Response {
    match request {
        Request::DemoPart => {
            Response::Mesh { id, result: tessellate(&demo_part(), VIEW_TESSELLATION_TOLERANCE_MM) }
        }
        Request::Rebuild { document } => Response::Rebuilt { id, result: model.rebuild(&document) },
        Request::Export { document, tolerance } => {
            Response::Exported { id, result: model.export(&document, tolerance) }
        }
    }
}

#[cfg(test)]
mod tests;
