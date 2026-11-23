//! cxx bridge to QOOB3D's own OCCT wrappers in `cpp/` (ADR-0004).
#![expect(
    unsafe_code,
    unreachable_pub,
    reason = "cxx generates the unsafe FFI glue and its pub items; the C++ side converts \
              every OCCT exception into an `Err` (ADR-0004), so calls are safe from Rust"
)]

use cxx::UniquePtr;

use crate::KernelError;

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

    /// Sketch curves in the local plane (ADR-0004): lines `x0 y0 x1 y1`,
    /// circles `cx cy r`, arcs `cx cy r a0 a1` (CCW, radians).
    struct Qoob3dCurves {
        lines: Vec<f64>,
        line_ids: Vec<u32>,
        circles: Vec<f64>,
        circle_ids: Vec<u32>,
        arcs: Vec<f64>,
        arc_ids: Vec<u32>,
    }

    /// Edges or faces of a shape (script selectors, ADR-0053). Element `i`
    /// has map index `i + 1`, kind (edges: 0 line, 1 circle, 2 other,
    /// 3 seam or degenerate; faces: 0 plane, 1 cylinder, 2 other), tight
    /// bounds `6 i..6 i + 6` (min, max) and direction `3 i..3 i + 3` (line
    /// direction, circle axis, outward plane normal, cylinder axis).
    struct Qoob3dElements {
        kinds: Vec<u32>,
        bounds: Vec<f64>,
        directions: Vec<f64>,
    }

    unsafe extern "C++" {
        include!("qoob3d-kernel/cpp/qoob3d_occt.hxx");

        type TopoDS_Shape = opencascade_sys::topo_ds::TopoDS_Shape;

        fn qoob3d_tessellate(
            shape: &TopoDS_Shape,
            linear_deflection: f64,
            angular_deflection: f64,
        ) -> Result<Qoob3dMesh>;

        fn qoob3d_profile_faces(
            curves: &Qoob3dCurves,
            keys: &[u32],
            key_starts: &[u32],
            frame: &[f64],
        ) -> Result<UniquePtr<TopoDS_Shape>>;
        fn qoob3d_outline_face(curves: &Qoob3dCurves) -> Result<UniquePtr<TopoDS_Shape>>;
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
        fn qoob3d_profile_boolean(
            a: &TopoDS_Shape,
            b: &TopoDS_Shape,
            op: u32,
        ) -> Result<UniquePtr<TopoDS_Shape>>;
        fn qoob3d_distance(a: &TopoDS_Shape, b: &TopoDS_Shape, max: f64) -> Result<f64>;
        fn qoob3d_volume(shape: &TopoDS_Shape) -> Result<f64>;
        fn qoob3d_is_valid(shape: &TopoDS_Shape) -> Result<bool>;
        fn qoob3d_solid_count(shape: &TopoDS_Shape) -> Result<u32>;

        fn qoob3d_primitive(kind: u32, dims: &[f64]) -> Result<UniquePtr<TopoDS_Shape>>;
        fn qoob3d_transform(
            shape: &TopoDS_Shape,
            kind: u32,
            params: &[f64],
        ) -> Result<UniquePtr<TopoDS_Shape>>;
        fn qoob3d_compound() -> Result<UniquePtr<TopoDS_Shape>>;
        fn qoob3d_compound_add(compound: Pin<&mut TopoDS_Shape>, part: &TopoDS_Shape)
        -> Result<()>;
        fn qoob3d_elements(shape: &TopoDS_Shape, faces: bool) -> Result<Qoob3dElements>;
        fn qoob3d_fillet(
            shape: &TopoDS_Shape,
            edges: &[u32],
            size: f64,
            chamfer: bool,
        ) -> Result<UniquePtr<TopoDS_Shape>>;
        fn qoob3d_shell(
            shape: &TopoDS_Shape,
            open_faces: &[u32],
            thickness: f64,
        ) -> Result<UniquePtr<TopoDS_Shape>>;
        fn qoob3d_area(shape: &TopoDS_Shape) -> Result<f64>;
        fn qoob3d_center(shape: &TopoDS_Shape) -> Result<Vec<f64>>;
        fn qoob3d_bbox(shape: &TopoDS_Shape) -> Result<Vec<f64>>;
        fn qoob3d_ray(shape: &TopoDS_Shape, origin: &[f64], direction: &[f64]) -> Result<Vec<f64>>;
        fn qoob3d_pipe(
            points: &[f64],
            radius: f64,
            bend_radius: f64,
        ) -> Result<UniquePtr<TopoDS_Shape>>;
        fn qoob3d_profile_offset(
            faces: &TopoDS_Shape,
            distance: f64,
        ) -> Result<UniquePtr<TopoDS_Shape>>;
        fn qoob3d_element_sizes(
            shape: &TopoDS_Shape,
            faces: bool,
            indices: &[u32],
        ) -> Result<Vec<f64>>;
        fn qoob3d_element_distances(
            shape: &TopoDS_Shape,
            faces: bool,
            indices: &[u32],
            point: &[f64],
        ) -> Result<Vec<f64>>;
        fn qoob3d_section(shape: &TopoDS_Shape, frame: &[f64]) -> Result<UniquePtr<TopoDS_Shape>>;
        fn qoob3d_bed_area(shape: &TopoDS_Shape) -> Result<f64>;
        fn qoob3d_overhang_area(shape: &TopoDS_Shape, max_angle: f64) -> Result<f64>;
        fn qoob3d_write_step(shape: &TopoDS_Shape, path: &str) -> Result<()>;
        fn qoob3d_read_step(path: &str) -> Result<UniquePtr<TopoDS_Shape>>;
        fn qoob3d_write_brep(shape: &TopoDS_Shape, path: &str) -> Result<()>;
        fn qoob3d_read_brep(path: &str) -> Result<UniquePtr<TopoDS_Shape>>;
    }
}

/// An OCCT shape that is never null.
pub(crate) struct Shape(UniquePtr<ffi::TopoDS_Shape>);

impl Shape {
    pub(crate) fn get(&self) -> &ffi::TopoDS_Shape {
        self.0.as_ref().expect("shapes are never null")
    }
}

/// An OCCT exception, caught in our C++ wrappers, as a [`KernelError`].
pub(crate) fn occ<T>(r: Result<T, cxx::Exception>) -> Result<T, KernelError> {
    r.map_err(|e| KernelError::Occt(e.what().to_string()))
}

/// The shape a wrapper returned; a null shape is an error.
pub(crate) fn shape(
    r: Result<UniquePtr<ffi::TopoDS_Shape>, cxx::Exception>,
) -> Result<Shape, KernelError> {
    let s = occ(r)?;
    if s.is_null() {
        return Err(KernelError::Occt("no shape".into()));
    }
    Ok(Shape(s))
}
