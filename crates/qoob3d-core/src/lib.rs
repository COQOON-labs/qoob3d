//! QOOB3D core: the document model and the plain data types shared between
//! kernel, renderer and UI. This crate must never depend on OpenCascade or a
//! GPU API, so it stays fast to compile and easy to unit-test.

pub mod mesh;

pub use mesh::{Aabb, MeshReport, TriMesh};
