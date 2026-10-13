use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec4};
use qoob3d_core::TriMesh;

use crate::camera::Camera;
use crate::wgpu::{self, util::DeviceExt};

/// Format of the texture handed to the UI. sRGB so egui samples it exactly
/// like its own (sRGB) textures.
pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SAMPLE_COUNT: u32 = 4;

/// Linear-space colours of the calm dark theme.
mod palette {
    pub const BG_TOP: [f32; 4] = [0.032, 0.036, 0.042, 1.0];
    pub const BG_BOTTOM: [f32; 4] = [0.010, 0.011, 0.013, 1.0];
    pub const BODY: [f32; 4] = [0.50, 0.53, 0.57, 1.0];
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    bg_top: [f32; 4],
    bg_bottom: [f32; 4],
    body_color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
}

struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
}

struct Targets {
    size: [u32; 2],
    msaa_color: wgpu::TextureView,
    depth: wgpu::TextureView,
    resolved: wgpu::TextureView,
}

/// Draws the viewport scene into its own texture.
pub struct ViewportRenderer {
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    background_pipeline: wgpu::RenderPipeline,
    mesh_pipeline: wgpu::RenderPipeline,
    mesh: Option<GpuMesh>,
    targets: Option<Targets>,
}

impl ViewportRenderer {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/scene.wgsl"));

        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("qoob3d globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("qoob3d globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("qoob3d globals"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("qoob3d scene"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let multisample = wgpu::MultisampleState { count: SAMPLE_COUNT, ..Default::default() };
        let color_target = [Some(wgpu::ColorTargetState::from(COLOR_FORMAT))];

        let background_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("qoob3d background"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_background"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_background"),
                compilation_options: Default::default(),
                targets: &color_target,
            }),
            multiview_mask: None,
            cache: None,
        });

        let mesh_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("qoob3d mesh"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_mesh"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3],
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                // No culling: open shells and section views must stay visible.
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Greater), // reverse-Z
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_mesh"),
                compilation_options: Default::default(),
                targets: &color_target,
            }),
            multiview_mask: None,
            cache: None,
        });

        Self { globals, bind_group, background_pipeline, mesh_pipeline, mesh: None, targets: None }
    }

    /// Replaces the displayed geometry.
    pub fn set_mesh(&mut self, device: &wgpu::Device, mesh: &TriMesh) {
        if mesh.is_empty() {
            self.mesh = None;
            return;
        }
        let vertices: Vec<Vertex> = mesh
            .positions
            .iter()
            .zip(&mesh.normals)
            .map(|(&position, &normal)| Vertex { position, normal })
            .collect();
        self.mesh = Some(GpuMesh {
            vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("qoob3d mesh vertices"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("qoob3d mesh indices"),
                contents: bytemuck::cast_slice(&mesh.indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
            index_count: mesh.indices.len() as u32,
        });
    }

    /// Renders the scene at `size` physical pixels and returns the resolved
    /// colour texture plus whether it was (re)created this call, in which
    /// case the caller must re-register it with the UI.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: [u32; 2],
        camera: &Camera,
    ) -> (&wgpu::TextureView, bool) {
        let size = [size[0].max(1), size[1].max(1)];
        let recreated = self.targets.as_ref().is_none_or(|t| t.size != size);
        if recreated {
            self.targets = Some(create_targets(device, size));
        }

        let aspect = size[0] as f32 / size[1] as f32;
        let view_proj: Mat4 = camera.projection(aspect) * camera.view();
        let globals = Globals {
            view_proj: view_proj.to_cols_array_2d(),
            eye: Vec4::from((camera.eye(), 1.0)).to_array(),
            bg_top: palette::BG_TOP,
            bg_bottom: palette::BG_BOTTOM,
            body_color: palette::BODY,
        };
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));

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
        }
        queue.submit([encoder.finish()]);

        (&targets.resolved, recreated)
    }
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
        resolved: make(
            "qoob3d resolved color",
            COLOR_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC, // readback in tests, screenshots later
        ),
    }
}
