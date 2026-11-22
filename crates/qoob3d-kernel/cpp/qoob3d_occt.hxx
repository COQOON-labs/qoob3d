// QOOB3D's own OCCT wrappers (ADR-0014).
//
// Rules for everything in this file:
// - every function catches Standard_Failure (which does NOT derive from
//   std::exception) and rethrows it as std::runtime_error, so cxx turns it
//   into a Rust `Err` instead of aborting the process (ADR-0004);
// - results are returned as flat buffers to keep the FFI surface small.
#pragma once

#include <memory>

#include <TopoDS_Shape.hxx>

#include "rust/cxx.h"

struct Qoob3dMesh;
struct Qoob3dRegions;

Qoob3dMesh qoob3d_tessellate(const TopoDS_Shape &shape, double linear_deflection,
                             double angular_deflection);

// Sketch curves for region computations (ADR-0028), all in the local plane
// z = 0: lines (x0 y0 x1 y1), circles (cx cy r), arcs (cx cy r a0 a1, CCW).
struct Qoob3dCurves;

// Profiles of a sketch in its local plane z = 0 (ADR-0028).
Qoob3dRegions qoob3d_sketch_regions(const Qoob3dCurves &curves, double deflection);

// Solid modelling (ADR-0031). Shapes are owned by the kernel worker thread.
// Faces of the given profiles (keys are concatenated, `key_starts` has a
// sentinel), placed into the plane frame (origin u v n, 12 values).
std::unique_ptr<TopoDS_Shape> qoob3d_profile_faces(const Qoob3dCurves &curves,
                                                   rust::Slice<const uint32_t> keys,
                                                   rust::Slice<const uint32_t> key_starts,
                                                   rust::Slice<const double> frame);
// The face inside curves that form one closed outline without
// self-intersection, the first curve running counter-clockwise (neither is
// checked), in the plane z = 0. No General Fuse.
std::unique_ptr<TopoDS_Shape> qoob3d_outline_face(const Qoob3dCurves &curves);
std::unique_ptr<TopoDS_Shape> qoob3d_extrude(const TopoDS_Shape &faces,
                                             rust::Slice<const double> vector,
                                             bool symmetric);
std::unique_ptr<TopoDS_Shape> qoob3d_revolve(const TopoDS_Shape &faces,
                                             rust::Slice<const double> axis, double angle);
// Ruled loft through the outer wires of `faces` (in order); optional tip
// points (empty slice or x y z) before and after.
std::unique_ptr<TopoDS_Shape> qoob3d_loft(const TopoDS_Shape &faces,
                                          rust::Slice<const double> start_point,
                                          rust::Slice<const double> end_point);
// op: 0 fuse, 1 cut (a − b), 2 common. Compounds count as lists of
// arguments, so a batch of shapes is one operation.
std::unique_ptr<TopoDS_Shape> qoob3d_boolean(const TopoDS_Shape &a, const TopoDS_Shape &b,
                                             uint32_t op);
// The same on planar faces in the plane z = 0 (profile algebra, AG-PF-02).
// Result: a compound of faces; an empty result is an error.
std::unique_ptr<TopoDS_Shape> qoob3d_profile_boolean(const TopoDS_Shape &a, const TopoDS_Shape &b,
                                                     uint32_t op);
// Distance between two shapes (0 if they touch or overlap). Exact up to
// `max`; beyond it a lower bound (the distance of the bounding boxes).
double qoob3d_distance(const TopoDS_Shape &a, const TopoDS_Shape &b, double max);
double qoob3d_volume(const TopoDS_Shape &shape);
bool qoob3d_is_valid(const TopoDS_Shape &shape);
// The solids of a shape (compounds are flattened).
uint32_t qoob3d_solid_count(const TopoDS_Shape &shape);
std::unique_ptr<TopoDS_Shape> qoob3d_solid(const TopoDS_Shape &shape, uint32_t index);
// Compound of the children of `a` followed by those of `b` (order kept;
// a null `a` counts as empty).
std::unique_ptr<TopoDS_Shape> qoob3d_append(const TopoDS_Shape &a, const TopoDS_Shape &b);
// Planar outline in 3D (closed): pieces `starts[i]..starts[i+1]` of
// `points` (x y z), kind 0 = straight segments, 1 = interpolating spline.
// Optional holes: circles (cx cy cz r) in the same plane with normal `n`.
// The face is extruded by `vector`.
std::unique_ptr<TopoDS_Shape> qoob3d_outline_prism(rust::Slice<const double> points,
                                                   rust::Slice<const uint32_t> starts,
                                                   rust::Slice<const uint32_t> kinds,
                                                   rust::Slice<const double> holes,
                                                   rust::Slice<const double> normal,
                                                   rust::Slice<const double> vector);
