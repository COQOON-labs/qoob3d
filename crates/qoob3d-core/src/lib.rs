//! QOOB3D core: the plain data types shared between kernel, renderer, service
//! and UI: meshes, exports, printers, script projects and their tasks. This
//! crate must never depend on OpenCascade or a GPU API, so it stays fast to
//! compile and easy to unit-test.

pub mod export;
pub mod fs;
pub mod mesh;
pub mod printer;
pub mod project;
pub mod qmesh;
pub mod tasks;

pub use mesh::{Aabb, MeshReport, Polylines, RayHit, ShapeMesh, TriMesh};
pub use printer::{BedGeometry, PRINTERS, PrinterProfile};
