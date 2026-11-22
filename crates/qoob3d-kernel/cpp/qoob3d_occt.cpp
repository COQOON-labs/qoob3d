#include "qoob3d_occt.hxx"

#include <stdexcept>
#include <string>

#include <algorithm>
#include <mutex>
#include <utility>
#include <vector>

#include <cmath>

#include <BRepAdaptor_Surface.hxx>
#include <BRepBuilderAPI_Copy.hxx>
#include <BRepFilletAPI_MakeChamfer.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <BRepOffsetAPI_MakeOffset.hxx>
#include <BRepOffsetAPI_MakeThickSolid.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCone.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakeSphere.hxx>
#include <BRepPrimAPI_MakeTorus.hxx>
#include <ChFi2d_FilletAPI.hxx>
#include <IMeshTools_Parameters.hxx>
#include <IntCurvesFace_ShapeIntersector.hxx>
#include <STEPControl_Reader.hxx>
#include <STEPControl_Writer.hxx>
#include <gp_Lin.hxx>

#include <BOPAlgo_Builder.hxx>
#include <BRepAlgoAPI_Common.hxx>
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepBndLib.hxx>
#include <BRepBuilderAPI_MakeVertex.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepExtrema_DistShapeShape.hxx>
#include <BRepOffsetAPI_MakePipeShell.hxx>
#include <BRepOffsetAPI_ThruSections.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <BRepPrimAPI_MakeRevol.hxx>
#include <BRepTools.hxx>
#include <BRep_Builder.hxx>
#include <Bnd_Box.hxx>
#include <GeomAPI_Interpolate.hxx>
#include <GeomLib.hxx>
#include <Geom_BSplineCurve.hxx>
#include <ShapeFix_Face.hxx>
#include <ShapeUpgrade_UnifySameDomain.hxx>
#include <TColgp_HArray1OfPnt.hxx>
#include <TopoDS_Compound.hxx>
#include <TopoDS_Iterator.hxx>
#include <TopoDS_Wire.hxx>
#include <gp_Ax1.hxx>
#include <gp_Ax3.hxx>
#include <gp_Trsf.hxx>
#include <BRepAdaptor_Curve.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepGProp.hxx>
#include <BRepLib.hxx>
#include <ElSLib.hxx>
#include <IntTools_FClass2d.hxx>
#include <GProp_GProps.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_DataMapOfShapeInteger.hxx>
#include <gp_Ax2.hxx>
#include <gp_Circ.hxx>
#include <gp_Pln.hxx>
#include <BRepLib_ToolTriangulatedShape.hxx>
#include <BRepMesh_IncrementalMesh.hxx>
#include <BRep_Tool.hxx>
#include <GCPnts_TangentialDeflection.hxx>
#include <Poly_PolygonOnTriangulation.hxx>
#include <Poly_Triangulation.hxx>
#include <Precision.hxx>
#include <Standard_Failure.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>

#include "qoob3d-kernel/src/occt.rs.h"

namespace {

void push3(rust::Vec<float> &v, double x, double y, double z) {
  v.push_back(static_cast<float>(x));
  v.push_back(static_cast<float>(y));
  v.push_back(static_cast<float>(z));
}

// Converts OCCT exceptions into std::runtime_error for cxx.
template <typename F> auto guarded(const char *what, F &&f) -> decltype(f()) {
  try {
    return f();
  } catch (const Standard_Failure &e) {
    std::string msg = std::string(what) + ": " + e.DynamicType()->Name();
    if (e.GetMessageString() && *e.GetMessageString()) {
      msg += ": ";
      msg += e.GetMessageString();
    }
    throw std::runtime_error(msg);
  }
}

void tessellate_faces(const TopoDS_Shape &shape, Qoob3dMesh &out) {
  TopTools_IndexedMapOfShape faces;
  TopExp::MapShapes(shape, TopAbs_FACE, faces);

  for (int fi = 1; fi <= faces.Extent(); ++fi) {
    const TopoDS_Face &face = TopoDS::Face(faces(fi));
    TopLoc_Location loc;
    const Handle(Poly_Triangulation) &tri = BRep_Tool::Triangulation(face, loc);
    if (tri.IsNull()) {
      throw std::runtime_error("face " + std::to_string(fi) + " has no triangulation");
    }
    if (!tri->HasNormals()) {
      BRepLib_ToolTriangulatedShape::ComputeNormals(face, tri);
    }
    const gp_Trsf trsf = loc.Transformation();
    // Triangulation and normals follow the surface; a reversed face points
    // the other way, so flip winding and normals (the opencascade-rs Mesher
    // got this wrong, see ADR-0004).
    const bool reversed = face.Orientation() == TopAbs_REVERSED;
    const uint32_t base = static_cast<uint32_t>(out.positions.size() / 3);

    for (int i = 1; i <= tri->NbNodes(); ++i) {
      const gp_Pnt p = tri->Node(i).Transformed(trsf);
      push3(out.positions, p.X(), p.Y(), p.Z());
      gp_Dir n = tri->Normal(i);
      n.Transform(trsf);
      if (reversed) {
        n.Reverse();
      }
      push3(out.normals, n.X(), n.Y(), n.Z());
    }
    for (int t = 1; t <= tri->NbTriangles(); ++t) {
      int a, b, c;
      tri->Triangle(t).Get(a, b, c);
      // Collapsed triangles (e.g. at the poles of a revolved sphere, where
      // an edge is degenerate) add nothing but confuse slicers.
      const gp_Pnt pa = tri->Node(a), pb = tri->Node(b), pc = tri->Node(c);
      const double tiny = 1e-14;
      if (pa.SquareDistance(pb) < tiny || pb.SquareDistance(pc) < tiny ||
          pc.SquareDistance(pa) < tiny) {
        continue;
      }
      if (reversed) {
        std::swap(b, c);
      }
      out.indices.push_back(base + a - 1);
      out.indices.push_back(base + b - 1);
      out.indices.push_back(base + c - 1);
      out.face_ids.push_back(static_cast<uint32_t>(fi - 1));
    }
  }
}

void extract_edges(const TopoDS_Shape &shape, double lin, double ang, Qoob3dMesh &out) {
  TopTools_IndexedDataMapOfShapeListOfShape edge_faces;
  TopExp::MapShapesAndAncestors(shape, TopAbs_EDGE, TopAbs_FACE, edge_faces);

  for (int ei = 1; ei <= edge_faces.Extent(); ++ei) {
    const TopoDS_Edge &edge = TopoDS::Edge(edge_faces.FindKey(ei));
    if (BRep_Tool::Degenerated(edge)) {
      continue; // e.g. the apex of a cone
    }
    const TopTools_ListOfShape &adjacent = edge_faces(ei);
    if (!adjacent.IsEmpty() && BRep_Tool::IsClosed(edge, TopoDS::Face(adjacent.First()))) {
      continue; // seam of a periodic surface: not a visible edge
    }

    const uint32_t start = static_cast<uint32_t>(out.edge_points.size() / 3);
    bool done = false;
    // Prefer the edge polygon on the face triangulation: its points are
    // exactly the mesh vertices, so edges sit precisely on the faces.
    if (!adjacent.IsEmpty()) {
      const TopoDS_Face &face = TopoDS::Face(adjacent.First());
      TopLoc_Location loc;
      const Handle(Poly_Triangulation) &tri = BRep_Tool::Triangulation(face, loc);
      if (!tri.IsNull()) {
        const Handle(Poly_PolygonOnTriangulation) &poly =
            BRep_Tool::PolygonOnTriangulation(edge, tri, loc);
        if (!poly.IsNull()) {
          const gp_Trsf trsf = loc.Transformation();
          for (int i = 1; i <= poly->NbNodes(); ++i) {
            const gp_Pnt p = tri->Node(poly->Node(i)).Transformed(trsf);
            push3(out.edge_points, p.X(), p.Y(), p.Z());
          }
          done = true;
        }
      }
    }
    if (!done) {
      // Free edges (e.g. wires without faces): discretise the curve itself.
      BRepAdaptor_Curve curve(edge);
      GCPnts_TangentialDeflection points(curve, ang, lin);
      for (int i = 1; i <= points.NbPoints(); ++i) {
        const gp_Pnt p = points.Value(i);
        push3(out.edge_points, p.X(), p.Y(), p.Z());
      }
    }
    if (out.edge_points.size() / 3 - start >= 2) {
      out.edge_starts.push_back(start);
    } else {
      out.edge_points.truncate(start * 3);
    }
  }
  out.edge_starts.push_back(static_cast<uint32_t>(out.edge_points.size() / 3));
}

} // namespace

