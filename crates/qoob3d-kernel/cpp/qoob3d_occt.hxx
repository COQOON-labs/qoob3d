// QOOB3D's own OCCT wrappers (ADR-0014).
//
// Rules for everything in this file:
// - every function catches Standard_Failure (which does NOT derive from
//   std::exception) and rethrows it as std::runtime_error, so cxx turns it
//   into a Rust `Err` instead of aborting the process (ADR-0004);
// - results are returned as flat buffers to keep the FFI surface small.
#pragma once

#include <TopoDS_Shape.hxx>

#include "rust/cxx.h"

struct Qoob3dMesh;

Qoob3dMesh qoob3d_tessellate(const TopoDS_Shape &shape, double linear_deflection,
                             double angular_deflection);
