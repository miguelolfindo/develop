//! Histogram and vectorscope without any CPU readback.
//!
//! Each frame the finished picture is rendered small (about its third mip level) into an
//! off-screen texture, a compute pass counts its pixels into bins with `atomicAdd`, and the
//! two scopes are drawn in egui's render pass by a fragment shader that reads the bins.

use std::sync::atomic::{AtomicBool, Ordering};

use eframe::{egui, egui_wgpu, wgpu};

use crate::pipeline::ImageRenderer;

/// Size of the small rendering the scopes are measured from. A 61 MP frame is about 12x
/// larger each way, so this sits between its third and fourth mip level.
const SOURCE_SIZE: [u32; 2] = [768, 512];
/// 4 x 256 histogram bins + a 128 x 128 Cb/Cr grid.
const BINS: usize = 4 * 256 + 128 * 128;

/// Vectorscope scale and colour targets (75 % bars as Cb, Cr); keep in step with scopes.wgsl.
pub const RING: f32 = 0.92;
pub const RING_CHROMA: f32 = 0.45;
pub const TARGETS: [(&str, f32, f32); 6] =
    [("R", -0.0859, 0.3750), ("YL", -0.3750, 0.0344), ("G", -0.2891, -0.3406), ("CY", 0.0859, -0.3750), ("B", 0.3750, -0.0344), ("MG", 0.2891, 0.3406)];

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Histogram,
    Vectorscope,
}

pub struct Scopes {
    source_view: wgpu::TextureView,
    bins: wgpu::Buffer,
    count_pipeline: wgpu::ComputePipeline,
    count_group: wgpu::BindGroup,
    draw_pipeline: wgpu::RenderPipeline,
    draw_groups: [wgpu::BindGroup; 2],
    draw_uniforms: [wgpu::Buffer; 2],
    target_is_linear_free: bool,
    /// Set by the app when the picture or its settings may have changed; the next scope
    /// drawn recounts.
    pending: AtomicBool,
}

impl Scopes {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scopes shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("scopes.wgsl").into()),
        });
        let source = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("scope source"),
            size: wgpu::Extent3d { width: SOURCE_SIZE[0], height: SOURCE_SIZE[1], depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::pipeline::SCOPE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let source_view = source.create_view(&wgpu::TextureViewDescriptor::default());
        let bins = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scope bins"),
            size: (BINS * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Counting: the small picture in, the bins out.
        let count_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scope count layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let count_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("scope count"),
            layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("scope count layout"),
                bind_group_layouts: &[&count_layout],
                push_constant_ranges: &[],
            })),
            module: &shader,
            entry_point: "count",
        });
        let count_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scope count group"),
            layout: &count_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&source_view) },
                wgpu::BindGroupEntry { binding: 1, resource: bins.as_entire_binding() },
            ],
        });

        // Drawing: the bins in, read-only.
        let draw_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scope draw layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let draw_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scope draw"),
            layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("scope draw layout"),
                bind_group_layouts: &[&draw_layout],
                push_constant_ranges: &[],
            })),
            vertex: wgpu::VertexState { module: &shader, entry_point: "vs_scope", buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_scope",
                targets: &[Some(wgpu::ColorTargetState { format: target_format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });
        let uniform = |label: &str| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let draw_uniforms = [uniform("histogram uniforms"), uniform("vectorscope uniforms")];
        let group = |buffer: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("scope draw group"),
                layout: &draw_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 2, resource: bins.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: buffer.as_entire_binding() },
                ],
            })
        };
        let draw_groups = [group(&draw_uniforms[0]), group(&draw_uniforms[1])];

        Self {
            source_view,
            bins,
            count_pipeline,
            count_group,
            draw_pipeline,
            draw_groups,
            draw_uniforms,
            target_is_linear_free: !target_format.is_srgb(),
            pending: AtomicBool::new(false),
        }
    }

    /// Ask for a recount before the scopes are next drawn.
    pub fn request(&self) {
        self.pending.store(true, Ordering::Relaxed);
    }

    /// Tell a scope the shape of the rectangle it is drawn into.
    pub fn set_shape(&self, queue: &wgpu::Queue, kind: Kind, aspect: f32) {
        let i = kind as usize;
        let info = [i as f32, (SOURCE_SIZE[0] * SOURCE_SIZE[1]) as f32, if self.target_is_linear_free { 1.0 } else { 0.0 }, aspect];
        queue.write_buffer(&self.draw_uniforms[i], 0, bytemuck::cast_slice(&info));
    }
}

/// Paint callback for one scope.
pub struct ScopeCallback(pub Kind);

impl egui_wgpu::CallbackTrait for ScopeCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let (Some(scopes), Some(image)) = (resources.get::<Scopes>(), resources.get::<ImageRenderer>()) else { return Vec::new() };
        if !scopes.pending.swap(false, Ordering::Relaxed) {
            return Vec::new();
        }
        // Empty the bins, render the picture small, count it.
        queue.write_buffer(&scopes.bins, 0, &vec![0u8; BINS * 4]);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("scopes") });
        image.render_scope_source(&mut encoder, &scopes.source_view);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("scope count"), timestamp_writes: None });
            pass.set_pipeline(&scopes.count_pipeline);
            pass.set_bind_group(0, &scopes.count_group, &[]);
            pass.dispatch_workgroups(SOURCE_SIZE[0].div_ceil(8), SOURCE_SIZE[1].div_ceil(8), 1);
        }
        vec![encoder.finish()]
    }

    fn paint<'a>(&'a self, _info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'a>, resources: &'a egui_wgpu::CallbackResources) {
        if let Some(scopes) = resources.get::<Scopes>() {
            pass.set_pipeline(&scopes.draw_pipeline);
            pass.set_bind_group(0, &scopes.draw_groups[self.0 as usize], &[]);
            pass.draw(0..3, 0..1);
        }
    }
}
