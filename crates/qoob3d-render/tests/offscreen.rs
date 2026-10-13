//! Headless smoke test: renders a cube offscreen and checks the pixels.
//! Skipped when no GPU adapter is available, unless QOOB3D_REQUIRE_GPU is set.
//! Set `QOOB3D_TEST_IMAGE_DIR` to also write the frame as PNG.

use qoob3d_core::{ShapeMesh, TriMesh};
use qoob3d_render::headless::{HeadlessGpu, read_rgba8};
use qoob3d_render::{COLOR_FORMAT, Camera, SceneOptions, ViewportRenderer, wgpu};

const W: u32 = 320;
const H: u32 = 200;

#[test]
fn cube_is_drawn_over_background() {
    let gpu = match HeadlessGpu::new() {
        Ok(gpu) => gpu,
        // `cargo xtask check` sets QOOB3D_REQUIRE_GPU so the test cannot pass
        // silently there (ADR-0013); elsewhere (e.g. CI without a software
        // rasteriser) a missing GPU skips it.
        Err(err) if std::env::var_os("QOOB3D_REQUIRE_GPU").is_some() => {
            panic!("GPU required but unavailable: {err}")
        }
        Err(err) => {
            eprintln!("skipping: {err}");
            return;
        }
    };
    let (device, queue) = (&gpu.device, &gpu.queue);
    assert_eq!(COLOR_FORMAT, wgpu::TextureFormat::Rgba8UnormSrgb);

    let mesh = cube(20.0);
    let mut renderer = ViewportRenderer::new(device);
    renderer.set_scene(device, &ShapeMesh { faces: mesh.clone(), ..Default::default() });
    let mut camera = Camera::default();
    camera.fit(mesh.bounds().unwrap(), W as f32 / H as f32);

    let (view, recreated) = renderer.render(
        device,
        queue,
        [W, H],
        &camera,
        &SceneOptions {
            show_grid: false,
            bed: None,
            show_build_volume: false,
            pixels_per_point: 1.0,
        },
    );
    assert!(recreated);
    let pixels = read_rgba8(device, queue, view.texture());

    if let Ok(dir) = std::env::var("QOOB3D_TEST_IMAGE_DIR") {
        write_png(&format!("{dir}/offscreen_cube.png"), &pixels);
    }

    let px = |x: u32, y: u32| {
        let i = ((y * W + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2]]
    };
    let center = px(W / 2, H / 2);
    let corner = px(2, 2);
    let luminance = |p: [u8; 3]| p.iter().map(|&c| c as u32).sum::<u32>();

    assert!(luminance(corner) < 3 * 80, "background should be dark, got {corner:?}");
    assert!(
        luminance(center) > luminance(corner) + 3 * 40,
        "cube should be clearly brighter than background: center {center:?}, corner {corner:?}"
    );
}

fn write_png(path: &str, rgba: &[u8]) {
    let file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let mut enc = png::Encoder::new(file, W, H);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(rgba).unwrap();
}

/// Axis-aligned cube on the XY plane with flat normals, CCW from outside.
fn cube(size: f32) -> TriMesh {
    let h = size / 2.0;
    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
        ([0.0, 0.0, 1.0], [[-h, -h, size], [h, -h, size], [h, h, size], [-h, h, size]]),
        ([0.0, 0.0, -1.0], [[-h, -h, 0.0], [-h, h, 0.0], [h, h, 0.0], [h, -h, 0.0]]),
        ([1.0, 0.0, 0.0], [[h, -h, 0.0], [h, h, 0.0], [h, h, size], [h, -h, size]]),
        ([-1.0, 0.0, 0.0], [[-h, -h, 0.0], [-h, -h, size], [-h, h, size], [-h, h, 0.0]]),
        ([0.0, 1.0, 0.0], [[-h, h, 0.0], [-h, h, size], [h, h, size], [h, h, 0.0]]),
        ([0.0, -1.0, 0.0], [[-h, -h, 0.0], [h, -h, 0.0], [h, -h, size], [-h, -h, size]]),
    ];
    let mut mesh = TriMesh::default();
    for (normal, quad) in faces {
        let base = mesh.positions.len() as u32;
        mesh.positions.extend(quad);
        mesh.normals.extend([normal; 4]);
        mesh.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    mesh
}