Qoob3dMesh qoob3d_tessellate(const TopoDS_Shape &shape, double linear_deflection,
                             double angular_deflection) {
  return guarded("tessellate", [&] {
    BRepMesh_IncrementalMesh mesher(shape, linear_deflection, Standard_False,
                                    angular_deflection, Standard_True);
    if (!mesher.IsDone()) {
      throw std::runtime_error("BRepMesh_IncrementalMesh failed");
    }
    Qoob3dMesh out;
    tessellate_faces(shape, out);
    extract_edges(shape, linear_deflection, angular_deflection, out);
    return out;
  });
}

// ---------------------------------------------------------------- sketch profiles

namespace {

// Binds every piece an input shape was split into (or the shape itself if it
// was not split) to `id`.
void bind_pieces(BOPAlgo_Builder &builder, const TopoDS_Shape &shape, int id,
                 TopTools_DataMapOfShapeInteger &map) {
  const TopTools_ListOfShape &pieces = builder.Modified(shape);
  if (pieces.IsEmpty()) {
    map.Bind(shape, id);
    return;
  }
  for (TopTools_ListIteratorOfListOfShape it(pieces); it.More(); it.Next()) {
    map.Bind(it.Value(), id);
  }
}

struct Region {
  TopoDS_Face face;
  std::vector<uint32_t> key; // sorted curve ids
};

// The curves as edges in the plane z = 0 with their ids; `grow(x, y)` gets
// points spanning their bounding box.
template <typename Grow>
void curve_edges(const Qoob3dCurves &c, std::vector<TopoDS_Shape> &edges, std::vector<int> &ids,
                 Grow &&grow) {
  const gp_Dir z(0, 0, 1), x(1, 0, 0);
  for (size_t i = 0; i + 3 < c.lines.size() && i / 4 < c.line_ids.size(); i += 4) {
    gp_Pnt a(c.lines[i], c.lines[i + 1], 0.0), b(c.lines[i + 2], c.lines[i + 3], 0.0);
    edges.push_back(BRepBuilderAPI_MakeEdge(a, b).Edge());
    ids.push_back(static_cast<int>(c.line_ids[i / 4]));
    grow(a.X(), a.Y());
    grow(b.X(), b.Y());
  }
  for (size_t i = 0; i + 2 < c.circles.size() && i / 3 < c.circle_ids.size(); i += 3) {
    const double cx = c.circles[i], cy = c.circles[i + 1], r = c.circles[i + 2];
    gp_Circ circ(gp_Ax2(gp_Pnt(cx, cy, 0.0), z, x), r);
    edges.push_back(BRepBuilderAPI_MakeEdge(circ).Edge());
    ids.push_back(static_cast<int>(c.circle_ids[i / 3]));
    grow(cx - r, cy - r);
    grow(cx + r, cy + r);
  }
  for (size_t i = 0; i + 4 < c.arcs.size() && i / 5 < c.arc_ids.size(); i += 5) {
    const double cx = c.arcs[i], cy = c.arcs[i + 1], r = c.arcs[i + 2];
    gp_Circ circ(gp_Ax2(gp_Pnt(cx, cy, 0.0), z, x), r);
    edges.push_back(BRepBuilderAPI_MakeEdge(circ, c.arcs[i + 3], c.arcs[i + 4]).Edge());
    ids.push_back(static_cast<int>(c.arc_ids[i / 5]));
    grow(cx - r, cy - r);
    grow(cx + r, cy + r);
  }
}

// Splits a helper face by all curves (General Fuse) and returns every closed
// region with the ids of its boundary curves (ADR-0028).
std::vector<Region> split_regions(const Qoob3dCurves &c) {
  std::vector<TopoDS_Shape> edges;
  std::vector<int> ids;
  double lo[2] = {1e300, 1e300}, hi[2] = {-1e300, -1e300};
  curve_edges(c, edges, ids, [&](double x, double y) {
    lo[0] = std::min(lo[0], x);
    lo[1] = std::min(lo[1], y);
    hi[0] = std::max(hi[0], x);
    hi[1] = std::max(hi[1], y);
  });
  std::vector<Region> out;
  if (edges.empty()) {
    return out;
  }

  // 1. Helper face around everything.
  const double size = std::max(hi[0] - lo[0], hi[1] - lo[1]);
  const double m = std::max(1.0, 0.1 * size);
  TopoDS_Face base =
      BRepBuilderAPI_MakeFace(gp_Pln(gp::XOY()), lo[0] - m, hi[0] + m, lo[1] - m, hi[1] + m)
          .Face();

  // 2. General Fuse: split the face by all curves. The fuzzy value glues
  //    end points that differ by rounding (e.g. arc ends).
  BOPAlgo_Builder builder;
  builder.SetRunParallel(false);
  builder.SetFuzzyValue(1e-7);
  builder.AddArgument(base);
  for (const auto &e : edges) {
    builder.AddArgument(e);
  }
  builder.Perform();
  if (builder.HasErrors()) {
    throw std::runtime_error("General Fuse failed");
  }

  // 3. Split edge → curve id; edges of the helper face → -1 (outside).
  TopTools_DataMapOfShapeInteger owner;
  for (size_t i = 0; i < edges.size(); ++i) {
    bind_pieces(builder, edges[i], ids[i], owner);
  }
  for (TopExp_Explorer ex(base, TopAbs_EDGE); ex.More(); ex.Next()) {
    bind_pieces(builder, ex.Current(), -1, owner);
  }

  // 4. Every split face not touching the helper boundary is a profile.
  TopTools_ListOfShape faces = builder.Modified(base);
  if (faces.IsEmpty()) {
    faces.Append(base);
  }
  for (TopTools_ListIteratorOfListOfShape it(faces); it.More(); it.Next()) {
    const TopoDS_Face &face = TopoDS::Face(it.Value());
    std::vector<uint32_t> key;
    bool outside = false;
    for (TopExp_Explorer ex(face, TopAbs_EDGE); ex.More(); ex.Next()) {
      if (!owner.IsBound(ex.Current())) {
        continue;
      }
      const int id = owner.Find(ex.Current());
      if (id < 0) {
        outside = true;
        break;
      }
      key.push_back(static_cast<uint32_t>(id));
    }
    if (outside || key.empty()) {
      continue;
    }
    std::sort(key.begin(), key.end());
    key.erase(std::unique(key.begin(), key.end()), key.end());
    out.push_back(Region{face, key});
  }
  return out;
}

gp_Trsf frame_trsf(rust::Slice<const double> f) {
  if (f.size() != 12) {
    throw std::runtime_error("plane frame needs 12 values");
  }
  gp_Ax3 to(gp_Pnt(f[0], f[1], f[2]), gp_Dir(f[9], f[10], f[11]), gp_Dir(f[3], f[4], f[5]));
  gp_Trsf t;
  t.SetDisplacement(gp_Ax3(), to);
  return t;
}

gp_Vec vec3(rust::Slice<const double> v, const char *what) {
  if (v.size() != 3) {
    throw std::runtime_error(std::string(what) + " needs 3 values");
  }
  return gp_Vec(v[0], v[1], v[2]);
}

std::unique_ptr<TopoDS_Shape> own(const TopoDS_Shape &s) {
  if (s.IsNull()) {
    throw std::runtime_error("operation produced no shape");
  }
  return std::make_unique<TopoDS_Shape>(s);
}

// Appends `s` to `out`, compounds replaced by their (nested) children.
void flatten(const TopoDS_Shape &s, TopTools_ListOfShape &out) {
  if (s.IsNull()) {
    return;
  }
  if (s.ShapeType() != TopAbs_COMPOUND) {
    out.Append(s);
    return;
  }
  for (TopoDS_Iterator it(s); it.More(); it.Next()) {
    flatten(it.Value(), out);
  }
}

// Merges coplanar / co-cylindrical faces after booleans so no seams show.
TopoDS_Shape unify(const TopoDS_Shape &s) {
  ShapeUpgrade_UnifySameDomain u(s, Standard_True, Standard_True, Standard_True);
  u.Build();
  return u.Shape();
}

} // namespace

