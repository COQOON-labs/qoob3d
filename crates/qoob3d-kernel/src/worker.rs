//! The kernel worker thread and its message protocol (ADR-0005).

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use qoob3d_core::ShapeMesh;

use crate::tessellate::{demo_part, tessellate};
use crate::{KernelError, VIEW_TESSELLATION_TOLERANCE_MM};

/// Monotonically increasing id of a submitted request. Responses carry the id
/// of the request they answer so callers can discard stale results.
pub type RequestId = u64;

/// Work the kernel can be asked to do.
#[derive(Debug, Clone)]
pub enum Request {
    /// Placeholder until the document model exists (M3): build the demo part
    /// and tessellate it.
    DemoPart,
}

/// Results sent back from the worker thread.
#[derive(Debug, Clone)]
pub enum Response {
    Mesh { id: RequestId, result: Result<ShapeMesh, KernelError> },
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
                // Ends when the request sender (the `Kernel` handle) is dropped.
                for (id, request) in req_rx {
                    let response = handle(id, request);
                    if resp_tx.send(response).is_err() {
                        break;
                    }
                    notify();
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
}

impl Drop for Kernel {
    fn drop(&mut self) {
        self.requests.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn handle(id: RequestId, request: Request) -> Response {
    match request {
        Request::DemoPart => {
            Response::Mesh { id, result: tessellate(&demo_part(), VIEW_TESSELLATION_TOLERANCE_MM) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_answers_demo_cube_request() {
        let (tx, rx) = mpsc::channel();
        let mut kernel = Kernel::spawn(move || {
            let _ = tx.send(());
        });
        let id = kernel.submit(Request::DemoPart).unwrap();
        rx.recv_timeout(std::time::Duration::from_secs(30)).expect("no notification");
        let Some(Response::Mesh { id: got, result }) = kernel.try_recv() else {
            panic!("no response");
        };
        assert_eq!(got, id);
        assert!(result.unwrap().faces.analyze().is_watertight());
    }
}
