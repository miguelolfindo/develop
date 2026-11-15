//! wgpu side: the RGBA16F image texture, the camera-profile lookup textures, the uniform
//! buffer, and the render pipeline that draws the image into egui's render pass.

use bytemuck::{Pod, Zeroable};
use eframe::{egui, egui_wgpu, wgpu};

use crate::profile::{GpuTable, ProfileRender, CURVE_LUT_SIZE};
use crate::raw::MipLevel;

/// Mirrors `Params` in shader.wgsl. Everything is a vec4 so there are no padding surprises.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Uniforms {
    pub cam_to_work: [[f32; 4]; 3],
    pub work_to_srgb: [[f32; 4]; 3],
    pub luma: [f32; 4],
    pub wb: [f32; 4],
    pub basic: [f32; 4],
    pub tone: [f32; 4],
    pub view: [f32; 4],
    pub rot: [f32; 4],
    pub rot_off: [f32; 4],
    pub profile: [f32; 4],
    pub hue_sat_dims: [f32; 4],
    pub look_dims: [f32; 4],
    /// Straighten: cos, sin of the angle, picture width / height (w unused).
    pub straighten: [f32; 4],
    /// Crop rectangle (x0, y0, x1, y1) in the straightened frame; outside it nothing is drawn.
    pub crop: [f32; 4],
    /// Sharpening: strength, radius in pixels, halo limit, masking threshold.
    pub sharpen: [f32; 4],
    /// x = input mode: 0 Bayer raw, 1 linear raw (already RGB), 2 finished picture.
    pub input: [f32; 4],
    /// Texture, clarity, dehaze (-1..1).
    pub presence: [f32; 4],
    /// Lens correction (see `lens::Gpu` for what each holds).
    pub lens_dist: [f32; 4],
    pub lens_geo: [f32; 4],
    pub lens_str: [f32; 4],
    pub tca_r: [f32; 4],
    pub tca_b: [f32; 4],
    pub vig: [f32; 4],
    pub vig2: [f32; 4],
    pub fringe: [f32; 4],
    pub knots: [[f32; 4]; 16],
    /// The photo's own display levels, from a small render with the tone sliders at zero:
    /// x = fraction of the frame above display level 176, y = 98th-percentile display
    /// level, z = median display level, w = 99.7th-percentile display level (0..1). The
    /// adaptive tone sliders read these.
    pub scene: [f32; 4],
    /// More of the same: x = 98th percentile of the darkest channel (min of R, G, B) on
    /// screen, the brightness of the photo's brightest near-neutral areas; yzw unused.
    pub scene2: [f32; 4],
}

/// Export renders into half floats and converts on the CPU, so 16-bit output is real 16-bit.
const EXPORT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// The small off-screen picture the scopes are measured from uses the same pipeline.
pub const SCOPE_FORMAT: wgpu::TextureFormat = EXPORT_FORMAT;
/// Export is rendered in square tiles of this size, which bounds GPU memory whatever the
/// picture size. 2048 * 8 bytes per row also satisfies wgpu's 256-byte row alignment.
const EXPORT_TILE: u32 = 2048;

pub struct ImageRenderer {
    pipeline: wgpu::RenderPipeline,
    export_pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    table_sampler: wgpu::Sampler,
    uniform_buf: wgpu::Buffer,
    image_view: wgpu::TextureView,
    hue_sat_view: wgpu::TextureView,
    look_view: wgpu::TextureView,
    curve_view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    /// Settings and bindings for rendering the picture small for the scopes.
    scope_uniform_buf: wgpu::Buffer,
    scope_bind_group: wgpu::BindGroup,
    /// Settings and bindings for the "before" half of the A/B split.
    before_uniform_buf: wgpu::Buffer,
    before_bind_group: wgpu::BindGroup,
    /// True when the surface format is not sRGB, i.e. the shader must encode gamma itself.
    pub shader_encodes_srgb: bool,
}

fn texture_entry(binding: u32, dimension: wgpu::TextureViewDimension) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: dimension,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

