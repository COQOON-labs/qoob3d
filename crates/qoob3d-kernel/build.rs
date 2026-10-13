//! Compiles QOOB3D's own OCCT wrappers (ADR-0014) against the OCCT that
//! `opencascade-sys` (feature `builtin`) has already built into target/OCCT.

fn main() {
    let occt_include = occt_sys::occt_path().join("include");
    assert!(
        occt_include.join("TopoDS_Shape.hxx").exists(),
        "OCCT headers not found in {} – is opencascade-sys built with `builtin`?",
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