// Round wire of `radius` swept along the spline through `path` (x y z).
// `binormal`: empty = corrected Frenet, else a fixed binormal direction.
std::unique_ptr<TopoDS_Shape> qoob3d_sweep_circle(rust::Slice<const double> path, double radius,
                                                  rust::Slice<const double> binormal);

// ---- Script kernel (ADR-0053). Placement and validation live in Rust.

// kind 0 box (corner x y z, size dx dy dz), 1 cylinder (r h), 2 sphere (r),
// 3 cone (r1 r2 h), 4 torus (R r); axis Z through the origin.
std::unique_ptr<TopoDS_Shape> qoob3d_primitive(uint32_t kind, rust::Slice<const double> dims);
// kind 0 translate (dx dy dz), 1 rotate (axis x y z, angle rad), 2 mirror at
// the plane through the origin (normal x y z), 3 scale about the origin (f),
// 4 from the XY plane into a plane frame (origin u v n, 12 values).
std::unique_ptr<TopoDS_Shape> qoob3d_transform(const TopoDS_Shape &shape, uint32_t kind,
                                               rust::Slice<const double> params);
// An empty compound; `qoob3d_compound_add` appends a shape (compounds flattened).
std::unique_ptr<TopoDS_Shape> qoob3d_compound();
void qoob3d_compound_add(TopoDS_Shape &compound, const TopoDS_Shape &part);

// Edges (or faces) of a shape in the order of TopExp::MapShapes (index i + 1).
struct Qoob3dElements;
Qoob3dElements qoob3d_elements(const TopoDS_Shape &shape, bool faces);
// Fillet (or symmetric chamfer) of the edges with map indices `edges`.
std::unique_ptr<TopoDS_Shape> qoob3d_fillet(const TopoDS_Shape &shape,
                                            rust::Slice<const uint32_t> edges, double size,
                                            bool chamfer);
// Hollow solid, wall inward, the faces with map indices `open_faces` removed.
std::unique_ptr<TopoDS_Shape> qoob3d_shell(const TopoDS_Shape &shape,
                                           rust::Slice<const uint32_t> open_faces,
                                           double thickness);
double qoob3d_area(const TopoDS_Shape &shape);
rust::Vec<double> qoob3d_center(const TopoDS_Shape &shape);
// Tight bounding box xmin ymin zmin xmax ymax zmax.
rust::Vec<double> qoob3d_bbox(const TopoDS_Shape &shape);
// Points (x y z) where the half-line from `origin` along `direction` crosses
// faces, sorted by distance.
rust::Vec<double> qoob3d_ray(const TopoDS_Shape &shape, rust::Slice<const double> origin,
                             rust::Slice<const double> direction);
// Circle of `radius` swept along the polyline `points` (x y z), corners
// rounded with `bend_radius`.
std::unique_ptr<TopoDS_Shape> qoob3d_pipe(rust::Slice<const double> points, double radius,
                                          double bend_radius);
void qoob3d_write_step(const TopoDS_Shape &shape, rust::Str path);
std::unique_ptr<TopoDS_Shape> qoob3d_read_step(rust::Str path);
// OCCT's native BREP text format (cache entries, AG-PJ-04).
void qoob3d_write_brep(const TopoDS_Shape &shape, rust::Str path);
std::unique_ptr<TopoDS_Shape> qoob3d_read_brep(rust::Str path);