Qoob3dRegions qoob3d_sketch_regions(const Qoob3dCurves &curves, double deflection) {
  return guarded("sketch_regions", [&] {
    Qoob3dRegions out;
    out.vertex_starts.push_back(0);
    out.index_starts.push_back(0);
    out.key_starts.push_back(0);
    for (const Region &region : split_regions(curves)) {
      const TopoDS_Face &face = region.face;
      GProp_GProps props;
      BRepGProp::SurfaceProperties(face, props);

      BRepMesh_IncrementalMesh mesher(face, deflection, Standard_False, 0.3, Standard_True);
      TopLoc_Location loc;
      const Handle(Poly_Triangulation) &tri = BRep_Tool::Triangulation(face, loc);
      if (tri.IsNull()) {
        throw std::runtime_error("profile face could not be triangulated");
      }
      const gp_Trsf trsf = loc.Transformation();
      for (int i = 1; i <= tri->NbNodes(); ++i) {
        const gp_Pnt p = tri->Node(i).Transformed(trsf);
        out.positions.push_back(p.X());
        out.positions.push_back(p.Y());
      }
      for (int t = 1; t <= tri->NbTriangles(); ++t) {
        int a, b, c;
        tri->Triangle(t).Get(a, b, c);
        out.indices.push_back(a - 1);
        out.indices.push_back(b - 1);
        out.indices.push_back(c - 1);
      }
      for (uint32_t id : region.key) {
        out.key_ids.push_back(id);
      }
      out.areas.push_back(props.Mass());
      out.vertex_starts.push_back(static_cast<uint32_t>(out.positions.size() / 2));
      out.index_starts.push_back(static_cast<uint32_t>(out.indices.size()));
      out.key_starts.push_back(static_cast<uint32_t>(out.key_ids.size()));
    }
    return out;
  });
}

// ---------------------------------------------------------------- solid modelling

std::unique_ptr<TopoDS_Shape> qoob3d_profile_faces(const Qoob3dCurves &curves,
                                                   rust::Slice<const uint32_t> keys,
                                                   rust::Slice<const uint32_t> key_starts,
                                                   rust::Slice<const double> frame) {
  return guarded("profile_faces", [&] {
    const std::vector<Region> regions = split_regions(curves);
    const gp_Trsf trsf = frame_trsf(frame);
    BRep_Builder bb;
    TopoDS_Compound compound;
    bb.MakeCompound(compound);
    for (size_t k = 0; k + 1 < key_starts.size(); ++k) {
      std::vector<uint32_t> key(keys.begin() + key_starts[k], keys.begin() + key_starts[k + 1]);
      const Region *found = nullptr;
      for (const Region &r : regions) {
        if (r.key == key) {
          if (found) {
            throw std::runtime_error("profile is ambiguous");
          }
          found = &r;
        }
      }
      if (!found) {
        throw std::runtime_error("profile not found");
      }
      bb.Add(compound, BRepBuilderAPI_Transform(found->face, trsf, Standard_True).Shape());
    }
    return own(compound);
  });
}

std::unique_ptr<TopoDS_Shape> qoob3d_outline_face(const Qoob3dCurves &curves) {
  return guarded("outline_face", [&] {
    std::vector<TopoDS_Shape> edges;
    std::vector<int> ids;
    curve_edges(curves, edges, ids, [](double, double) {});
    TopTools_ListOfShape list;
    for (const TopoDS_Shape &e : edges) {
      list.Append(e);
    }
    BRepBuilderAPI_MakeWire wire;
    wire.Add(list);
    if (!wire.IsDone() || !wire.Wire().Closed()) {
      throw std::runtime_error("the outline is not closed");
    }
    // The wire runs like its first edge, and the outlines of the script API
    // start counter-clockwise: no search for the inner side (a fifth of a
    // profile cut with many tools).
    return own(BRepBuilderAPI_MakeFace(gp_Pln(gp::XOY()), wire.Wire(), Standard_False).Face());
  });
}

std::unique_ptr<TopoDS_Shape> qoob3d_extrude(const TopoDS_Shape &faces,
                                             rust::Slice<const double> vector,
                                             bool symmetric) {
  return guarded("extrude", [&] {
    gp_Vec v = vec3(vector, "extrude vector");
    TopoDS_Shape base = faces;
    if (symmetric) {
      gp_Trsf t;
      t.SetTranslation(-0.5 * v);
      base = BRepBuilderAPI_Transform(faces, t, Standard_True).Shape();
    }
    BRepPrimAPI_MakePrism prism(base, v);
    prism.Build();
    if (!prism.IsDone()) {
      throw std::runtime_error("extrusion failed");
    }
    return own(prism.Shape());
  });
}

std::unique_ptr<TopoDS_Shape> qoob3d_revolve(const TopoDS_Shape &faces,
                                             rust::Slice<const double> axis, double angle) {
  return guarded("revolve", [&] {
    if (axis.size() != 6) {
      throw std::runtime_error("axis needs 6 values");
    }
    gp_Ax1 ax(gp_Pnt(axis[0], axis[1], axis[2]), gp_Dir(axis[3], axis[4], axis[5]));
    BRepPrimAPI_MakeRevol revol(faces, ax, angle, Standard_True);
    revol.Build();
    if (!revol.IsDone()) {
      throw std::runtime_error("revolve failed");
    }
    return own(revol.Shape());
  });
}

std::unique_ptr<TopoDS_Shape> qoob3d_loft(const TopoDS_Shape &faces,
                                          rust::Slice<const double> start_point,
                                          rust::Slice<const double> end_point) {
  return guarded("loft", [&] {
    BRepOffsetAPI_ThruSections loft(Standard_True, Standard_True);
    loft.CheckCompatibility(Standard_True);
    if (start_point.size() == 3) {
      loft.AddVertex(
          BRepBuilderAPI_MakeVertex(gp_Pnt(start_point[0], start_point[1], start_point[2]))
              .Vertex());
    }
    for (TopoDS_Iterator it(faces); it.More(); it.Next()) {
      loft.AddWire(BRepTools::OuterWire(TopoDS::Face(it.Value())));
    }
    if (end_point.size() == 3) {
      loft.AddVertex(
          BRepBuilderAPI_MakeVertex(gp_Pnt(end_point[0], end_point[1], end_point[2])).Vertex());
    }
    loft.Build();
    if (!loft.IsDone()) {
      throw std::runtime_error("loft failed");
    }
    return own(loft.Shape());
  });
}

namespace {

// One BOP for any number of shapes on both sides, in OCCT's parallel mode
// (AG-PF-01; OSD_ThreadPool, no extra library).
TopoDS_Shape boolean(const TopTools_ListOfShape &objects, const TopTools_ListOfShape &tools,
                     uint32_t op) {
  if (objects.IsEmpty() || tools.IsEmpty()) {
    throw std::runtime_error("boolean needs shapes on both sides");
  }
  std::unique_ptr<BRepAlgoAPI_BooleanOperation> bop;
  const char *failure = nullptr;
  switch (op) {
  case 0:
    bop = std::make_unique<BRepAlgoAPI_Fuse>();
    failure = "join failed";
    break;
  case 1:
    bop = std::make_unique<BRepAlgoAPI_Cut>();
    failure = "cut failed";
    break;
  case 2:
    bop = std::make_unique<BRepAlgoAPI_Common>();
    failure = "common failed";
    break;
  default:
    throw std::runtime_error("unknown boolean operation");
  }
  bop->SetRunParallel(Standard_True);
  bop->SetArguments(objects);
  bop->SetTools(tools);
  bop->Build();
  if (!bop->IsDone() || bop->HasErrors()) {
    throw std::runtime_error(failure);
  }
  return unify(bop->Shape());
}

// Extent of an outline in the plane z = 0, with a margin.
struct Span {
  double x0, y0, x1, y1;
  bool tool;
};

Span span_of(const TopoDS_Shape &s, bool tool) {
  Bnd_Box box;
  BRepBndLib::Add(s, box, Standard_False);
  box.Enlarge(1e-6);
  double z0, z1;
  Span span{0.0, 0.0, 0.0, 0.0, tool};
  box.Get(span.x0, span.y0, z0, span.x1, span.y1, z1);
  return span;
}

// Whether a tool overlaps another tool or a hole (a sweep along X: a plate
// with a thousand holes is no million comparisons).
bool tools_collide(std::vector<Span> spans) {
  std::sort(spans.begin(), spans.end(), [](const Span &a, const Span &b) { return a.x0 < b.x0; });
  for (size_t i = 0; i < spans.size(); ++i) {
    for (size_t j = i + 1; j < spans.size() && spans[j].x0 <= spans[i].x1; ++j) {
      if ((spans[i].tool || spans[j].tool) && spans[j].y0 <= spans[i].y1 &&
          spans[i].y0 <= spans[j].y1) {
        return true;
      }
    }
  }
  return false;
}

// Whether the material of a planar profile face is seen from +Z.
bool faces_up(const TopoDS_Face &face) {
  const bool plane_up = BRepAdaptor_Surface(face).Plane().Axis().Direction().Z() > 0.0;
  return plane_up != (face.Orientation() == TopAbs_REVERSED);
}

// Profile cut without a boolean (AG-PF-02): the tools become inner wires of
// the base face. Only if that is certainly the same result: one base face,
// tools without holes, each strictly inside the base, clear of its holes and
// of each other (by bounding boxes). Otherwise false, and the BOP decides.
bool punch(const TopTools_ListOfShape &objects, const TopTools_ListOfShape &tools,
           TopoDS_Shape &out) {
  if (objects.Extent() != 1 || objects.First().ShapeType() != TopAbs_FACE) {
    return false;
  }
  const TopoDS_Face base = TopoDS::Face(objects.First());
  const TopoDS_Wire outer = BRepTools::OuterWire(base);
  std::vector<Span> spans; // holes of the base, then the tools
  for (TopoDS_Iterator it(base); it.More(); it.Next()) {
    if (!it.Value().IsSame(outer)) {
      spans.push_back(span_of(it.Value(), false));
    }
  }
  std::vector<TopoDS_Wire> holes;
  const bool base_up = faces_up(base);
  for (TopTools_ListIteratorOfListOfShape t(tools); t.More(); t.Next()) {
    if (t.Value().ShapeType() != TopAbs_FACE) {
      return false;
    }
    const TopoDS_Face tool = TopoDS::Face(t.Value());
    TopoDS_Iterator wires(tool); // orientations as seen through the face
    if (!wires.More()) {
      return false;
    }
    const TopoDS_Wire wire = TopoDS::Wire(wires.Value());
    wires.Next();
    if (wires.More()) {
      return false;
    }
    spans.push_back(span_of(wire, true));
    // A hole runs against the outline of the face it is cut from.
    holes.push_back(faces_up(tool) == base_up ? TopoDS::Wire(wire.Reversed()) : wire);
  }
  if (tools_collide(spans)) {
    return false;
  }
  BRepBuilderAPI_MakeFace result(base);
  // One classifier for all tools; it works in the parameters of the plane.
  const gp_Pln plane = BRepAdaptor_Surface(base).Plane();
  IntTools_FClass2d where(base, 1e-7);
  for (const TopoDS_Wire &hole : holes) {
    // The outline does not reach the outer wire and one of its points lies
    // in the material: all of it does.
    BRepExtrema_DistShapeShape gap(hole, outer);
    if (!gap.IsDone() || gap.Value() < 1e-6) {
      return false;
    }
    TopExp_Explorer vertex(hole, TopAbs_VERTEX);
    if (!vertex.More()) {
      return false;
    }
    double u = 0.0, v = 0.0;
    ElSLib::Parameters(plane, BRep_Tool::Pnt(TopoDS::Vertex(vertex.Current())), u, v);
    if (where.Perform(gp_Pnt2d(u, v)) != TopAbs_IN) {
      return false;
    }
    result.Add(hole);
  }
  out = result.Face();
  // Without stored 2D curves OCCT projects every edge onto the plane again
  // whenever it is asked about the face: the validity check of a plate with
  // 1000 holes took 9 s instead of 4 s.
  for (TopExp_Explorer edge(out, TopAbs_EDGE); edge.More(); edge.Next()) {
    BRepLib::BuildPCurveForEdgeOnPlane(TopoDS::Edge(edge.Current()), TopoDS::Face(out));
  }
  return true;
}

} // namespace

