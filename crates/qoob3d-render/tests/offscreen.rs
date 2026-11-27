//! Headless smoke test: renders a cube offscreen and checks the pixels.
//! Skipped when no GPU adapter is available, unless QOOB3D_REQUIRE_GPU is set.
//! Set `QOOB3D_TEST_IMAGE_DIR` to also write the frame as PNG.

use qoob3d_core::{ShapeMesh, TriMesh};
use qoob3d_render::headless::{HeadlessGpu, read_rgba8};
use qoob3d_render::{COLOR_FORMAT, Camera, SceneOptions, ViewportRenderer, wgpu};

const W: u32 = 320;
const H: u32 = 200;

/// The scene rendered without grid, fitted; `None` without a GPU.
fn rendered(scene: &ShapeMesh, name: &str) -> Option<Vec<u8>> {
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
            return None;
        }
    };
    let (device, queue) = (&gpu.device, &gpu.queue);
    assert_eq!(COLOR_FORMAT, wgpu::TextureFormat::Rgba8UnormSrgb);

    let mut renderer = ViewportRenderer::new(device);
    renderer.set_scene(device, scene);
    let mut camera = Camera::default();
    camera.fit(scene.faces.bounds().expect("a scene with faces"), W as f32 / H as f32);

    let (view, recreated) = renderer.render(
        device,
        queue,
        [W, H],
        &camera,
        &SceneOptions { show_grid: false, ..SceneOptions::new(None) },
    );
    assert!(recreated);
    let pixels = read_rgba8(device, queue, view.texture());
    if let Ok(dir) = std::env::var("QOOB3D_TEST_IMAGE_DIR") {
        write_png(&format!("{dir}/{name}.png"), &pixels);
    }
    Some(pixels)
}

fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * W + x) * 4) as usize;
    [pixels[i], pixels[i + 1], pixels[i + 2]]
}

#[test]
fn cube_is_drawn_over_background() {
    let scene = ShapeMesh { faces: cube(20.0), ..Default::default() };
    let Some(pixels) = rendered(&scene, "offscreen_cube") else { return };
    let center = pixel(&pixels, W / 2, H / 2);
    let corner = pixel(&pixels, 2, 2);
    let luminance = |p: [u8; 3]| p.iter().map(|&c| c as u32).sum::<u32>();

    assert!(luminance(corner) < 3 * 80, "background should be dark, got {corner:?}");
    assert!(
        luminance(center) > luminance(corner) + 3 * 40,
        "cube should be clearly brighter than background: center {center:?}, corner {corner:?}"
    );
}

/// AG-PJ-07: a part that declares a colour is drawn in it; the theme's body
/// colour stays for the others.
#[test]
fn a_tinted_part_is_drawn_in_its_colour() {
    let plain = ShapeMesh { faces: cube(20.0), ..Default::default() };
    let red = plain.clone().tinted(Some([0xC0, 0x20, 0x20]));
    let (Some(plain), Some(red)) =
        (rendered(&plain, "offscreen_plain"), rendered(&red, "offscreen_red"))
    else {
        return;
    };
    let [r, g, b] = pixel(&red, W / 2, H / 2).map(i32::from);
    assert!(r > g + 60 && r > b + 60, "the tinted cube is red: {:?}", [r, g, b]);
    let [r, g, b] = pixel(&plain, W / 2, H / 2).map(i32::from);
    assert!(
        (r - g).abs() < 40 && r < b + 40,
        "the plain cube keeps the theme colour: {:?}",
        [r, g, b]
    );
    assert_eq!(pixel(&red, 2, 2), pixel(&plain, 2, 2), "the background is the same");
}

fn write_png(path: &str, rgba: &[u8]) {
    let file = std::io::BufWriter::new(std::fs::File::create(path).expect("create png"));
    let mut enc = png::Encoder::new(file, W, H);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().expect("png header").write_image_data(rgba).expect("png data");
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
