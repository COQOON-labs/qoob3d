#include "qoob3d_occt.hxx"

#include <stdexcept>
#include <string>

#include <algorithm>
#include <vector>

#include <BOPAlgo_Builder.hxx>
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

// Splits a helper face by all curves (General Fuse) and returns every closed
// region with the ids of its boundary curves (ADR-0028).
std::vector<Region> split_regions(const Qoob3dCurves &c) {
  std::vector<TopoDS_Shape> edges;
  std::vector<int> ids;
  double lo[2] = {1e300, 1e300}, hi[2] = {-1e300, -1e300};
  auto grow = [&](double x, double y) {
    lo[0] = std::min(lo[0], x);
    lo[1] = std::min(lo[1], y);
    hi[0] = std::max(hi[0], x);
    hi[1] = std::max(hi[1], y);
  };
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

std::unique_ptr<TopoDS_Shape> qoob3d_boolean(const TopoDS_Shape &a, const TopoDS_Shape &b,
                                             uint32_t op) {
  return guarded("boolean", [&] {
    TopoDS_Shape result;
    if (op == 0) {
      BRepAlgoAPI_Fuse fuse(a, b);
      if (!fuse.IsDone() || fuse.HasErrors()) {
        throw std::runtime_error("join failed");
      }
      result = fuse.Shape();
    } else {
      BRepAlgoAPI_Cut cut(a, b);
      if (!cut.IsDone() || cut.HasErrors()) {
        throw std::runtime_error("cut failed");
      }
      result = cut.Shape();
    }
    return own(unify(result));
  });
}

double qoob3d_distance(const TopoDS_Shape &a, const TopoDS_Shape &b) {
  return guarded("distance", [&] {
    Bnd_Box ba, bb;
    BRepBndLib::Add(a, ba);
    BRepBndLib::Add(b, bb);
    if (ba.IsOut(bb)) {
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