std::unique_ptr<TopoDS_Shape> qoob3d_boolean(const TopoDS_Shape &a, const TopoDS_Shape &b,
                                             uint32_t op) {
  return guarded("boolean", [&] {
    // Compounds are argument lists: one BOP for any number of shapes
    // (batched booleans, AG-SC-05), and never a self-interfering compound.
    TopTools_ListOfShape objects, tools;
    flatten(a, objects);
    flatten(b, tools);
    return own(boolean(objects, tools, op));
  });
}

std::unique_ptr<TopoDS_Shape> qoob3d_profile_boolean(const TopoDS_Shape &a, const TopoDS_Shape &b,
                                                     uint32_t op) {
  return guarded("profile_boolean", [&] {
    TopTools_ListOfShape objects, tools;
    flatten(a, objects);
    flatten(b, tools);
    TopoDS_Shape result;
    if (op != 1 || !punch(objects, tools, result)) {
      result = boolean(objects, tools, op);
    }
    TopExp_Explorer face(result, TopAbs_FACE);
    if (!face.More()) {
      throw std::runtime_error("the result is empty");
    }
    BRep_Builder bb;
    TopoDS_Compound faces;
    bb.MakeCompound(faces);
    for (; face.More(); face.Next()) {
      bb.Add(faces, face.Current());
    }
    return own(faces);
  });
}

double qoob3d_distance(const TopoDS_Shape &a, const TopoDS_Shape &b, double max) {
  return guarded("distance", [&] {
    Bnd_Box ba, bb;
    BRepBndLib::Add(a, ba);
    BRepBndLib::Add(b, bb);
    // The box distance is a lower bound; beyond `max` it is answer enough.
    if (ba.IsOut(bb) && ba.Distance(bb) > max) {
      return ba.Distance(bb);
    }
    BRepExtrema_DistShapeShape dist(a, b);
    if (!dist.IsDone()) {
      throw std::runtime_error("distance failed");
    }
    // Inner solutions: one shape inside the other counts as touching.
    return dist.InnerSolution() ? 0.0 : dist.Value();
  });
}

double qoob3d_volume(const TopoDS_Shape &shape) {
  return guarded("volume", [&] {
    GProp_GProps props;
    BRepGProp::VolumeProperties(shape, props);
    return props.Mass();
  });
}

bool qoob3d_is_valid(const TopoDS_Shape &shape) {
  return guarded("is_valid", [&] {
    BRepCheck_Analyzer check(shape);
    return static_cast<bool>(check.IsValid());
  });
}

uint32_t qoob3d_solid_count(const TopoDS_Shape &shape) {
  return guarded("solid_count", [&] {
    TopTools_IndexedMapOfShape solids;
    TopExp::MapShapes(shape, TopAbs_SOLID, solids);
    return static_cast<uint32_t>(solids.Extent());
  });
}

std::unique_ptr<TopoDS_Shape> qoob3d_solid(const TopoDS_Shape &shape, uint32_t index) {
  return guarded("solid", [&] {
    TopTools_IndexedMapOfShape solids;
    TopExp::MapShapes(shape, TopAbs_SOLID, solids);
    if (index >= static_cast<uint32_t>(solids.Extent())) {
      throw std::runtime_error("solid index out of range");
    }
    return own(solids(static_cast<int>(index) + 1));
  });
}

std::unique_ptr<TopoDS_Shape> qoob3d_append(const TopoDS_Shape &a, const TopoDS_Shape &b) {
  return guarded("append", [&] {
    BRep_Builder bb;
    TopoDS_Compound out;
    bb.MakeCompound(out);
    for (const TopoDS_Shape *s : {&a, &b}) {
      if (s->IsNull()) {
        continue;
      }
      if (s->ShapeType() == TopAbs_COMPOUND) {
        for (TopoDS_Iterator it(*s); it.More(); it.Next()) {
          bb.Add(out, it.Value());
        }
      } else {
        bb.Add(out, *s);
      }
    }
    return own(out);
  });
}

namespace {

Handle(Geom_BSplineCurve) interpolate(const std::vector<gp_Pnt> &pts) {
  Handle(TColgp_HArray1OfPnt) arr = new TColgp_HArray1OfPnt(1, static_cast<int>(pts.size()));
  for (size_t i = 0; i < pts.size(); ++i) {
    arr->SetValue(static_cast<int>(i) + 1, pts[i]);
  }
  GeomAPI_Interpolate interp(arr, Standard_False, 1e-7);
  interp.Perform();
  if (!interp.IsDone()) {
    throw std::runtime_error("spline interpolation failed");
  }
  return interp.Curve();
}

} // namespace