impl ImageRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("raw shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("raw bind group layout"),
            entries: &[
                texture_entry(0, wgpu::TextureViewDimension::D2),
                sampler_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Uniforms>() as u64),
                    },
                    count: None,
                },
                texture_entry(3, wgpu::TextureViewDimension::D3),
                texture_entry(4, wgpu::TextureViewDimension::D3),
                texture_entry(5, wgpu::TextureViewDimension::D2),
                sampler_entry(6),
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("raw pipeline layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });

        let make_pipeline = |format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("raw pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: "vs_main", buffers: &[] },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: "fs_main",
                    targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
            })
        };
        // Same shader twice: once for the window, once for export into a 16-bit float target.
        let pipeline = make_pipeline(target_format);
        let export_pipeline = make_pipeline(EXPORT_FORMAT);

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("raw sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        // Hue is cyclic, so the table's hue axis wraps; saturation and value clamp.
        let table_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("profile table sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("raw uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // 1-texel placeholders so the bind group is always valid.
        let image_view = Self::create_image(device, 1, 1, 1).create_view(&Default::default());
        let one_texel = [0u16; 4];
        let hue_sat_view = Self::upload_lut(device, queue, wgpu::TextureDimension::D3, [1, 1, 1], &one_texel);
        let look_view = Self::upload_lut(device, queue, wgpu::TextureDimension::D3, [1, 1, 1], &one_texel);
        let curve_view = Self::upload_lut(device, queue, wgpu::TextureDimension::D2, [1, 1, 1], &one_texel);

        let scope_uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scope uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let before_uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("before uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut this = Self {
            before_bind_group: Self::placeholder_bind_group(device, &layout, &before_uniform_buf),
            before_uniform_buf,
            scope_bind_group: Self::placeholder_bind_group(device, &layout, &scope_uniform_buf),
            scope_uniform_buf,
            bind_group: Self::placeholder_bind_group(device, &layout, &uniform_buf),
            pipeline,
            export_pipeline,
            layout,
            sampler,
            table_sampler,
            uniform_buf,
            image_view,
            hue_sat_view,
            look_view,
            curve_view,
            shader_encodes_srgb: !target_format.is_srgb(),
        };
        this.rebuild_bind_group(device);
        this
    }

    /// The struct needs *a* bind group before the real one can be built from its own fields;
    /// this one is replaced immediately and never drawn with.
    fn placeholder_bind_group(
        device: &wgpu::Device,
        _layout: &wgpu::BindGroupLayout,
        uniform_buf: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform_buf.as_entire_binding() }],
        })
    }

    fn create_image(device: &wgpu::Device, width: u32, height: u32, mips: u32) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("raw image"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    /// Create and fill a small RGBA16F lookup texture (2D or 3D).
    fn upload_lut(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        dimension: wgpu::TextureDimension,
        dims: [u32; 3],
        rgba: &[u16],
    ) -> wgpu::TextureView {
        let size = wgpu::Extent3d { width: dims[0], height: dims[1], depth_or_array_layers: dims[2] };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("profile lookup"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(rgba),
            wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(dims[0] * 8), rows_per_image: Some(dims[1]) },
            size,
        );
        texture.create_view(&wgpu::TextureViewDescriptor::default())
    }

    fn make_bind_group(&self, device: &wgpu::Device, uniform_buf: &wgpu::Buffer) -> wgpu::BindGroup {
        fn view(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry { binding, resource: wgpu::BindingResource::TextureView(view) }
        }
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("raw bind group"),
            layout: &self.layout,
            entries: &[
                view(0, &self.image_view),
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: uniform_buf.as_entire_binding() },
                view(3, &self.hue_sat_view),
                view(4, &self.look_view),
                view(5, &self.curve_view),
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::Sampler(&self.table_sampler) },
            ],
        })
    }

    fn rebuild_bind_group(&mut self, device: &wgpu::Device) {
        self.bind_group = self.make_bind_group(device, &self.uniform_buf);
        self.scope_bind_group = self.make_bind_group(device, &self.scope_uniform_buf);
        self.before_bind_group = self.make_bind_group(device, &self.before_uniform_buf);
    }

    /// Render `region` of the picture (x, y, width, height in the straightened frame, 0..1 =
    /// the whole picture) at `width` x `height` pixels
    /// with the given settings and read it back as sRGB-encoded 16-bit RGB, row-major.
    /// `uniforms` are the same ones the viewport uses; only the view rectangle is replaced.
    pub fn render_full(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        uniforms: &Uniforms,
        width: u32,
        height: u32,
        region: [f32; 4],
    ) -> Result<Vec<u16>, String> {
        let tile = EXPORT_TILE;
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("export tile"),
            size: wgpu::Extent3d { width: tile, height: tile, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: EXPORT_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let row_bytes = tile * 8;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("export readback"),
            size: (row_bytes * tile) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("export uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = self.make_bind_group(device, &uniform_buf);

        let (w, h) = (width as usize, height as usize);
        let mut out = vec![0u16; w * h * 3];
        let mut y0 = 0u32;
        while y0 < height {
            let mut x0 = 0u32;
            while x0 < width {
                // The tile covers image pixels [x0, x0 + tile) x [y0, y0 + tile); whatever
                // falls outside the picture is discarded by the shader and cropped below.
                let mut u = *uniforms;
                u.view = [
                    tile as f32 / width as f32 * region[2],
                    tile as f32 / height as f32 * region[3],
                    region[0] + (x0 as f32 + tile as f32 * 0.5) / width as f32 * region[2],
                    region[1] + (y0 as f32 + tile as f32 * 0.5) / height as f32 * region[3],
                ];
                u.basic[3] = 1.0; // the float target does not encode sRGB for us
                queue.write_buffer(&uniform_buf, 0, bytemuck::bytes_of(&u));

                let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("export") });
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("export pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &target_view,
                            resolve_target: None,
                            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                    pass.set_pipeline(&self.export_pipeline);
                    pass.set_bind_group(0, &bind_group, &[]);
                    pass.draw(0..3, 0..1);
                }
                encoder.copy_texture_to_buffer(
                    wgpu::ImageCopyTexture {
                        texture: &target,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::ImageCopyBuffer {
                        buffer: &readback,
                        layout: wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(row_bytes), rows_per_image: Some(tile) },
                    },
                    wgpu::Extent3d { width: tile, height: tile, depth_or_array_layers: 1 },
                );
                queue.submit(Some(encoder.finish()));

                let slice = readback.slice(..);
                let (tx, rx) = std::sync::mpsc::channel();
                slice.map_async(wgpu::MapMode::Read, move |result| {
                    let _ = tx.send(result);
                });
                device.poll(wgpu::Maintain::Wait);
                match rx.recv() {
                    Ok(Ok(())) => {}
                    _ => return Err("The GPU could not hand the rendered image back.".into()),
                }
                {
                    let data = slice.get_mapped_range();
                    let texels: &[u16] = bytemuck::cast_slice(&data);
                    let copy_w = (width - x0).min(tile) as usize;
                    let copy_h = (height - y0).min(tile) as usize;
                    for ty in 0..copy_h {
                        let src = &texels[ty * tile as usize * 4..];
                        let dst = &mut out[((y0 as usize + ty) * w + x0 as usize) * 3..][..copy_w * 3];
                        for (d, s) in dst.chunks_exact_mut(3).zip(src.chunks_exact(4)) {
                            for c in 0..3 {
                                let v = half::f16::from_bits(s[c]).to_f32();
                                // NaN compares false both ways and so lands on 0.
                                d[c] = if v >= 1.0 { 65535 } else if v > 0.0 { (v * 65535.0 + 0.5) as u16 } else { 0 };
                            }
                        }
                    }
                }
                readback.unmap();
                x0 += tile;
            }
            y0 += tile;
        }
        Ok(out)
    }

    /// Stage 1 hand-off: copy the demosaiced mip chain into a fresh RGBA16F texture.
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, mips: &[MipLevel]) {
        let Some(base) = mips.first() else { return };
        let texture = Self::create_image(device, base.width, base.height, mips.len() as u32);

        // Upload in bands and flush between them so the staging memory stays small
        // (a full 61 MP level is ~480 MB).
        const BAND_BYTES: usize = 32 << 20;
        for (level, mip) in mips.iter().enumerate() {
            let row_bytes = mip.width as usize * 8;
            let band_rows = (BAND_BYTES / row_bytes).max(1);
            let bytes: &[u8] = bytemuck::cast_slice(&mip.rgba);
            let mut y = 0usize;
            while y < mip.height as usize {
                let rows = band_rows.min(mip.height as usize - y);
                queue.write_texture(
                    wgpu::ImageCopyTexture {
                        texture: &texture,
                        mip_level: level as u32,
                        origin: wgpu::Origin3d { x: 0, y: y as u32, z: 0 },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &bytes[y * row_bytes..(y + rows) * row_bytes],
                    wgpu::ImageDataLayout {
                        offset: 0,
                        bytes_per_row: Some(row_bytes as u32),
                        rows_per_image: Some(rows as u32),
                    },
                    wgpu::Extent3d { width: mip.width, height: rows as u32, depth_or_array_layers: 1 },
                );
                queue.submit(std::iter::empty());
                y += rows;
            }
        }

        self.image_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.rebuild_bind_group(device);
    }

    /// Upload a profile's lookup tables. Cheap (a few hundred kilobytes at most), so it is
    /// simply redone whenever the profile or the white-balance temperature changes.
    pub fn set_profile(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, profile: &ProfileRender) {
        let one_texel = [0u16; 4];
        let table = |t: &Option<GpuTable>| match t {
            Some(t) => Self::upload_lut(device, queue, wgpu::TextureDimension::D3, t.dims, &t.rgba),
            None => Self::upload_lut(device, queue, wgpu::TextureDimension::D3, [1, 1, 1], &one_texel),
        };
        self.hue_sat_view = table(&profile.hue_sat);
        self.look_view = table(&profile.look);
        self.curve_view = match &profile.curve {
            Some(c) => Self::upload_lut(device, queue, wgpu::TextureDimension::D2, [CURVE_LUT_SIZE as u32, 1, 1], c),
            None => Self::upload_lut(device, queue, wgpu::TextureDimension::D2, [1, 1, 1], &one_texel),
        };
        self.rebuild_bind_group(device);
    }

    /// Settings for the scopes' small rendering: the same as the viewport's, but always the
    /// whole cropped picture and always display-encoded.
    pub fn write_scope_uniforms(&self, queue: &wgpu::Queue, uniforms: &Uniforms) {
        queue.write_buffer(&self.scope_uniform_buf, 0, bytemuck::bytes_of(uniforms));
    }

    /// Draw the finished picture into `target` (the scopes' off-screen texture).
    pub fn render_scope_source(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scope source"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.export_pipeline);
        pass.set_bind_group(0, &self.scope_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Settings for the "before" half of the A/B split (the photo's defaults).
    pub fn write_before_uniforms(&self, queue: &wgpu::Queue, uniforms: &Uniforms) {
        queue.write_buffer(&self.before_uniform_buf, 0, bytemuck::bytes_of(uniforms));
    }

    pub fn write_uniforms(&self, queue: &wgpu::Queue, uniforms: &Uniforms) {
        queue.write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(uniforms));
    }
}

/// Paint callback: egui sets the viewport to the callback rect, we draw one triangle over it.
pub struct ImageCallback {
    /// Draw with the "before" settings (A/B split) instead of the current ones.
    pub before: bool,
}

impl egui_wgpu::CallbackTrait for ImageCallback {
    fn paint<'a>(
        &'a self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'a>,
        resources: &'a egui_wgpu::CallbackResources,
    ) {
        let Some(renderer) = resources.get::<ImageRenderer>() else { return };
        render_pass.set_pipeline(&renderer.pipeline);
        render_pass.set_bind_group(0, if self.before { &renderer.before_bind_group } else { &renderer.bind_group }, &[]);
        render_pass.draw(0..3, 0..1);
    }
}
