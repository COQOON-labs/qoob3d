//! Window-less GPU access for tests and the `--render` CLI (ADR-0013).

use crate::wgpu;

#[derive(Debug, thiserror::Error)]
pub enum HeadlessError {
    #[error("no suitable GPU adapter: {0}")]
    Adapter(String),
    #[error("could not create GPU device: {0}")]
    Device(String),
}

/// A wgpu device and queue without any window or surface.
pub struct HeadlessGpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl HeadlessGpu {
    pub fn new() -> Result<Self, HeadlessError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
            .map_err(|e| HeadlessError::Adapter(e.to_string()))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))
            .map_err(|e| HeadlessError::Device(e.to_string()))?;
        Ok(Self { device, queue })
    }
}

/// Copies an RGBA8 texture back to the CPU as tightly packed rows.
///
/// Blocks until the GPU is done. The texture needs `COPY_SRC` usage.
pub fn read_rgba8(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<u8> {
    let (width, height) = (texture.width(), texture.height());
    let row_bytes = width * 4;
    let padded_row_bytes = row_bytes.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);

    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("qoob3d readback"),
        size: u64::from(padded_row_bytes * height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row_bytes),
                rows_per_image: Some(height),
            },
        },
        texture.size(),
    );
    queue.submit([encoder.finish()]);

    buffer.slice(..).map_async(wgpu::MapMode::Read, |r| r.expect("readback map failed"));
    device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU poll failed");

    let mapped = buffer.slice(..).get_mapped_range().expect("readback buffer not mapped");
    let mut out = Vec::with_capacity((row_bytes * height) as usize);
    for row in mapped.chunks_exact(padded_row_bytes as usize) {
        out.extend_from_slice(&row[..row_bytes as usize]);
    }
    drop(mapped);
    buffer.unmap();
    out
}