std::unique_ptr<TopoDS_Shape> qoob3d_outline_prism(rust::Slice<const double> points,
                                                   rust::Slice<const uint32_t> starts,
                                                   rust::Slice<const uint32_t> kinds,
                                                   rust::Slice<const double> holes,
                                                   rust::Slice<const double> normal,
                                                   rust::Slice<const double> vector) {
  return guarded("outline_prism", [&] {
    if (starts.size() < 2 || kinds.size() + 1 != starts.size()) {
      throw std::runtime_error("outline pieces are inconsistent");
    }
    auto pt = [&](uint32_t i) {
      if (3 * i + 2 >= points.size()) {
        throw std::runtime_error("outline point out of range");
      }
      return gp_Pnt(points[3 * i], points[3 * i + 1], points[3 * i + 2]);
    };
    BRepBuilderAPI_MakeWire wire;
    for (size_t k = 0; k + 1 < starts.size(); ++k) {
      const uint32_t a = starts[k], b = starts[k + 1];
      if (b < a + 2) {
        throw std::runtime_error("outline piece needs two points");
      }
      if (kinds[k] == 1 && b - a >= 3) {
        std::vector<gp_Pnt> pts;
        for (uint32_t i = a; i < b; ++i) {
          pts.push_back(pt(i));
        }
        wire.Add(BRepBuilderAPI_MakeEdge(interpolate(pts)).Edge());
      } else {
        for (uint32_t i = a; i + 1 < b; ++i) {
          wire.Add(BRepBuilderAPI_MakeEdge(pt(i), pt(i + 1)).Edge());
        }
      }
      if (!wire.IsDone()) {
        throw std::runtime_error("outline is not connected");
      }
    }
    if (!wire.Wire().Closed()) {
      throw std::runtime_error("outline is not closed");
    }
    BRepBuilderAPI_MakeFace face(wire.Wire(), Standard_True);
    if (!face.IsDone()) {
      throw std::runtime_error("outline is not planar");
    }
    const gp_Dir n(vec3(normal, "normal"));
    for (size_t i = 0; i + 3 < holes.size(); i += 4) {
      gp_Circ c(gp_Ax2(gp_Pnt(holes[i], holes[i + 1], holes[i + 2]), n), holes[i + 3]);
      TopoDS_Wire w = BRepBuilderAPI_MakeWire(BRepBuilderAPI_MakeEdge(c).Edge()).Wire();
      face.Add(TopoDS::Wire(w.Reversed()));
    }
    ShapeFix_Face fix(face.Face());
    fix.Perform();
    BRepPrimAPI_MakePrism prism(fix.Face(), vec3(vector, "extrude vector"));
    prism.Build();
    if (!prism.IsDone()) {
      throw std::runtime_error("extrusion failed");
    }
    return own(prism.Shape());
  });
}

std::unique_ptr<TopoDS_Shape> qoob3d_sweep_circle(rust::Slice<const double> path, double radius,
                                                  rust::Slice<const double> binormal) {
  return guarded("sweep_circle", [&] {
    if (path.size() < 9 || path.size() % 3 != 0) {
      throw std::runtime_error("sweep path needs at least three points");
    }
    std::vector<gp_Pnt> pts;
    for (size_t i = 0; i < path.size(); i += 3) {
      pts.emplace_back(path[i], path[i + 1], path[i + 2]);
    }
    Handle(Geom_BSplineCurve) curve = interpolate(pts);
    TopoDS_Wire spine = BRepBuilderAPI_MakeWire(BRepBuilderAPI_MakeEdge(curve).Edge()).Wire();

    gp_Pnt p0;
    gp_Vec t0;
    curve->D1(curve->FirstParameter(), p0, t0);
    gp_Circ section_circle(gp_Ax2(p0, gp_Dir(t0)), radius);
    TopoDS_Wire section =
        BRepBuilderAPI_MakeWire(BRepBuilderAPI_MakeEdge(section_circle).Edge()).Wire();

    BRepOffsetAPI_MakePipeShell pipe(spine);
    if (binormal.size() == 3) {
      pipe.SetMode(gp_Dir(binormal[0], binormal[1], binormal[2]));
    } else {
      pipe.SetMode(Standard_True); // Frenet: exact for helices
    }
    pipe.Add(section);
    pipe.Build();
    if (!pipe.IsDone()) {
      throw std::runtime_error("sweep failed");
    }
    if (!pipe.MakeSolid()) {
      throw std::runtime_error("sweep could not be closed into a solid");
    }
    return own(pipe.Shape());
  });
}

// ---------------------------------------------------------------- script kernel (ADR-0053)

namespace {

void need(rust::Slice<const double> values, size_t n, const char *what) {
  if (values.size() != n) {
    throw std::runtime_error(std::string(what) + " needs " + std::to_string(n) + " values");
  }
}

gp_Pnt point_at(rust::Slice<const double> xyz, size_t i) {
  return gp_Pnt(xyz[3 * i], xyz[3 * i + 1], xyz[3 * i + 2]);
}

// STEP translation uses process-wide static settings (Interface_Static).
std::mutex step_mutex;

} // namespace

std::unique_ptr<TopoDS_Shape> qoob3d_primitive(uint32_t kind, rust::Slice<const double> d) {
  return guarded("primitive", [&] {
    switch (kind) {
    case 0:
      need(d, 6, "box");
      return own(BRepPrimAPI_MakeBox(gp_Pnt(d[0], d[1], d[2]), d[3], d[4], d[5]).Shape());
    case 1:
      need(d, 2, "cylinder");
      return own(BRepPrimAPI_MakeCylinder(d[0], d[1]).Shape());
    case 2:
      need(d, 1, "sphere");
      return own(BRepPrimAPI_MakeSphere(d[0]).Shape());
    case 3:
      need(d, 3, "cone");
      return own(BRepPrimAPI_MakeCone(d[0], d[1], d[2]).Shape());
    case 4:
      need(d, 2, "torus");
      return own(BRepPrimAPI_MakeTorus(d[0], d[1]).Shape());
    default:
      throw std::runtime_error("unknown primitive");
    }
  });
}

std::unique_ptr<TopoDS_Shape> qoob3d_transform(const TopoDS_Shape &shape, uint32_t kind,
                                               rust::Slice<const double> p) {
  return guarded("transform", [&] {
    gp_Trsf t;
    switch (kind) {
    case 0:
      need(p, 3, "translation");
      t.SetTranslation(gp_Vec(p[0], p[1], p[2]));
      break;
    case 1:
      need(p, 4, "rotation");
      t.SetRotation(gp_Ax1(gp::Origin(), gp_Dir(p[0], p[1], p[2])), p[3]);
      break;
    case 2:
      need(p, 3, "mirror");
      t.SetMirror(gp_Ax2(gp::Origin(), gp_Dir(p[0], p[1], p[2])));
      break;
    case 3:
      need(p, 1, "scale");
      t.SetScale(gp::Origin(), p[0]);
      break;
    case 4:
      t = frame_trsf(p);
      break;
    default:
      throw std::runtime_error("unknown transform");
    }
    // Rigid motions only set a location; mirror and scale copy the geometry.
    return own(BRepBuilderAPI_Transform(shape, t, Standard_False).Shape());
  });
}

std::unique_ptr<TopoDS_Shape> qoob3d_compound() {
  return guarded("compound", [&] {
    BRep_Builder bb;
    TopoDS_Compound c;
    bb.MakeCompound(c);
    return own(c);
  });
}

void qoob3d_compound_add(TopoDS_Shape &compound, const TopoDS_Shape &part) {
  guarded("compound_add", [&] {
    if (compound.IsNull() || compound.ShapeType() != TopAbs_COMPOUND) {
      throw std::runtime_error("not a compound");
    }
    TopTools_ListOfShape parts;
    flatten(part, parts);
    BRep_Builder bb;
    for (TopTools_ListIteratorOfListOfShape it(parts); it.More(); it.Next()) {
      bb.Add(compound, it.Value());
    }
  });
}

Qoob3dElements qoob3d_elements(const TopoDS_Shape &shape, bool faces) {
  return guarded("elements", [&] {
    TopTools_IndexedMapOfShape map;
    TopExp::MapShapes(shape, faces ? TopAbs_FACE : TopAbs_EDGE, map);
    TopTools_IndexedDataMapOfShapeListOfShape edge_faces;
    if (!faces) {
      TopExp::MapShapesAndAncestors(shape, TopAbs_EDGE, TopAbs_FACE, edge_faces);
    }
    Qoob3dElements out;
    for (int i = 1; i <= map.Extent(); ++i) {
      uint32_t kind = 2;
      gp_Dir dir(0, 0, 1);
      if (faces) {
        const TopoDS_Face &face = TopoDS::Face(map(i));
        BRepAdaptor_Surface surface(face);
        if (surface.GetType() == GeomAbs_Plane) {
          kind = 0;
          dir = surface.Plane().Axis().Direction();
          if (face.Orientation() == TopAbs_REVERSED) {
            dir.Reverse();
          }
        } else if (surface.GetType() == GeomAbs_Cylinder) {
          kind = 1;
          dir = surface.Cylinder().Axis().Direction();
        }
      } else {
        const TopoDS_Edge &edge = TopoDS::Edge(map(i));
        bool seam = BRep_Tool::Degenerated(edge);
        if (edge_faces.Contains(edge)) {
          for (TopTools_ListIteratorOfListOfShape it(edge_faces.FindFromKey(edge)); it.More();
               it.Next()) {
            seam = seam || BRep_Tool::IsClosed(edge, TopoDS::Face(it.Value()));
          }
        }
        if (seam) {
          kind = 3;
        } else {
          BRepAdaptor_Curve curve(edge);
          if (curve.GetType() == GeomAbs_Line) {
            kind = 0;
            dir = curve.Line().Direction();
          } else if (curve.GetType() == GeomAbs_Circle) {
            kind = 1;
            dir = curve.Circle().Axis().Direction();
          }
        }
      }
      Bnd_Box box;
      double b[6] = {0, 0, 0, 0, 0, 0};
      if (kind != 3) {
        BRepBndLib::AddOptimal(map(i), box, Standard_False, Standard_False);
        if (!box.IsVoid()) {
          box.Get(b[0], b[1], b[2], b[3], b[4], b[5]);
        }
      }
      out.kinds.push_back(kind);
      for (double v : b) {
        out.bounds.push_back(v);
      }
      out.directions.push_back(dir.X());
      out.directions.push_back(dir.Y());
      out.directions.push_back(dir.Z());
    }
    return out;
  });
}

