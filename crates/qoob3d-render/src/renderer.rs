use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use qoob3d_core::{BedGeometry, ShapeMesh};

use crate::camera::{Camera, Projection};
use crate::wgpu::{self, util::DeviceExt};

/// Render target format: sRGB, so shading, blending and the MSAA resolve
/// happen in linear space (ADR-0019).
pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
/// Format of the view handed to egui. egui treats texture samples as
/// gamma-encoded (its own textures are `Rgba8Unorm`), so it must read the
/// stored sRGB bytes *without* decoding them (ADR-0019).
pub const DISPLAY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SAMPLE_COUNT: u32 = 4;

/// Linear-space colours of the calm dark theme (docs/design/theme.md).
mod palette {
    pub(super) const BG_TOP: [f32; 4] = [0.032, 0.036, 0.042, 1.0];
    pub(super) const BG_BOTTOM: [f32; 4] = [0.010, 0.011, 0.013, 1.0];
    pub(super) const BODY: [f32; 4] = [0.50, 0.53, 0.57, 1.0];
    pub(super) const EDGE: [f32; 4] = [0.006, 0.007, 0.008, 1.0];
    pub(super) const GRID_MINOR: [f32; 4] = [0.09, 0.10, 0.11, 0.35];
    pub(super) const GRID_MAJOR: [f32; 4] = [0.14, 0.15, 0.17, 0.6];
    pub(super) const PLATE_FILL: [f32; 4] = [0.035, 0.040, 0.046, 0.65];
    pub(super) const PLATE_BORDER: [f32; 4] = [0.20, 0.22, 0.24, 0.9];
    pub(super) const AXIS_X: [f32; 4] = [0.55, 0.06, 0.05, 0.9];
    pub(super) const AXIS_Y: [f32; 4] = [0.08, 0.40, 0.10, 0.9];
    pub(super) const AXIS_Z: [f32; 4] = [0.08, 0.20, 0.65, 0.9];
    /// Warning amber (#F5A623) for non-printable / single-nozzle zones.
    pub(super) const RESTRICTED: [f32; 4] = [0.91, 0.38, 0.02, 1.0];
    pub(super) const BUILD_VOLUME: [f32; 4] = [0.30, 0.33, 0.37, 0.55];
}

/// Capacity of the bed polygon uniform; must match `bed.points` in the
/// shader and `MAX_POINTS` in `xtask printers`.
pub const MAX_BED_POINTS: usize = 192;

/// Width (points) and depth offset (fraction of camera distance) of lines.
const EDGE_STYLE: [f32; 2] = [1.3, 0.0015];
const AXIS_STYLE: [f32; 2] = [1.6, 0.0];
const VOLUME_STYLE: [f32; 2] = [1.0, 0.0];

/// What to draw besides the geometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneOptions<'a> {
    pub show_grid: bool,
    /// Build plate in world coordinates, centred on the origin (ADR-0021).
    pub bed: Option<&'a BedGeometry>,
    /// Wireframe of the build volume up to the printer's max height.
    pub show_build_volume: bool,
    /// UI scale: line widths are specified in points.
    pub pixels_per_point: f32,
    /// Plane the grid lies in (active sketch plane); `None` = world XY.
    /// The build plate is only shown on world XY.
    pub grid_plane: Option<GridPlane>,
}

impl<'a> SceneOptions<'a> {
    pub fn new(bed: Option<&'a BedGeometry>) -> Self {
        Self {
            show_grid: true,
            bed,
            show_build_volume: false,
            pixels_per_point: 1.0,
            grid_plane: None,
        }
    }
}

/// A plane frame for the grid: origin, in-plane axes u/v, normal n.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridPlane {
    pub origin: Vec3,
    pub u: Vec3,
    pub v: Vec3,
    pub n: Vec3,
}

impl GridPlane {
    pub const WORLD_XY: GridPlane =
        GridPlane { origin: Vec3::ZERO, u: Vec3::X, v: Vec3::Y, n: Vec3::Z };

    fn is_world_xy(&self) -> bool {
        self.origin.abs_diff_eq(Vec3::ZERO, 1e-6)
            && self.u.abs_diff_eq(Vec3::X, 1e-6)
            && self.v.abs_diff_eq(Vec3::Y, 1e-6)
    }
}

/// Colour of a grid axis: the world axis it runs along, else neutral.
fn axis_colour(dir: Vec3) -> [f32; 4] {
    let d = dir.normalize_or_zero();
    if d.x.abs() > 0.99 {
        palette::AXIS_X
    } else if d.y.abs() > 0.99 {
        palette::AXIS_Y
    } else if d.z.abs() > 0.99 {
        palette::AXIS_Z
    } else {
        [0.3, 0.3, 0.3, 0.8]
    }
}

