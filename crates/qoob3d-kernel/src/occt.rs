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

    /// Profiles of a sketch (ADR-0028). Region `r` has vertices
    /// `vertex_starts[r]..vertex_starts[r + 1]` (x, y pairs in `positions`),
    /// triangle indices `index_starts[r]..` (relative to its first vertex),
    /// key curve ids `key_starts[r]..` and area `areas[r]` (mm²).
    struct Qoob3dRegions {
        positions: Vec<f64>,
        indices: Vec<u32>,
        vertex_starts: Vec<u32>,
        index_starts: Vec<u32>,
        key_ids: Vec<u32>,
        key_starts: Vec<u32>,
        areas: Vec<f64>,
    }

    /// Sketch curves in the local plane (ADR-0028): lines `x0 y0 x1 y1`,
    /// circles `cx cy r`, arcs `cx cy r a0 a1` (CCW, radians).
    struct Qoob3dCurves {
        lines: Vec<f64>,
        line_ids: Vec<u32>,
        circles: Vec<f64>,
        circle_ids: Vec<u32>,
        arcs: Vec<f64>,
        arc_ids: Vec<u32>,
    }

    unsafe extern "C++" {
        include!("qoob3d-kernel/cpp/qoob3d_occt.hxx");

        type TopoDS_Shape = opencascade_sys::topo_ds::TopoDS_Shape;

        fn qoob3d_tessellate(
            shape: &TopoDS_Shape,
            linear_deflection: f64,
            angular_deflection: f64,
        ) -> Result<Qoob3dMesh>;

        fn qoob3d_sketch_regions(curves: &Qoob3dCurves, deflection: f64) -> Result<Qoob3dRegions>;

        fn qoob3d_profile_faces(
            curves: &Qoob3dCurves,
            keys: &[u32],
            key_starts: &[u32],
            frame: &[f64],
        ) -> Result<UniquePtr<TopoDS_Shape>>;
        fn qoob3d_extrude(
            faces: &TopoDS_Shape,
            vector: &[f64],
            symmetric: bool,
        ) -> Result<UniquePtr<TopoDS_Shape>>;
        fn qoob3d_revolve(
            faces: &TopoDS_Shape,
            axis: &[f64],
            angle: f64,
        ) -> Result<UniquePtr<TopoDS_Shape>>;
        fn qoob3d_loft(
            faces: &TopoDS_Shape,
            start_point: &[f64],
            end_point: &[f64],
        ) -> Result<UniquePtr<TopoDS_Shape>>;
        fn qoob3d_boolean(
            a: &TopoDS_Shape,
            b: &TopoDS_Shape,
            op: u32,
        ) -> Result<UniquePtr<TopoDS_Shape>>;
        fn qoob3d_distance(a: &TopoDS_Shape, b: &TopoDS_Shape) -> Result<f64>;
        fn qoob3d_volume(shape: &TopoDS_Shape) -> Result<f64>;
        fn qoob3d_is_valid(shape: &TopoDS_Shape) -> Result<bool>;
        fn qoob3d_solid_count(shape: &TopoDS_Shape) -> Result<u32>;
        fn qoob3d_solid(shape: &TopoDS_Shape, index: u32) -> Result<UniquePtr<TopoDS_Shape>>;
        fn qoob3d_append(a: &TopoDS_Shape, b: &TopoDS_Shape) -> Result<UniquePtr<TopoDS_Shape>>;
        fn qoob3d_outline_prism(
            points: &[f64],
            starts: &[u32],
            kinds: &[u32],
            holes: &[f64],
            normal: &[f64],
            vector: &[f64],
        ) -> Result<UniquePtr<TopoDS_Shape>>;
        fn qoob3d_sweep_circle(
            path: &[f64],
            radius: f64,
            binormal: &[f64],
        ) -> Result<UniquePtr<TopoDS_Shape>>;
    }
}