std::unique_ptr<TopoDS_Shape> qoob3d_fillet(const TopoDS_Shape &shape,
                                            rust::Slice<const uint32_t> edges, double size,
                                            bool chamfer) {
  return guarded(chamfer ? "chamfer" : "fillet", [&] {
    TopTools_IndexedMapOfShape map;
    TopExp::MapShapes(shape, TopAbs_EDGE, map);
    auto edge = [&](uint32_t i) {
      if (i < 1 || static_cast<int>(i) > map.Extent()) {
        throw std::runtime_error("edge index out of range");
      }
      return TopoDS::Edge(map(static_cast<int>(i)));
    };
    TopoDS_Shape result;
    if (chamfer) {
      BRepFilletAPI_MakeChamfer mk(shape);
      for (uint32_t i : edges) {
        mk.Add(size, edge(i));
      }
      mk.Build();
      if (!mk.IsDone()) {
        throw std::runtime_error("chamfer failed");
      }
      result = mk.Shape();
    } else {
      BRepFilletAPI_MakeFillet mk(shape);
      for (uint32_t i : edges) {
        mk.Add(size, edge(i));
      }
      mk.Build();
      if (!mk.IsDone()) {
        throw std::runtime_error("fillet failed");
      }
      result = mk.Shape();
    }
    return own(result);
  });
}

std::unique_ptr<TopoDS_Shape> qoob3d_shell(const TopoDS_Shape &shape,
                                           rust::Slice<const uint32_t> open_faces,
                                           double thickness) {
  return guarded("shell", [&] {
    TopTools_IndexedMapOfShape map;
    TopExp::MapShapes(shape, TopAbs_FACE, map);
    TopTools_ListOfShape open;
    for (uint32_t i : open_faces) {
      if (i < 1 || static_cast<int>(i) > map.Extent()) {
        throw std::runtime_error("face index out of range");
      }
      open.Append(map(static_cast<int>(i)));
    }
    BRepOffsetAPI_MakeThickSolid mk;
    // Negative offset: the wall grows inward, the outer size is kept.
    mk.MakeThickSolidByJoin(shape, open, -thickness, 1e-5);
    mk.Build();
    if (!mk.IsDone()) {
      throw std::runtime_error("shell failed");
    }
    return own(mk.Shape());
  });
}

double qoob3d_area(const TopoDS_Shape &shape) {
  return guarded("area", [&] {
    GProp_GProps props;
    BRepGProp::SurfaceProperties(shape, props);
    return props.Mass();
  });
}

rust::Vec<double> qoob3d_center(const TopoDS_Shape &shape) {
  return guarded("center", [&] {
    GProp_GProps props;
    BRepGProp::VolumeProperties(shape, props);
    const gp_Pnt c = props.CentreOfMass();
    rust::Vec<double> out;
    out.push_back(c.X());
    out.push_back(c.Y());
    out.push_back(c.Z());
    return out;
  });
}

rust::Vec<double> qoob3d_bbox(const TopoDS_Shape &shape) {
  return guarded("bbox", [&] {
    // Tight box from the exact geometry, without tolerance enlargement.
    Bnd_Box box;
    BRepBndLib::AddOptimal(shape, box, Standard_False, Standard_False);
    if (box.IsVoid()) {
      throw std::runtime_error("empty shape has no bounding box");
    }
    double b[6];
    box.Get(b[0], b[1], b[2], b[3], b[4], b[5]);
    rust::Vec<double> out;
    for (double v : b) {
      out.push_back(v);
    }
    return out;
  });
}

rust::Vec<double> qoob3d_ray(const TopoDS_Shape &shape, rust::Slice<const double> origin,
                             rust::Slice<const double> direction) {
  return guarded("ray", [&] {
    need(origin, 3, "ray origin");
    need(direction, 3, "ray direction");
    const gp_Lin line(point_at(origin, 0), gp_Dir(direction[0], direction[1], direction[2]));
    IntCurvesFace_ShapeIntersector inter;
    inter.Load(shape, 1e-7);
    // A half-line: only hits ahead of the origin.
    inter.Perform(line, 0.0, RealLast());
    if (!inter.IsDone()) {
      throw std::runtime_error("ray intersection failed");
    }
    std::vector<std::pair<double, gp_Pnt>> hits;
    for (int i = 1; i <= inter.NbPnt(); ++i) {
      hits.emplace_back(inter.WParameter(i), inter.Pnt(i));
    }
    std::sort(hits.begin(), hits.end(),
              [](const auto &a, const auto &b) { return a.first < b.first; });
    rust::Vec<double> out;
    double last = -1.0;
    for (const auto &[w, p] : hits) {
      // A hit on an edge or seam is reported by every face that shares it.
      if (!out.empty() && w - last < 1e-7) {
        continue;
      }
      last = w;
      out.push_back(p.X());
      out.push_back(p.Y());
      out.push_back(p.Z());
    }
    return out;
  });
}

std::unique_ptr<TopoDS_Shape> qoob3d_pipe(rust::Slice<const double> points, double radius,
                                          double bend_radius) {
  return guarded("pipe", [&] {
    const size_t n = points.size() / 3;
    if (n < 2 || points.size() % 3 != 0) {
      throw std::runtime_error("pipe needs at least two points");
    }
    // Every corner is rounded by a 2D fillet in the plane of its two
    // segments; the edges are trimmed by OCCT (ChFi2d).
    BRepBuilderAPI_MakeWire wire;
    TopoDS_Edge current = BRepBuilderAPI_MakeEdge(point_at(points, 0), point_at(points, 1));
    for (size_t k = 1; k + 1 < n; ++k) {
      const gp_Pnt a = point_at(points, k - 1), p = point_at(points, k),
                   b = point_at(points, k + 1);
      TopoDS_Edge next = BRepBuilderAPI_MakeEdge(p, b);
      const gp_Vec normal = gp_Vec(a, p).Crossed(gp_Vec(p, b));
      if (normal.Magnitude() < 1e-12 * a.Distance(p) * p.Distance(b)) {
        wire.Add(current); // straight on: no bend
        current = next;
        continue;
      }
      ChFi2d_FilletAPI fillet(current, next, gp_Pln(p, gp_Dir(normal)));
      if (!fillet.Perform(bend_radius)) {
        throw std::runtime_error("bend radius does not fit at point " + std::to_string(k));
      }
      TopoDS_Edge before, after;
      const TopoDS_Edge arc = fillet.Result(p, before, after);
      if (arc.IsNull()) {
        throw std::runtime_error("bend radius does not fit at point " + std::to_string(k));
      }
      if (!before.IsNull()) {
        wire.Add(before);
      }
      wire.Add(arc);
      current = after;
    }
    if (!current.IsNull()) {
      wire.Add(current);
    }
    if (!wire.IsDone()) {
      throw std::runtime_error("pipe path is not connected");
    }
    const gp_Pnt p0 = point_at(points, 0);
    const gp_Circ circle(gp_Ax2(p0, gp_Dir(gp_Vec(p0, point_at(points, 1)))), radius);
    BRepOffsetAPI_MakePipeShell pipe(wire.Wire());
    pipe.Add(BRepBuilderAPI_MakeWire(BRepBuilderAPI_MakeEdge(circle).Edge()).Wire());
    pipe.Build();
    if (!pipe.IsDone()) {
      throw std::runtime_error("pipe sweep failed");
    }
    if (!pipe.MakeSolid()) {
      throw std::runtime_error("pipe could not be closed into a solid");
    }
    return own(pipe.Shape());
  });
}

void qoob3d_write_step(const TopoDS_Shape &shape, rust::Str path) {
  guarded("write_step", [&] {
    std::lock_guard<std::mutex> lock(step_mutex);
    STEPControl_Writer writer;
    if (writer.Transfer(shape, STEPControl_AsIs) != IFSelect_RetDone) {
      throw std::runtime_error("STEP transfer failed");
    }
    if (writer.Write(std::string(path).c_str()) != IFSelect_RetDone) {
      throw std::runtime_error("STEP file could not be written");
    }
  });
}