/// Bed polygons for the grid shader: printable area, exclude zone and up to
/// two nozzle areas, stored back to back (`counts` gives the lengths).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BedUniform {
    counts: [u32; 4],
    points: [[f32; 4]; MAX_BED_POINTS],
}

impl BedUniform {
    fn new(bed: Option<&BedGeometry>) -> Self {
        let mut u = Self { counts: [0; 4], points: [[0.0; 4]; MAX_BED_POINTS] };
        let Some(bed) = bed else { return u };
        let empty = Vec::new();
        let polys = [
            &bed.printable,
            &bed.exclude,
            bed.extruder_areas.first().unwrap_or(&empty),
            bed.extruder_areas.get(1).unwrap_or(&empty),
        ];
        let mut next = 0;
        for (slot, poly) in polys.iter().enumerate() {
            let n = poly.len().min(MAX_BED_POINTS - next);
            for (k, p) in poly.iter().take(n).enumerate() {
                u.points[next + k] = [p[0], p[1], 0.0, 0.0];
            }
            u.counts[slot] = n as u32;
            next += n;
        }
        u
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    cam_right: [f32; 4],
    cam_up: [f32; 4],
    cam_forward: [f32; 4],
    orbit_target: [f32; 4],
    bg_top: [f32; 4],
    bg_bottom: [f32; 4],
    body_color: [f32; 4],
    grid: [f32; 4],
    viewport: [f32; 4],
    grid_minor: [f32; 4],
    grid_major: [f32; 4],
    plate_fill: [f32; 4],
    plate_border: [f32; 4],
    axis_x: [f32; 4],
    axis_y: [f32; 4],
    restricted: [f32; 4],
    grid_origin: [f32; 4],
    grid_u: [f32; 4],
    grid_v: [f32; 4],
    grid_n: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
}

/// One line segment, expanded to a screen-space quad in the vertex shader.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LineInstance {
    a: [f32; 3],
    b: [f32; 3],
    color: [f32; 4],
    style: [f32; 2],
}

struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
}

struct GpuLines {
    instances: wgpu::Buffer,
    count: u32,
}

struct Targets {
    size: [u32; 2],
    msaa_color: wgpu::TextureView,
    depth: wgpu::TextureView,
    /// sRGB view, MSAA resolve target.
    resolved: wgpu::TextureView,
    /// Unorm view of the same texture for egui (ADR-0019).
    display: wgpu::TextureView,
}

/// Draws the viewport scene into its own texture (ADR-0019).
pub struct ViewportRenderer {
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    background_pipeline: wgpu::RenderPipeline,
    mesh_pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    grid_pipeline: wgpu::RenderPipeline,
    mesh: Option<GpuMesh>,
    edges: Option<GpuLines>,
    /// Z axis; rewritten every frame because its length follows the plate.
    axis_z: GpuLines,
    bed_buffer: wgpu::Buffer,
    /// Bed and volume flag the GPU data was built for.
    bed_state: Option<(BedGeometry, bool)>,
    build_volume: Option<GpuLines>,
    targets: Option<Targets>,
}

