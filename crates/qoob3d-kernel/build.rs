//! Compiles QOOB3D's own OCCT wrappers (ADR-0014) against the OCCT that
//! `opencascade-sys` has built. Its fork declares `links = "occt"`, so Cargo
//! runs this script after it and passes the include dir as `DEP_OCCT_INCLUDE`
//! (ADR-0047).

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=DEP_OCCT_INCLUDE");
    let occt_include = PathBuf::from(std::env::var_os("DEP_OCCT_INCLUDE").expect(
        "DEP_OCCT_INCLUDE not set – is the opencascade-sys fork (links = \"occt\") patched in?",
    ));
    assert!(
        occt_include.join("TopoDS_Shape.hxx").exists(),
        "OCCT headers not found in {} (DEP_OCCT_INCLUDE from opencascade-sys)",
        occt_include.display()
    );

    cxx_build::bridge("src/occt.rs")
        .file("cpp/qoob3d_occt.cpp")
        .include("cpp")
        .include(&occt_include)
        .std("c++17")
        .flag_if_supported("-Wno-deprecated-declarations")
        .compile("qoob3d-occt");

    for path in ["src/occt.rs", "cpp/qoob3d_occt.cpp", "cpp/qoob3d_occt.hxx"] {
        println!("cargo:rerun-if-changed={path}");
    }
}