std::unique_ptr<TopoDS_Shape> qoob3d_read_step(rust::Str path) {
  return guarded("read_step", [&] {
    std::lock_guard<std::mutex> lock(step_mutex);
    STEPControl_Reader reader;
    if (reader.ReadFile(std::string(path).c_str()) != IFSelect_RetDone) {
      throw std::runtime_error("STEP file could not be read");
    }
    if (reader.TransferRoots() == 0) {
      throw std::runtime_error("STEP file contains no shape");
    }
    return own(reader.OneShape());
  });
}

void qoob3d_write_brep(const TopoDS_Shape &shape, rust::Str path) {
  guarded("write_brep", [&] {
    if (!BRepTools::Write(shape, std::string(path).c_str())) {
      throw std::runtime_error("BREP file could not be written");
    }
  });
}

std::unique_ptr<TopoDS_Shape> qoob3d_read_brep(rust::Str path) {
  return guarded("read_brep", [&] {
    TopoDS_Shape shape;
    BRep_Builder builder;
    if (!BRepTools::Read(shape, std::string(path).c_str(), builder) || shape.IsNull()) {
      throw std::runtime_error("BREP file could not be read");
    }
    return own(shape);
  });
}

// ---------------------------------------------------------------- script API additions

namespace {

// Largest angle (rad) between neighbouring mesh segments of curved faces in
// the overhang analysis.
const double OVERHANG_MESH_ANGLE = 0.1;

// All faces of `shape` as one compound; none is an error saying `empty`.
TopoDS_Shape faces_of(const TopoDS_Shape &shape, const char *empty) {
  TopExp_Explorer face(shape, TopAbs_FACE);
  if (!face.More()) {
    throw std::runtime_error(empty);
  }
  BRep_Builder bb;
  TopoDS_Compound faces;
  bb.MakeCompound(faces);
  for (; face.More(); face.Next()) {
    bb.Add(faces, face.Current());
  }
  return faces;
}

// Closed outlines in the plane z = 0 that do not cross, as regions: an
// outline inside an odd number of others bounds a hole.
void outline_regions(const TopoDS_Shape &outlines, TopTools_ListOfShape &regions) {
  const gp_Pln plane(gp::XOY());
  std::vector<TopoDS_Face> discs;
  std::vector<gp_Pnt2d> points; // one point of each outline
  for (TopExp_Explorer w(outlines, TopAbs_WIRE); w.More(); w.Next()) {
    const TopoDS_Wire wire = TopoDS::Wire(w.Current());
    TopExp_Explorer vertex(wire, TopAbs_VERTEX);
    if (!wire.Closed() || !vertex.More()) {
      throw std::runtime_error("an offset outline is not closed");
    }
    const gp_Pnt p = BRep_Tool::Pnt(TopoDS::Vertex(vertex.Current()));
    points.emplace_back(p.X(), p.Y());
    // `Inside`: the face is the finite side, whichever way the wire runs.
    discs.push_back(BRepBuilderAPI_MakeFace(plane, wire, Standard_True).Face());
  }
  const size_t n = discs.size();
  std::vector<std::vector<bool>> inside(n, std::vector<bool>(n, false)); // [i][j]: i in j
  std::vector<int> depth(n, 0);
  for (size_t j = 0; j < n; ++j) {
    IntTools_FClass2d where(discs[j], 1e-7);
    for (size_t i = 0; i < n; ++i) {
      if (i != j && where.Perform(points[i]) == TopAbs_IN) {
        inside[i][j] = true;
        ++depth[i];
      }
    }
  }
  for (size_t i = 0; i < n; ++i) {
    if (depth[i] % 2 != 0) {
      continue;
    }
    BRepBuilderAPI_MakeFace region(discs[i]);
    for (size_t j = 0; j < n; ++j) {
      if (depth[j] == depth[i] + 1 && inside[j][i]) {
        // A hole runs against the outline of its face.
        region.Add(TopoDS::Wire(BRepTools::OuterWire(discs[j]).Reversed()));
      }
    }
    regions.Append(region.Face());
  }
}

// The regions of the closed outline `wire` in the plane z = 0 after it grew
// by `delta` (shrank, if negative); none if it vanished. Corners that point
// outward become arcs.
void grow_outline(const TopoDS_Wire &wire, double delta, TopTools_ListOfShape &regions) {
  const gp_Pln plane(gp::XOY());
  TopExp_Explorer edges(wire, TopAbs_EDGE);
  if (!edges.More()) {
    throw std::runtime_error("an outline has no edge");
  }
  BRepAdaptor_Curve curve(TopoDS::Edge(edges.Current()));
  edges.Next();
  const double turn = curve.LastParameter() - curve.FirstParameter();
  if (!edges.More() && curve.GetType() == GeomAbs_Circle && std::abs(turn - 2.0 * M_PI) < 1e-9) {
    // A full circle changes its radius. OCCT's own shortcut for this misses
    // radii up to 1 mm and then crashes in the bisector code.
    gp_Circ circle = curve.Circle();
    if (circle.Radius() + delta > 1e-7) {
      circle.SetRadius(circle.Radius() + delta);
      const TopoDS_Wire grown = BRepBuilderAPI_MakeWire(BRepBuilderAPI_MakeEdge(circle).Edge());
      regions.Append(BRepBuilderAPI_MakeFace(plane, grown, Standard_True).Face());
    }
    return;
  }
  // The offset follows the bisectors of the outline; parts that vanish are
  // trimmed away.
  BRepOffsetAPI_MakeOffset offset(BRepBuilderAPI_MakeFace(plane, wire, Standard_True).Face(),
                                  GeomAbs_Arc);
  offset.Perform(delta);
  if (!offset.IsDone()) {
    throw std::runtime_error("offset failed");
  }
  outline_regions(offset.Shape(), regions);
}

TopoDS_Shape element(const TopTools_IndexedMapOfShape &map, uint32_t i) {
  if (i < 1 || static_cast<int>(i) > map.Extent()) {
    throw std::runtime_error("element index out of range");
  }
  return map(static_cast<int>(i));
}

double lowest_z(const TopoDS_Shape &shape) {
  Bnd_Box box;
  BRepBndLib::AddOptimal(shape, box, Standard_False, Standard_False);
  if (box.IsVoid()) {
    throw std::runtime_error("empty shape");
  }
  return box.CornerMin().Z();
}

double area_of(const TopoDS_Shape &shape) {
  GProp_GProps props;
  BRepGProp::SurfaceProperties(shape, props);
  return props.Mass();
}

// Outward normal of a planar face (mirrored planes have a left-handed frame).
gp_Dir outward(const TopoDS_Face &face, const gp_Pln &plane) {
  const gp_Dir n = plane.Position().XDirection().Crossed(plane.Position().YDirection());
  return face.Orientation() == TopAbs_REVERSED ? n.Reversed() : n;
}

// Whether a planar face with the outward normal `n` lies on the bed z = `bed`.
bool on_bed(const gp_Pln &plane, const gp_Dir &n, double bed) {
  return n.Z() < -(1.0 - 1e-9) && std::abs(plane.Location().Z() - bed) < 1e-6;
}

// Share of a triangle where a function that is linear on it and has the
// values `f` at the corners is positive.
double positive_share(const double f[3]) {
  const int count = (f[0] > 0.0) + (f[1] > 0.0) + (f[2] > 0.0);
  if (count == 0 || count == 3) {
    return count == 3 ? 1.0 : 0.0;
  }
  // The corner that stands alone cuts off a similar triangle.
  const bool lone_positive = count == 1;
  int a = 0;
  while ((f[a] > 0.0) != lone_positive) {
    ++a;
  }
  const double fa = f[a], fb = f[(a + 1) % 3], fc = f[(a + 2) % 3];
  const double corner = (fa / (fa - fb)) * (fa / (fa - fc));
  return lone_positive ? corner : 1.0 - corner;
}

// Angle (rad) between a normal and straight down.
double from_down(const gp_Dir &n) {
  return std::acos(std::max(-1.0, std::min(1.0, -n.Z())));
}

// Area of a meshed face where the outward normal is closer than `limit`
// (rad) to −Z. Every corner of a triangle carries the angle of the true
// surface normal there; between the corners it is taken as linear, which
// is exact along the turn of a cylinder, a sphere or a torus. The triangle
// areas are scaled to the exact area of the face.
double overhang_of_mesh(const TopoDS_Face &face, double limit) {
  TopLoc_Location loc;
  const Handle(Poly_Triangulation) &tri = BRep_Tool::Triangulation(face, loc);
  const Handle(Geom_Surface) surface = BRep_Tool::Surface(face);
  if (tri.IsNull() || !tri->HasUVNodes() || surface.IsNull()) {
    throw std::runtime_error("a curved face could not be meshed");
  }
  const gp_Trsf trsf = loc.Transformation();
  const bool reversed = face.Orientation() == TopAbs_REVERSED;
  // Margin to the limit per node; NaN where the surface has no normal (the
  // tip of a cone).
  std::vector<double> margin(tri->NbNodes() + 1, std::nan(""));
  for (int i = 1; i <= tri->NbNodes(); ++i) {
    gp_Dir n;
    if (GeomLib::NormEstim(surface, tri->UVNode(i), Precision::Confusion(), n) <= 1) {
      margin[i] = limit - from_down(reversed ? n.Reversed() : n);
    }
  }
  double meshed = 0.0, hit = 0.0;
  for (int t = 1; t <= tri->NbTriangles(); ++t) {
    int node[3];
    tri->Triangle(t).Get(node[0], node[1], node[2]);
    gp_Pnt p[3];
    double f[3], sum = 0.0;
    int known = 0;
    for (int k = 0; k < 3; ++k) {
      p[k] = tri->Node(node[k]).Transformed(trsf);
      f[k] = margin[node[k]];
      if (!std::isnan(f[k])) {
        sum += f[k];
        ++known;
      }
    }
    if (known == 0) {
      continue;
    }
    for (double &value : f) {
      // A corner without a normal follows the others.
      value = std::isnan(value) ? sum / known : value;
    }
    const double area = 0.5 * gp_Vec(p[0], p[1]).Crossed(gp_Vec(p[0], p[2])).Magnitude();
    meshed += area;
    hit += area * positive_share(f);
  }
  return meshed > 0.0 ? hit * area_of(face) / meshed : 0.0;
}

} // namespace