impl ViewportRenderer {
    #[expect(clippy::too_many_lines, reason = "predates ADR-0040; split when next changed")]
    pub fn new(device: &wgpu::Device) -> Self {
        use wgpu::CompareFunction::{Always, Greater, GreaterEqual};
        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/scene.wgsl"));

        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("qoob3d globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bed_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("qoob3d bed"),
            size: std::mem::size_of::<BedUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("qoob3d globals"),
            entries: &[0, 1].map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }),
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("qoob3d globals"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: bed_buffer.as_entire_binding() },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("qoob3d scene"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = |label: &str,
                        vs: &str,
                        fs: &str,
                        buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
                        depth_write: bool,
                        depth_compare: wgpu::CompareFunction,
                        blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers,
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: wgpu::FrontFace::Ccw,
                    // No culling: open shells and section views must stay visible,
                    // and line quads may wind either way.
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(depth_compare),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState { count: SAMPLE_COUNT, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: COLOR_FORMAT,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };

        let alpha = Some(wgpu::BlendState::ALPHA_BLENDING);
        let background_pipeline = pipeline(
            "qoob3d background",
            "vs_background",
            "fs_background",
            &[],
            false,
            Always,
            None,
        );
        let mesh_pipeline = pipeline(
            "qoob3d mesh",
            "vs_mesh",
            "fs_mesh",
            &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3],
            })],
            true,
            Greater, // reverse-Z
            None,
        );
        let line_pipeline = pipeline(
            "qoob3d lines",
            "vs_line",
            "fs_line",
            &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<LineInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array![
                    0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32x2
                ],
            })],
            true,
            GreaterEqual,
            alpha,
        );
        // The grid is transparent: tested against bodies, never occludes them.
        let grid_pipeline =
            pipeline("qoob3d grid", "vs_grid", "fs_grid", &[], false, GreaterEqual, alpha);

        let axis_z = GpuLines {
            instances: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("qoob3d z axis"),
                size: std::mem::size_of::<LineInstance>() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            count: 1,
        };

        Self {
            globals,
            bind_group,
            background_pipeline,
            mesh_pipeline,
            line_pipeline,
            grid_pipeline,
            mesh: None,
            edges: None,
            axis_z,
            bed_buffer,
            bed_state: None,
            build_volume: None,
            targets: None,
        }
    }

    /// Replaces the displayed geometry (faces and feature edges).
    pub fn set_scene(&mut self, device: &wgpu::Device, scene: &ShapeMesh) {
        let faces = &scene.faces;
        self.mesh = (!faces.is_empty()).then(|| {
            let vertices: Vec<Vertex> = faces
                .positions
                .iter()
                .zip(&faces.normals)
                .map(|(&position, &normal)| Vertex { position, normal })
                .collect();
            GpuMesh {
                vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("qoob3d mesh vertices"),
                    contents: bytemuck::cast_slice(&vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                }),
                indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("qoob3d mesh indices"),
                    contents: bytemuck::cast_slice(&faces.indices),
                    usage: wgpu::BufferUsages::INDEX,
                }),
                index_count: faces.indices.len() as u32,
            }
        });

        let segments: Vec<LineInstance> = scene
            .edges
            .segments()
            .map(|[a, b]| LineInstance { a, b, color: palette::EDGE, style: EDGE_STYLE })
            .collect();
        self.edges = (!segments.is_empty()).then(|| GpuLines {
            instances: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("qoob3d edges"),
                contents: bytemuck::cast_slice(&segments),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            count: segments.len() as u32,
        });
    }

    /// Renders the scene at `size` physical pixels and returns the view to
    /// display in egui (gamma-encoded, see [`DISPLAY_FORMAT`]) plus whether
    /// it was (re)created this call, in which case the caller must
    /// re-register it with the UI. `view.texture()` gives the sRGB bytes for
    /// readback.
    #[expect(clippy::too_many_lines, reason = "predates ADR-0040; split when next changed")]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: [u32; 2],
        camera: &Camera,
        options: &SceneOptions<'_>,
    ) -> (&wgpu::TextureView, bool) {
        let size = [size[0].max(1), size[1].max(1)];
        let recreated = self.targets.as_ref().is_none_or(|t| t.size != size);
        if recreated {
            self.targets = Some(create_targets(device, size));
        }

        let aspect = size[0] as f32 / size[1] as f32;
        let view_proj = camera.view_projection(aspect);
        self.update_bed(device, queue, options);
        let plate_size = options.bed.map_or([0.0, 0.0], |b| bed_size(&b.printable));
        let v4 = |v: Vec3, w: f32| v.extend(w).to_array();
        let plane = options.grid_plane.unwrap_or(GridPlane::WORLD_XY);
        let show_plate = options.bed.is_some() && plane.is_world_xy();
        let globals = Globals {
            view_proj: view_proj.to_cols_array_2d(),
            inv_view_proj: view_proj.inverse().to_cols_array_2d(),
            eye: v4(
                camera.eye(),
                if camera.projection == Projection::Perspective { 1.0 } else { 0.0 },
            ),
            cam_right: v4(camera.right(), 0.0),
            cam_up: v4(camera.up(), 0.0),
            cam_forward: v4(camera.forward(), 0.0),
            orbit_target: v4(camera.target, camera.distance),
            bg_top: palette::BG_TOP,
            bg_bottom: palette::BG_BOTTOM,
            body_color: palette::BODY,
            grid: [
                plate_size[0],
                plate_size[1],
                f32::from(u8::from(options.show_grid)),
                f32::from(u8::from(show_plate)),
            ],
            viewport: [size[0] as f32, size[1] as f32, options.pixels_per_point.max(0.5), 0.0],
            grid_minor: palette::GRID_MINOR,
            grid_major: palette::GRID_MAJOR,
            plate_fill: palette::PLATE_FILL,
            plate_border: palette::PLATE_BORDER,
            axis_x: axis_colour(plane.u),
            axis_y: axis_colour(plane.v),
            restricted: palette::RESTRICTED,
            grid_origin: v4(plane.origin, 1.0),
            grid_u: v4(plane.u, 0.0),
            grid_v: v4(plane.v, 0.0),
            grid_n: v4(plane.n, 0.0),
        };
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));

        let z_len =
            if options.bed.is_some() { 0.25 * plate_size[0].max(plate_size[1]) } else { 50.0 };
        let axis = LineInstance {
            a: [0.0; 3],
            b: [0.0, 0.0, z_len],
            color: palette::AXIS_Z,
            style: AXIS_STYLE,
        };
        queue.write_buffer(&self.axis_z.instances, 0, bytemuck::bytes_of(&axis));

        let targets = self.targets.as_ref().expect("targets created above");
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("qoob3d viewport"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("qoob3d viewport"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.msaa_color,
                    depth_slice: None,
                    resolve_target: Some(&targets.resolved),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0), // reverse-Z: 0 = far
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.bind_group, &[]);

            pass.set_pipeline(&self.background_pipeline);
            pass.draw(0..3, 0..1);

            if let Some(mesh) = &self.mesh {
                pass.set_pipeline(&self.mesh_pipeline);
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.index_count, 0, 0..1);
            }

            pass.set_pipeline(&self.line_pipeline);
            let axis = options.show_grid.then_some(&self.axis_z);
            for lines in self.edges.iter().chain(axis).chain(self.build_volume.as_ref()) {
                pass.set_vertex_buffer(0, lines.instances.slice(..));
                pass.draw(0..6, 0..lines.count);
            }

            if options.show_grid || options.bed.is_some() {
                pass.set_pipeline(&self.grid_pipeline);
                pass.draw(0..3, 0..1);
            }
        }
        queue.submit([encoder.finish()]);

        (&targets.display, recreated)
    }
}

