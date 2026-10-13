//! cxx bridge to QOOB3D's own OCCT wrappers in `cpp/` (ADR-0014).

// cxx generates the unsafe FFI glue; the C++ side converts every OCCT
// exception into an `Err` (ADR-0004), so calls are safe to use from Rust.
#[allow(unsafe_code)]
#[cxx::bridge]
pub(crate) mod ffi {
    /// Flat tessellation result. Triangles: 3 floats per vertex, 3 indices
    /// and one face id per triangle. Edges: polyline `i` spans points
    /// `edge_starts[i]..edge_starts[i + 1]` (the last entry is a sentinel).
    struct Qoob3dMesh {
        positions: Vec<f32>,
        normals: Vec<f32>,
        indices: Vec<u32>,
        face_ids: Vec<u32>,
        edge_points: Vec<f32>,
        edge_starts: Vec<u32>,
    }

    unsafe extern "C++" {
        include!("qoob3d-kernel/cpp/qoob3d_occt.hxx");

        type TopoDS_Shape = opencascade_sys::topo_ds::TopoDS_Shape;

        fn qoob3d_tessellate(
            shape: &TopoDS_Shape,
            linear_deflection: f64,
            angular_deflection: f64,
        ) -> Result<Qoob3dMesh>;
    }
}