std::unique_ptr<TopoDS_Shape> qoob3d_profile_offset(const TopoDS_Shape &faces, double distance) {
  return guarded("profile_offset", [&] {
    // Every outline is offset on its own and the regions are combined
    // afterwards: the offset of a whole face needs the bisectors between
    // its wires, and OCCT crashes on those of concentric circles.
    TopTools_ListOfShape regions;
    for (TopExp_Explorer f(faces, TopAbs_FACE); f.More(); f.Next()) {
      const TopoDS_Wire outer = BRepTools::OuterWire(TopoDS::Face(f.Current()));
      TopTools_ListOfShape grown, holes;
      for (TopoDS_Iterator w(f.Current()); w.More(); w.Next()) {
        const bool hole = !w.Value().IsSame(outer);
        grow_outline(TopoDS::Wire(w.Value()), hole ? -distance : distance, hole ? holes : grown);
      }
      TopoDS_Shape cut;
      if (grown.IsEmpty() || holes.IsEmpty()) {
        regions.Append(grown);
      } else if (grown.Extent() == 1 && punch(grown, holes, cut)) {
        regions.Append(cut);
      } else {
        cut = boolean(grown, holes, 1);
        for (TopExp_Explorer left(cut, TopAbs_FACE); left.More(); left.Next()) {
          regions.Append(left.Current());
        }
      }
    }
    const char *empty = "nothing is left of the profile";
    if (regions.IsEmpty()) {
      throw std::runtime_error(empty);
    }
    BRep_Builder bb;
    TopoDS_Compound all;
    bb.MakeCompound(all);
    for (TopTools_ListIteratorOfListOfShape it(regions); it.More(); it.Next()) {
      bb.Add(all, it.Value());
    }
    if (distance > 0.0 && regions.Extent() > 1) {
      // Regions that grew may have met.
      TopTools_ListOfShape first;
      first.Append(regions.First());
      regions.RemoveFirst();
      return own(faces_of(boolean(first, regions, 0), empty));
    }
    return own(all);
  });
}

rust::Vec<double> qoob3d_element_sizes(const TopoDS_Shape &shape, bool faces,
                                       rust::Slice<const uint32_t> indices) {
  return guarded("element_sizes", [&] {
    TopTools_IndexedMapOfShape map;
    TopExp::MapShapes(shape, faces ? TopAbs_FACE : TopAbs_EDGE, map);
    rust::Vec<double> out;
    for (uint32_t i : indices) {
      GProp_GProps props;
      if (faces) {
        BRepGProp::SurfaceProperties(element(map, i), props);
      } else {
        BRepGProp::LinearProperties(element(map, i), props);
      }
      out.push_back(props.Mass());
    }
    return out;
  });
}

rust::Vec<double> qoob3d_element_distances(const TopoDS_Shape &shape, bool faces,
                                           rust::Slice<const uint32_t> indices,
                                           rust::Slice<const double> point) {
  return guarded("element_distances", [&] {
    need(point, 3, "point");
    TopTools_IndexedMapOfShape map;
    TopExp::MapShapes(shape, faces ? TopAbs_FACE : TopAbs_EDGE, map);
    const TopoDS_Vertex vertex = BRepBuilderAPI_MakeVertex(point_at(point, 0)).Vertex();
    rust::Vec<double> out;
    for (uint32_t i : indices) {
      BRepExtrema_DistShapeShape dist(vertex, element(map, i));
      if (!dist.IsDone()) {
        throw std::runtime_error("distance failed");
      }
      out.push_back(dist.Value());
    }
    return out;
  });
}

std::unique_ptr<TopoDS_Shape> qoob3d_section(const TopoDS_Shape &shape,
                                             rust::Slice<const double> frame) {
  return guarded("section", [&] {
    // In the coordinates of the plane the cut is at z = 0.
    const TopoDS_Shape local =
        BRepBuilderAPI_Transform(shape, frame_trsf(frame).Inverted(), Standard_False).Shape();
    Bnd_Box box;
    BRepBndLib::Add(local, box);
    if (box.IsVoid()) {
      throw std::runtime_error("empty shape");
    }
    const gp_Pnt lo = box.CornerMin(), hi = box.CornerMax();
    const double m = 1.0;
    TopTools_ListOfShape plane, solids;
    plane.Append(BRepBuilderAPI_MakeFace(gp_Pln(gp::XOY()), lo.X() - m, hi.X() + m, lo.Y() - m,
                                         hi.Y() + m)
                     .Face());
    flatten(local, solids);
    return own(faces_of(boolean(plane, solids, 2), "the plane does not cut the shape"));
  });
}

double qoob3d_bed_area(const TopoDS_Shape &shape) {
  return guarded("bed_area", [&] {
    const double bed = lowest_z(shape);
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(shape, TopAbs_FACE, faces);
    double total = 0.0;
    for (int i = 1; i <= faces.Extent(); ++i) {
      const TopoDS_Face &face = TopoDS::Face(faces(i));
      BRepAdaptor_Surface surface(face);
      if (surface.GetType() == GeomAbs_Plane &&
          on_bed(surface.Plane(), outward(face, surface.Plane()), bed)) {
        total += area_of(face);
      }
    }
    return total;
  });
}

double qoob3d_overhang_area(const TopoDS_Shape &shape, double max_angle) {
  return guarded("overhang_area", [&] {
    const double bed = lowest_z(shape);
    // A hair below the angle: a wall that is vertical up to rounding never counts.
    const double limit = max_angle - 1e-9;
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(shape, TopAbs_FACE, faces);
    double total = 0.0;
    BRep_Builder bb;
    TopoDS_Compound curved;
    bb.MakeCompound(curved);
    for (int i = 1; i <= faces.Extent(); ++i) {
      const TopoDS_Face &face = TopoDS::Face(faces(i));
      BRepAdaptor_Surface surface(face);
      if (surface.GetType() != GeomAbs_Plane) {
        bb.Add(curved, face);
        continue;
      }
      const gp_Dir n = outward(face, surface.Plane());
      if (from_down(n) < limit && !on_bed(surface.Plane(), n, bed)) {
        total += area_of(face);
      }
    }
    if (TopoDS_Iterator(curved).More()) {
      // Mesh a copy (same geometry): the mesh of the shape itself, which
      // export and display use, stays as it is.
      const TopoDS_Shape copy = BRepBuilderAPI_Copy(curved, Standard_False).Shape();
      Bnd_Box box;
      BRepBndLib::Add(copy, box);
      IMeshTools_Parameters mesh;
      mesh.Angle = mesh.AngleInterior = OVERHANG_MESH_ANGLE;
      mesh.Deflection = mesh.DeflectionInterior = 1e-3 * std::sqrt(box.SquareExtent());
      mesh.InParallel = Standard_True;
      BRepMesh_IncrementalMesh mesher(copy, mesh);
      for (TopExp_Explorer f(copy, TopAbs_FACE); f.More(); f.Next()) {
        total += overhang_of_mesh(TopoDS::Face(f.Current()), limit);
      }
    }
    return total;
  });
}
