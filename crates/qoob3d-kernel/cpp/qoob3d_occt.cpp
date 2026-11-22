#include "qoob3d_occt.hxx"

#include <stdexcept>
#include <string>

#include <algorithm>
#include <mutex>
#include <utility>
#include <vector>

#include <BRepAdaptor_Surface.hxx>
#include <BRepFilletAPI_MakeChamfer.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <BRepOffsetAPI_MakeThickSolid.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCone.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakeSphere.hxx>
#include <BRepPrimAPI_MakeTorus.hxx>
#include <ChFi2d_FilletAPI.hxx>
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
    // Compounds are argument lists: one BOP for any number of shapes
    // (batched booleans, AG-SC-05), and never a self-interfering compound.
    TopTools_ListOfShape objects, tools;
    flatten(a, objects);
    flatten(b, tools);
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
    bop->SetArguments(objects);
    bop->SetTools(tools);
    bop->Build();
    if (!bop->IsDone() || bop->HasErrors()) {
      throw std::runtime_error(failure);
    }
    return own(unify(bop->Shape()));
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