impl ViewportRenderer {
    /// Uploads bed polygons and rebuilds the build-volume wireframe when the
    /// printer or the volume toggle changed.
    fn update_bed(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        options: &SceneOptions<'_>,
    ) {
        let wanted = options.bed.map(|b| (b.clone(), options.show_build_volume));
        if self.bed_state == wanted {
            return;
        }
        queue.write_buffer(&self.bed_buffer, 0, bytemuck::bytes_of(&BedUniform::new(options.bed)));
        let segments = match options.bed {
            Some(bed) if options.show_build_volume => build_volume_segments(bed),
            _ => Vec::new(),
        };
        self.build_volume = (!segments.is_empty()).then(|| GpuLines {
            instances: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("qoob3d build volume"),
                contents: bytemuck::cast_slice(&segments),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            count: segments.len() as u32,
        });
        self.bed_state = wanted;
    }
}

fn bed_size(poly: &[[f32; 2]]) -> [f32; 2] {
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in poly {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    if poly.is_empty() { [0.0; 2] } else { [hi[0] - lo[0], hi[1] - lo[1]] }
}

/// Top outline of the printable area at max height plus vertical edges (at
/// every corner of simple shapes, at four extremes of round ones).
fn build_volume_segments(bed: &BedGeometry) -> Vec<LineInstance> {
    let line = |a: [f32; 3], b: [f32; 3]| LineInstance {
        a,
        b,
        color: palette::BUILD_VOLUME,
        style: VOLUME_STYLE,
    };
    let h = bed.max_height;
    let pts = &bed.printable;
    let n = pts.len();
    let mut out: Vec<LineInstance> = (0..n)
        .map(|i| {
            let (p, q) = (pts[i], pts[(i + 1) % n]);
            line([p[0], p[1], h], [q[0], q[1], h])
        })
        .collect();
    let corners: Vec<[f32; 2]> = if n <= 16 {
        pts.clone()
    } else {
        let pick = |key: fn(&[f32; 2]) -> f32, max: bool| {
            *pts.iter()
                .max_by(|a, b| {
                    let o = key(a).total_cmp(&key(b));
                    if max { o } else { o.reverse() }
                })
                .expect("non-empty")
        };
        vec![
            pick(|p| p[0], true),
            pick(|p| p[0], false),
            pick(|p| p[1], true),
            pick(|p| p[1], false),
        ]
    };
    out.extend(corners.iter().map(|p| line([p[0], p[1], 0.0], [p[0], p[1], h])));
    out
}

fn create_targets(device: &wgpu::Device, size: [u32; 2]) -> Targets {
    let extent = wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 };
    let make = |label, format, samples, usage| {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: extent,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
            .create_view(&Default::default())
    };
    let resolved_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("qoob3d resolved color"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: COLOR_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC, // readback in tests and --render
        view_formats: &[DISPLAY_FORMAT],
    });
    let resolved = resolved_texture.create_view(&Default::default());
    let display = resolved_texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("qoob3d display view"),
        format: Some(DISPLAY_FORMAT),
        ..Default::default()
    });
    Targets {
        size,
        msaa_color: make(
            "qoob3d msaa color",
            COLOR_FORMAT,
            SAMPLE_COUNT,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        ),
        depth: make(
            "qoob3d depth",
            DEPTH_FORMAT,
            SAMPLE_COUNT,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        ),
        resolved,
        display,
    }
}
