#include "qoob3d_occt.hxx"

#include <stdexcept>
#include <string>

#include <BRepAdaptor_Curve.hxx>
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
