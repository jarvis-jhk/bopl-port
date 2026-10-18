//! The renderer: textures and render targets, pipelines, batching, passes.
//!
//! [`Renderer::render`] draws a [`Frame`]: for each [`Pass`] in order it
//! renders the pass's items into its target (the output or a render
//! texture), with a stencil buffer for sprite masks. All passes share one
//! uber-shader (`shaders/quad.wgsl`) with a per-instance shading kind;
//! pipelines differ only by blend mode, stencil mode and target format and
//! are created on first use.

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use glam::{Mat2, Mat4, Vec2, Vec3};

use crate::RenderError;
use crate::camera::{Projection, screen_proj};
use crate::draw::{Blend, Frame, Item, PassTarget, Quad, Shading, Stencil, Text, View};
use crate::gpu::Gpu;
use crate::image::Image;
use crate::sdf::{DYN_ATLAS_SIZE, DynamicSdfDesc, SdfFontDesc, SdfFonts};
use crate::target::{OFFSCREEN_FORMAT, TargetView, capture_texture};
use crate::text::{ATLAS_SIZE, FontId, MAX_GLYPH_PX, PlacedGlyph, TextSystem};
use crate::texture::{
    Filter, GpuTexture, TextureId, TextureOptions, Wrap, sampler_desc, texture_format, write_rows,
};

const STENCIL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;

const K_SPRITE: u32 = 0;
const K_ALPHA_TINT: u32 = 1;
const K_CHANNEL: u32 = 2;
const K_GRADIENT: u32 = 3;
const K_SLIME: u32 = 4;
const K_WATER: u32 = 5;
const K_DISTORT: u32 = 6;
const K_AURORA: u32 = 7;
const K_ELLIPSE: u32 = 8;
const K_RING: u32 = 9;
const K_RRECT: u32 = 10;
const K_SDF: u32 = 11;
const K_MASK: u32 = 12;
const K_COPY: u32 = 13;
const K_BLACKHOLE: u32 = 14;
const K_GRAB_INVERT: u32 = 15;
const K_DISTORT_SPRITE: u32 = 16;
const K_SMEAR: u32 = 17;
const K_ONLY_WHITE: u32 = 18;
const K_EXPANDING: u32 = 19;
const K_WHITE_TO_COL: u32 = 20;
const K_WALL_MASKED: u32 = 21;
const K_SINGLE_CHANNEL: u32 = 22;
const K_CHANNELS3: u32 = 23;
const K_RPLATFORM: u32 = 24;
const K_LIGHTNING: u32 = 25;
const K_BEAM: u32 = 26;
const K_CLONE: u32 = 27;
const K_PARTICLE_PREMUL: u32 = 28;
const K_PARTICLE_BLEND: u32 = 29;
const K_INDICATOR: u32 = 30;

/// Statistics of the last rendered frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameStats {
    pub passes: u32,
    pub quads: u32,
    pub draw_calls: u32,
    pub pipelines: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct QuadInstance {
    origin: [f32; 4],
    axis_x: [f32; 4],
    axis_y: [f32; 4],
    uv: [f32; 4],
    color: [f32; 4],
    p: [[f32; 4]; 6],
    kind: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    time: [f32; 4],
    sin_time: [f32; 4],
    cos_time: [f32; 4],
    misc: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct PipeKey {
    blend: Blend,
    stencil: Stencil,
    format: wgpu::TextureFormat,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Batch {
    blend: Blend,
    stencil: Stencil,
    texture: TextureId,
    extras: [TextureId; 3],
    start: u32,
    end: u32,
}

/// A vertex/instance buffer that grows on demand.
struct GrowBuffer {
    buffer: wgpu::Buffer,
    usage: wgpu::BufferUsages,
    label: &'static str,
}

impl GrowBuffer {
    fn new(device: &wgpu::Device, label: &'static str, usage: wgpu::BufferUsages) -> Self {
        let usage = usage | wgpu::BufferUsages::COPY_DST;
        Self {
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: 4096,
                usage,
                mapped_at_creation: false,
            }),
            usage,
            label,
        }
    }

    fn write(&mut self, gpu: &Gpu, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let len = data.len() as u64;
        if len > self.buffer.size() {
            self.buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(self.label),
                size: len.next_power_of_two(),
                usage: self.usage,
                mapped_at_creation: false,
            });
        }
        gpu.queue.write_buffer(&self.buffer, 0, data);
    }
}

/// The renderer. One per output format (window or offscreen).
pub struct Renderer {
    output_format: wgpu::TextureFormat,
    shader: wgpu::ShaderModule,
    layout: wgpu::PipelineLayout,
    globals_bgl: wgpu::BindGroupLayout,
    texture_bgl: wgpu::BindGroupLayout,
    extras_bgl: wgpu::BindGroupLayout,
    pipelines: HashMap<PipeKey, wgpu::RenderPipeline>,
    globals_buf: wgpu::Buffer,
    globals_bg: wgpu::BindGroup,
    globals_stride: u32,
    globals_slots: u32,
    textures: Vec<Option<GpuTexture>>,
    /// Named render targets (`_ReflectionRT`, ...).
    targets: HashMap<String, TextureId>,
    /// Stencil buffers by size.
    stencils: HashMap<(u32, u32), wgpu::TextureView>,
    /// Bind groups of three secondary textures, keyed by ids and generations.
    extras: HashMap<[(TextureId, u32); 3], wgpu::BindGroup>,
    text: TextSystem,
    sdf: SdfFonts,
    instance_buf: GrowBuffer,
    // Per-frame scratch.
    instances: Vec<QuadInstance>,
    batches: Vec<Vec<Batch>>,
    glyphs: Vec<PlacedGlyph>,
    stats: FrameStats,
}

fn uniform_entry(binding: u32, dynamic: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: dynamic,
            min_binding_size: None,
        },
        count: None,
    }
}

fn tex_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
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

fn blend_state(b: Blend) -> wgpu::BlendState {
    let comp = |src, dst| wgpu::BlendComponent {
        src_factor: src,
        dst_factor: dst,
        operation: wgpu::BlendOperation::Add,
    };
    use wgpu::BlendFactor as F;
    let c = match b {
        Blend::Premultiplied => comp(F::One, F::OneMinusSrcAlpha),
        Blend::Additive => comp(F::One, F::One),
        Blend::InverseAlpha => comp(F::OneMinusSrcAlpha, F::SrcAlpha),
        Blend::SoftAdditive => comp(F::One, F::OneMinusSrc),
    };
    wgpu::BlendState { color: c, alpha: c }
}

/// Stencil state (reference value 1). Unity: visible inside = `1 <= s`,
/// outside = `1 > s`; masks increment / decrement.
fn stencil_state(s: Stencil) -> wgpu::DepthStencilState {
    use wgpu::{CompareFunction as C, StencilOperation as O};
    let (compare, pass_op) = match s {
        Stencil::None => (C::Always, O::Keep),
        Stencil::VisibleInside => (C::LessEqual, O::Keep),
        Stencil::VisibleOutside => (C::Greater, O::Keep),
        Stencil::MaskAdd => (C::Always, O::IncrementClamp),
        Stencil::MaskRemove => (C::Always, O::DecrementClamp),
    };
    let face = wgpu::StencilFaceState {
        compare,
        fail_op: O::Keep,
        depth_fail_op: O::Keep,
        pass_op,
    };
    wgpu::DepthStencilState {
        format: STENCIL_FORMAT,
        depth_write_enabled: Some(false),
        depth_compare: Some(C::Always),
        stencil: wgpu::StencilState {
            front: face,
            back: face,
            read_mask: 0xff,
            write_mask: 0xff,
        },
        bias: Default::default(),
    }
}

impl Renderer {
    /// Creates a renderer whose [`PassTarget::Output`] has `output_format`
    /// (the surface format, or [`OFFSCREEN_FORMAT`]).
    pub fn new(gpu: &Gpu, output_format: wgpu::TextureFormat) -> Self {
        let device = &gpu.device;
        let globals_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[uniform_entry(0, true)],
        });
        let texture_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("texture"),
            entries: &[tex_entry(0), sampler_entry(1)],
        });
        let extras_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("extra textures"),
            entries: &[
                tex_entry(0),
                sampler_entry(1),
                tex_entry(2),
                sampler_entry(3),
                tex_entry(4),
                sampler_entry(5),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("quads"),
            bind_group_layouts: &[Some(&globals_bgl), Some(&texture_bgl), Some(&extras_bgl)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("quad.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/quad.wgsl").into()),
        });
        let globals_stride = device.limits().min_uniform_buffer_offset_alignment.max(256);
        let (globals_buf, globals_bg) =
            Self::make_globals(device, &globals_bgl, globals_stride, 16);
        let mut r = Self {
            output_format,
            shader,
            layout,
            globals_bgl,
            texture_bgl,
            extras_bgl,
            pipelines: HashMap::new(),
            globals_buf,
            globals_bg,
            globals_stride,
            globals_slots: 16,
            textures: Vec::new(),
            targets: HashMap::new(),
            stencils: HashMap::new(),
            extras: HashMap::new(),
            text: TextSystem::new(),
            sdf: SdfFonts::new(),
            instance_buf: GrowBuffer::new(device, "quad instances", wgpu::BufferUsages::VERTEX),
            instances: Vec::new(),
            batches: Vec::new(),
            glyphs: Vec::new(),
            stats: FrameStats::default(),
        };
        let white = r.create_texture(
            gpu,
            &Image::solid(1, 1, [255; 4]),
            TextureOptions::default(),
        );
        debug_assert_eq!(white, TextureId::WHITE);
        let atlas = Image::from_pixels(ATLAS_SIZE, ATLAS_SIZE, r.text.atlas.clone());
        let glyphs = r.create_texture(gpu, &atlas, TextureOptions::default());
        debug_assert_eq!(glyphs, TextureId::GLYPHS);
        r.text.dirty_rows = None;
        let clear = r.create_texture(gpu, &Image::new(1, 1), TextureOptions::default());
        debug_assert_eq!(clear, TextureId::TRANSPARENT);
        let dyn_atlas = Image::new(DYN_ATLAS_SIZE, DYN_ATLAS_SIZE);
        let dyn_sdf = r.create_texture(gpu, &dyn_atlas, TextureOptions::default());
        debug_assert_eq!(dyn_sdf, TextureId::DYN_SDF);
        r
    }

    fn make_globals(
        device: &wgpu::Device,
        bgl: &wgpu::BindGroupLayout,
        stride: u32,
        slots: u32,
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: (stride * slots) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buf,
                    offset: 0,
                    size: wgpu::BufferSize::new(std::mem::size_of::<Globals>() as u64),
                }),
            }],
        });
        (buf, bg)
    }

    pub fn output_format(&self) -> wgpu::TextureFormat {
        self.output_format
    }

    /// Statistics of the last [`render`](Self::render).
    pub fn stats(&self) -> FrameStats {
        self.stats
    }

    // ---- textures -------------------------------------------------------

    fn texture_bind_group(
        &self,
        gpu: &Gpu,
        view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("texture"),
            layout: &self.texture_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    }

    fn insert_texture(&mut self, tex: GpuTexture) -> TextureId {
        let base = TextureId::BUILTIN as usize;
        if self.textures.len() >= base
            && let Some(i) = self.textures[base..].iter().position(Option::is_none)
        {
            self.textures[base + i] = Some(tex);
            TextureId((base + i) as u32)
        } else {
            self.textures.push(Some(tex));
            TextureId(self.textures.len() as u32 - 1)
        }
    }

    fn make_texture(
        &self,
        gpu: &Gpu,
        size: (u32, u32),
        format: wgpu::TextureFormat,
        usage: wgpu::TextureUsages,
        opts: &TextureOptions,
        generation: u32,
    ) -> GpuTexture {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("texture"),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let sampler = gpu.device.create_sampler(&sampler_desc(opts));
        let bind_group = self.texture_bind_group(gpu, &view, &sampler);
        GpuTexture {
            texture,
            view,
            sampler,
            bind_group,
            size,
            opts: *opts,
            generation,
        }
    }

    /// Uploads an RGBA8 image as a texture.
    pub fn create_texture(&mut self, gpu: &Gpu, img: &Image, opts: TextureOptions) -> TextureId {
        let t = self.make_texture(
            gpu,
            (img.width, img.height),
            texture_format(&opts),
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            &opts,
            0,
        );
        write_rows(
            &gpu.queue,
            &t.texture,
            &img.pixels,
            img.width,
            (0, img.height),
        );
        self.insert_texture(t)
    }

    /// Replaces the contents of a texture created by
    /// [`create_texture`](Self::create_texture) (same size).
    pub fn update_texture(
        &mut self,
        gpu: &Gpu,
        id: TextureId,
        img: &Image,
    ) -> Result<(), RenderError> {
        let t = self.gpu_texture(id).ok_or(RenderError::BadTexture)?;
        if t.size != (img.width, img.height) {
            return Err(RenderError::Unsupported(
                "update_texture with a different size".into(),
            ));
        }
        write_rows(
            &gpu.queue,
            &t.texture,
            &img.pixels,
            img.width,
            (0, img.height),
        );
        Ok(())
    }

    /// The render texture called `name`, created (or recreated when the size
    /// or options change) at `size`. Its contents survive between frames
    /// while the size stays the same.
    pub fn render_target(
        &mut self,
        gpu: &Gpu,
        name: &str,
        size: (u32, u32),
        wrap: Wrap,
    ) -> TextureId {
        let size = (size.0.max(1), size.1.max(1));
        let opts = TextureOptions {
            filter: Filter::Linear,
            wrap,
            srgb: false,
        };
        if let Some(&id) = self.targets.get(name) {
            let t = self.gpu_texture(id).expect("render target exists");
            if t.size == size && t.opts == opts {
                return id;
            }
            let generation = t.generation + 1;
            let t = self.make_render_texture(gpu, size, &opts, generation);
            self.textures[id.0 as usize] = Some(t);
            return id;
        }
        let t = self.make_render_texture(gpu, size, &opts, 0);
        let id = self.insert_texture(t);
        self.targets.insert(name.to_string(), id);
        id
    }

    fn make_render_texture(
        &self,
        gpu: &Gpu,
        size: (u32, u32),
        opts: &TextureOptions,
        generation: u32,
    ) -> GpuTexture {
        self.make_texture(
            gpu,
            size,
            OFFSCREEN_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            opts,
            generation,
        )
    }

    /// Frees a texture or render target; the id may be reused.
    pub fn destroy_texture(&mut self, id: TextureId) {
        if id.0 >= TextureId::BUILTIN
            && let Some(slot) = self.textures.get_mut(id.0 as usize)
        {
            *slot = None;
            self.targets.retain(|_, v| *v != id);
        }
    }

    pub fn texture_size(&self, id: TextureId) -> Option<(u32, u32)> {
        self.gpu_texture(id).map(|t| t.size)
    }

    fn gpu_texture(&self, id: TextureId) -> Option<&GpuTexture> {
        self.textures.get(id.0 as usize).and_then(Option::as_ref)
    }

    fn texture_or_white(&self, id: TextureId) -> &GpuTexture {
        self.gpu_texture(id)
            .unwrap_or_else(|| self.textures[0].as_ref().unwrap())
    }

    /// Reads a render target back (debugging, tests).
    pub fn capture(&self, gpu: &Gpu, id: TextureId) -> Result<Image, RenderError> {
        let t = self.gpu_texture(id).ok_or(RenderError::BadTexture)?;
        capture_texture(gpu, &t.texture, t.size, OFFSCREEN_FORMAT)
    }

    // ---- fonts ----------------------------------------------------------

    /// Loads a TTF/OTF font for raster text.
    pub fn load_font(&mut self, bytes: &[u8]) -> Result<FontId, RenderError> {
        self.text.add_font(bytes)
    }

    /// Whether raster font `font` has a glyph for `ch`.
    pub fn font_has_char(&self, font: FontId, ch: char) -> bool {
        self.text.has_char(font, ch)
    }

    /// Adds a static signed-distance-field font (TMP font asset with atlas).
    pub fn add_sdf_font(&mut self, desc: SdfFontDesc) -> FontId {
        self.sdf.add_static(desc)
    }

    /// Adds a dynamic SDF font (glyphs generated from the TTF on use).
    pub fn add_dynamic_sdf_font(&mut self, desc: DynamicSdfDesc) -> Result<FontId, RenderError> {
        self.sdf.add_dynamic(desc)
    }

    /// Fallback fonts of an SDF font (TMP fallback font asset table).
    pub fn set_sdf_fallbacks(&mut self, font: FontId, fallbacks: &[FontId]) {
        self.sdf.set_fallbacks(font, fallbacks);
    }

    /// Size of a text block at `size` (same units as [`Text::size`]).
    pub fn measure_text(&mut self, font: FontId, text: &str, size: f32) -> Vec2 {
        self.measure_text_spaced(font, text, size, 0.0)
    }

    /// [`Self::measure_text`] with [`Text::spacing`] (SDF fonts).
    pub fn measure_text_spaced(
        &mut self,
        font: FontId,
        text: &str,
        size: f32,
        spacing: f32,
    ) -> Vec2 {
        if self.sdf.contains(font) {
            return self.sdf.layout_spaced(font, text, size, 0.0, spacing).1;
        }
        let px = 64.0;
        self.text.measure(font, text, px) * (size / px)
    }

    // ---- frame ----------------------------------------------------------

    fn stencil_view(&mut self, gpu: &Gpu, size: (u32, u32)) {
        self.stencils.entry(size).or_insert_with(|| {
            gpu.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("stencil"),
                    size: wgpu::Extent3d {
                        width: size.0,
                        height: size.1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: STENCIL_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        });
    }

    fn ensure_pipeline(&mut self, gpu: &Gpu, key: PipeKey) {
        if self.pipelines.contains_key(&key) {
            return;
        }
        let colour = !matches!(key.stencil, Stencil::MaskAdd | Stencil::MaskRemove);
        let p = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("quads"),
                layout: Some(&self.layout),
                vertex: wgpu::VertexState {
                    module: &self.shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<QuadInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &wgpu::vertex_attr_array![
                            0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4,
                            4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4,
                            8 => Float32x4, 9 => Float32x4, 10 => Float32x4, 11 => Uint32x4
                        ],
                    })],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: Some(stencil_state(key.stencil)),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &self.shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: key.format,
                        blend: Some(blend_state(key.blend)),
                        write_mask: if colour {
                            wgpu::ColorWrites::ALL
                        } else {
                            wgpu::ColorWrites::empty()
                        },
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
        self.pipelines.insert(key, p);
    }

    fn extras_key(&self, ids: [TextureId; 3]) -> [(TextureId, u32); 3] {
        ids.map(|id| {
            let t = self.texture_or_white(id);
            (id, t.generation)
        })
    }

    fn ensure_extras(&mut self, gpu: &Gpu, ids: [TextureId; 3]) {
        let key = self.extras_key(ids);
        if self.extras.contains_key(&key) {
            return;
        }
        let [a, b, c] = ids.map(|id| self.texture_or_white(id));
        let bg = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("extra textures"),
            layout: &self.extras_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&a.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&a.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&b.view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&b.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&c.view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&c.sampler),
                },
            ],
        });
        // Drop stale entries for replaced textures now and then.
        if self.extras.len() > 256 {
            self.extras.clear();
        }
        self.extras.insert(key, bg);
    }

    /// Renders `frame`. Passes targeting [`PassTarget::Output`] draw into
    /// `output` (whose format must be the renderer's output format).
    pub fn render(&mut self, gpu: &Gpu, output: TargetView<'_>, frame: &Frame) {
        assert_eq!(
            output.format, self.output_format,
            "output format differs from the renderer's"
        );
        self.text.begin_frame();
        self.stats = FrameStats::default();
        self.instances.clear();
        let n = frame.passes.len();
        self.batches.resize_with(n, Vec::new);
        for b in &mut self.batches {
            b.clear();
        }

        // Geometry and per-pass globals.
        let mut globals = Vec::with_capacity(n);
        let t = frame.time;
        for (pi, pass) in frame.passes.iter().enumerate() {
            let size = match pass.target {
                PassTarget::Output => output.size,
                PassTarget::Texture(id) => self.texture_size(id).unwrap_or((1, 1)),
            };
            let size = (size.0.max(1), size.1.max(1));
            let (vp, ppu, screen) = match &pass.view {
                View::Camera(c) => {
                    let aspect = c.aspect_for(size);
                    let ppu = match c.projection {
                        Projection::Orthographic { size: s } => size.1 as f32 / (2.0 * s),
                        Projection::Perspective { .. } => size.1 as f32 / 2.0,
                    };
                    (c.view_proj(aspect), ppu, false)
                }
                View::Screen => (screen_proj(size.0, size.1), 1.0, true),
            };
            globals.push(Globals {
                view_proj: vp.to_cols_array_2d(),
                time: [t / 20.0, t, t * 2.0, t * 3.0],
                sin_time: [(t / 8.0).sin(), (t / 4.0).sin(), (t / 2.0).sin(), t.sin()],
                cos_time: [(t / 8.0).cos(), (t / 4.0).cos(), (t / 2.0).cos(), t.cos()],
                misc: [frame.timestop, 1.0, size.0 as f32, size.1 as f32],
            });
            for item in &pass.items {
                match item {
                    Item::Quad(q) => self.push_quad(pi, q, screen),
                    Item::Text(tx) => self.push_text(pi, tx, screen, ppu),
                }
            }
        }

        // Uploads: atlases, instances, globals.
        if let Some(rows) = self.text.dirty_rows.take() {
            let atlas = &self.textures[TextureId::GLYPHS.0 as usize]
                .as_ref()
                .unwrap()
                .texture;
            write_rows(&gpu.queue, atlas, &self.text.atlas, ATLAS_SIZE, rows);
        }
        if let Some(rows) = self.sdf.dirty_rows.take() {
            let atlas = &self.textures[TextureId::DYN_SDF.0 as usize]
                .as_ref()
                .unwrap()
                .texture;
            write_rows(&gpu.queue, atlas, &self.sdf.atlas, DYN_ATLAS_SIZE, rows);
        }
        self.instance_buf
            .write(gpu, bytemuck::cast_slice(&self.instances));
        if n as u32 > self.globals_slots {
            self.globals_slots = (n as u32).next_power_of_two();
            let (b, g) = Self::make_globals(
                &gpu.device,
                &self.globals_bgl,
                self.globals_stride,
                self.globals_slots,
            );
            self.globals_buf = b;
            self.globals_bg = g;
        }
        for (i, g) in globals.iter().enumerate() {
            gpu.queue.write_buffer(
                &self.globals_buf,
                (i as u32 * self.globals_stride) as u64,
                bytemuck::bytes_of(g),
            );
        }

        // Pipelines, bind groups, stencil buffers.
        for (pi, pass) in frame.passes.iter().enumerate() {
            let (format, size) = match pass.target {
                PassTarget::Output => (output.format, output.size),
                PassTarget::Texture(id) => {
                    (OFFSCREEN_FORMAT, self.texture_size(id).unwrap_or((1, 1)))
                }
            };
            self.stencil_view(gpu, (size.0.max(1), size.1.max(1)));
            let batches = std::mem::take(&mut self.batches[pi]);
            for b in &batches {
                self.ensure_pipeline(
                    gpu,
                    PipeKey {
                        blend: b.blend,
                        stencil: b.stencil,
                        format,
                    },
                );
                self.ensure_extras(gpu, b.extras);
            }
            self.batches[pi] = batches;
        }

        // Passes.
        let (mut passes, mut draws) = (0u32, 0u32);
        let mut enc = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        for (pi, pass) in frame.passes.iter().enumerate() {
            let (view, format, size) = match pass.target {
                PassTarget::Output => (output.view, output.format, output.size),
                PassTarget::Texture(id) => {
                    let Some(t) = self.gpu_texture(id) else {
                        log::warn!("pass {}: unknown render target", pass.label);
                        continue;
                    };
                    (&t.view, OFFSCREEN_FORMAT, t.size)
                }
            };
            let size = (size.0.max(1), size.1.max(1));
            let load = match pass.clear {
                Some(c) => wgpu::LoadOp::Clear(wgpu::Color {
                    r: c.r as f64,
                    g: c.g as f64,
                    b: c.b as f64,
                    a: c.a as f64,
                }),
                None => wgpu::LoadOp::Load,
            };
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(&pass.label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.stencils[&size],
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0),
                        store: wgpu::StoreOp::Discard,
                    }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            passes += 1;
            let batches = &self.batches[pi];
            if batches.is_empty() {
                continue;
            }
            rp.set_bind_group(0, &self.globals_bg, &[pi as u32 * self.globals_stride]);
            rp.set_vertex_buffer(0, self.instance_buf.buffer.slice(..));
            rp.set_stencil_reference(1);
            for b in batches {
                let key = PipeKey {
                    blend: b.blend,
                    stencil: b.stencil,
                    format,
                };
                rp.set_pipeline(&self.pipelines[&key]);
                rp.set_bind_group(1, &self.texture_or_white(b.texture).bind_group, &[]);
                rp.set_bind_group(2, &self.extras[&self.extras_key(b.extras)], &[]);
                rp.draw(0..4, b.start..b.end);
                draws += 1;
            }
        }
        gpu.queue.submit([enc.finish()]);
        self.stats.passes = passes;
        self.stats.draw_calls = draws;
        self.stats.quads = self.instances.len() as u32;
        self.stats.pipelines = self.pipelines.len() as u32;
    }

    fn push_instance(
        &mut self,
        pass: usize,
        inst: QuadInstance,
        texture: TextureId,
        extras: [TextureId; 3],
        blend: Blend,
        stencil: Stencil,
    ) {
        let i = self.instances.len() as u32;
        self.instances.push(inst);
        let batches = &mut self.batches[pass];
        if let Some(last) = batches.last_mut()
            && last.texture == texture
            && last.extras == extras
            && last.blend == blend
            && last.stencil == stencil
            && last.end == i
        {
            last.end = i + 1;
            return;
        }
        batches.push(Batch {
            blend,
            stencil,
            texture,
            extras,
            start: i,
            end: i + 1,
        });
    }

    fn push_quad(&mut self, pass: usize, q: &Quad, screen: bool) {
        let m = q.transform;
        let origin = m.transform_point3((-q.pivot * q.size).extend(0.0));
        let ax = m.transform_vector3(Vec3::new(q.size.x, 0.0, 0.0));
        let ay = m.transform_vector3(Vec3::new(0.0, q.size.y, 0.0));
        // World: local y up, so the quad's bottom edge samples the image's
        // bottom (max v). Screen: y down, local y = 0 is the top.
        let uv = if screen {
            [q.uv.min.x, q.uv.min.y, q.uv.max.x, q.uv.max.y]
        } else {
            [q.uv.min.x, q.uv.max.y, q.uv.max.x, q.uv.min.y]
        };
        let none = TextureId::TRANSPARENT;
        let mut p = [[0.0f32; 4]; 6];
        let mut extras = [none; 3];
        let mut flags = 0u32;
        let kind = match q.shading {
            Shading::Sprite => K_SPRITE,
            Shading::AlphaTint => K_ALPHA_TINT,
            Shading::ChannelColour { main, shadow } => {
                p[0] = main.to_array();
                p[1] = shadow.to_array();
                K_CHANNEL
            }
            Shading::Gradient {
                top,
                bottom,
                alpha_top,
                alpha_bottom,
                height,
            } => {
                p[0] = top.to_array();
                p[1] = bottom.to_array();
                p[2] = [alpha_top, alpha_bottom, height, 0.0];
                K_GRADIENT
            }
            Shading::SlimeTrailed {
                trail,
                trail_texel,
                flip_x,
                flip_y,
            } => {
                extras[0] = trail;
                p[0] = [
                    flip_x as u8 as f32,
                    flip_y as u8 as f32,
                    trail_texel.x,
                    trail_texel.y,
                ];
                K_SLIME
            }
            Shading::ReflectiveWater(w) => {
                extras = [w.wave, w.reflection, w.background];
                p[0] = [w.alpha, w.distortion, w.horizon_height, w.surface_gradient];
                p[1] = [
                    w.extra_noise,
                    w.gradient_height,
                    w.horizon_gradient_height,
                    w.reflection_height,
                ];
                p[2] = w.gradient_top.to_array();
                p[3] = w.gradient_bottom.to_array();
                p[4] = w.water_color.to_array();
                K_WATER
            }
            Shading::WaterDistortion(d) => {
                extras[0] = d.noise;
                p[0] = [d.strength, d.subdivide, d.subdivide_noise, 0.0];
                p[1] = d.noise_scale.extend(0.0).to_array();
                p[2] = d.strength3.extend(0.0).to_array();
                p[3] = d.uv_center.to_array();
                K_DISTORT
            }
            Shading::Aurora(a) => {
                extras[0] = a.noise;
                extras[1] = a.mask;
                p[0] = [a.height, a.blue, a.wobbliness, 0.0];
                K_AURORA
            }
            Shading::Ellipse => K_ELLIPSE,
            Shading::Ring {
                thickness,
                start,
                sweep,
            } => {
                let (mut start, mut sweep) = (
                    start,
                    sweep.clamp(-std::f32::consts::TAU, std::f32::consts::TAU),
                );
                if sweep < 0.0 {
                    start += sweep;
                    sweep = -sweep;
                }
                p[0] = [thickness, start, sweep, 0.0];
                K_RING
            }
            Shading::AbilityIndicator {
                circle,
                rim,
                cd,
                radius,
                rim_width,
                progress,
                uv_clamp,
            } => {
                p[0] = circle.to_array();
                p[1] = rim.to_array();
                p[2] = cd.to_array();
                p[3] = [radius, rim_width, progress, 0.0];
                p[4] = [
                    uv_clamp.min.x,
                    uv_clamp.min.y,
                    uv_clamp.max.x,
                    uv_clamp.max.y,
                ];
                K_INDICATOR
            }
            Shading::RoundedRect { radius } => {
                p[0] = [radius, 0.0, 0.0, 0.0];
                K_RRECT
            }
            Shading::Mask { cutoff } => {
                p[0] = [cutoff, 0.0, 0.0, 0.0];
                K_MASK
            }
            Shading::SpriteCorner { corner } => {
                // The free corner travels in p5 (no other sprite input
                // uses it); kind.y flags the free-form quad.
                p[5] = corner.extend(1.0).to_array();
                flags = 1;
                K_SPRITE
            }
            Shading::Copy => K_COPY,
            Shading::BlackHole {
                grab,
                distortion,
                proximity,
                scale,
                hole_brightness,
            } => {
                extras[0] = grab;
                p[0] = [distortion, proximity, scale, hole_brightness];
                K_BLACKHOLE
            }
            Shading::GrabInvert { grab } => {
                extras[0] = grab;
                K_GRAB_INVERT
            }
            Shading::Distort(d) => {
                extras[0] = d.noise;
                p[0] = [d.speed, d.uv_center.x, d.uv_center.y, d.affect_radius];
                p[1] = [d.cc, d.cw, d.strength, 0.0];
                K_DISTORT_SPRITE
            }
            Shading::Smear {
                main,
                shadow,
                offset,
                scale,
            } => {
                p[0] = main.to_array();
                p[1] = shadow.to_array();
                p[2] = offset.to_array();
                p[3] = [scale.x, scale.y, 0.0, 0.0];
                K_SMEAR
            }
            Shading::OnlyWhite => K_ONLY_WHITE,
            Shading::ExpandingCircle {
                noise,
                mask,
                progress,
                dist,
                radius,
            } => {
                extras[0] = noise;
                extras[1] = mask;
                p[0] = [progress, 0.0, 0.0, 0.0];
                p[1] = dist.extend(0.0).to_array();
                p[2] = radius.extend(0.0).to_array();
                K_EXPANDING
            }
            Shading::WhiteToColour => K_WHITE_TO_COL,
            Shading::WallMasked { platform } => {
                extras[0] = platform;
                K_WALL_MASKED
            }
            Shading::SingleChannel { mask } => {
                p[0] = mask.extend(0.0).to_array();
                K_SINGLE_CHANNEL
            }
            Shading::Channels3 { red, green, blue } => {
                p[0] = red.to_array();
                p[1] = green.to_array();
                p[2] = blue.to_array();
                K_CHANNELS3
            }
            Shading::ResizablePlatform {
                half,
                radius,
                border,
            } => {
                p[0] = [half.x, half.y, radius, border];
                K_RPLATFORM
            }
            Shading::Lightning(l) => {
                p[0] = [l.scale, l.uv_min, l.uv_max, l.blur];
                p[1] = [l.directions, l.quality, 0.0, 0.0];
                K_LIGHTNING
            }
            Shading::Beam(b) => {
                extras[0] = b.noise;
                extras[1] = b.platform;
                p[0] = [b.kind as u32 as f32, b.line_width, b.noise_freq, b.scale];
                p[1] = [b.aura, b.time_scale, b.dist_str, b.dist_str2];
                p[2] = [b.uv_ratio, b.uv_ratio2, 0.0, 0.0];
                K_BEAM
            }
            Shading::Clone {
                noise,
                progress,
                noise_str,
                noise_freq,
                direction,
                uv_offset,
            } => {
                extras[0] = noise;
                p[0] = [progress, noise_str, noise_freq, uv_offset];
                p[1] = [direction.x, direction.y, 0.0, 0.0];
                K_CLONE
            }
            Shading::ParticlePremultiply => K_PARTICLE_PREMUL,
            Shading::ParticleBlended { tint } => {
                p[0] = tint.to_array();
                K_PARTICLE_BLEND
            }
        };
        let inst = QuadInstance {
            origin: origin.extend(0.0).to_array(),
            axis_x: ax.extend(q.size.x).to_array(),
            axis_y: ay.extend(q.size.y).to_array(),
            uv,
            color: q.color.to_array(),
            p,
            kind: [kind, flags, 0, 0],
        };
        self.push_instance(pass, inst, q.texture, extras, q.blend, q.stencil);
    }

    fn push_text(&mut self, pass: usize, t: &Text, screen: bool, ppu: f32) {
        if t.text.is_empty() || t.size.is_nan() || t.size <= 0.0 {
            return;
        }
        let rot = Mat2::from_angle(t.rotation);
        let none = [TextureId::TRANSPARENT; 3];
        if self.sdf.contains(t.font) {
            let (quads, block) = self
                .sdf
                .layout_spaced(t.font, &t.text, t.size, t.anchor.x, t.spacing);
            let offset = -t.anchor * block;
            for q in quads {
                let g = q.glyph;
                let (aw, ah) = (g.atlas_size.0 as f32, g.atlas_size.1 as f32);
                let pad = g.padding;
                let [gx, gy, gw, gh] = g.rect;
                let (u0, v0, u1, v1) = (
                    (gx - pad) / aw,
                    (gy - pad) / ah,
                    (gx + gw + pad) / aw,
                    (gy + gh + pad) / ah,
                );
                let tl = q.top_left + offset;
                let (corner, uv) = if screen {
                    (tl, [u0, v0, u1, v1])
                } else {
                    (Vec2::new(tl.x, -tl.y - q.size.y), [u0, v1, u1, v0])
                };
                let origin = t.position + rot * corner;
                let xa = rot * Vec2::new(q.size.x, 0.0);
                let ya = rot * Vec2::new(0.0, q.size.y);
                let mut p = [[0.0f32; 4]; 6];
                let per_texel = 1.0 / (2.0 * (pad + 1.0));
                p[0] = [t.dilate * 0.5, per_texel, t.outline, 0.0];
                p[1] = t.outline_color.to_array();
                let inst = QuadInstance {
                    origin: [origin.x, origin.y, 0.0, 0.0],
                    axis_x: [xa.x, xa.y, 0.0, q.size.x],
                    axis_y: [ya.x, ya.y, 0.0, q.size.y],
                    uv,
                    color: t.color.to_array(),
                    p,
                    kind: [K_SDF, 0, 0, 0],
                };
                self.push_instance(pass, inst, g.atlas, none, Blend::Premultiplied, t.stencil);
            }
            return;
        }
        // Raster font.
        let px_f = if screen { t.size } else { t.size * ppu };
        if px_f.is_nan() || px_f < 1.0 {
            return;
        }
        let px = (px_f.round() as u32).clamp(1, MAX_GLYPH_PX);
        let scale = t.size / px as f32;
        let mut glyphs = std::mem::take(&mut self.glyphs);
        glyphs.clear();
        let block = self
            .text
            .layout(t.font, &t.text, px, t.anchor.x, &mut glyphs);
        let offset = -t.anchor * block;
        let mut base = t.position;
        if screen && t.rotation == 0.0 {
            base = (base + offset * scale).round() - offset * scale;
        }
        let inv = 1.0 / ATLAS_SIZE as f32;
        for g in &glyphs {
            let [ax, ay, w, h] = g.atlas.rect;
            let (wf, hf) = (w as f32 * scale, h as f32 * scale);
            let tl = (g.pos + offset) * scale;
            let (u0, v0, u1, v1) = (
                ax as f32 * inv,
                ay as f32 * inv,
                (ax + w) as f32 * inv,
                (ay + h) as f32 * inv,
            );
            let (corner, uv) = if screen {
                (tl, [u0, v0, u1, v1])
            } else {
                (Vec2::new(tl.x, -tl.y - hf), [u0, v1, u1, v0])
            };
            let origin = base + rot * corner;
            let xa = rot * Vec2::new(wf, 0.0);
            let ya = rot * Vec2::new(0.0, hf);
            let inst = QuadInstance {
                origin: [origin.x, origin.y, 0.0, 0.0],
                axis_x: [xa.x, xa.y, 0.0, wf],
                axis_y: [ya.x, ya.y, 0.0, hf],
                uv,
                color: t.color.to_array(),
                p: [[0.0; 4]; 6],
                kind: [K_SPRITE, 0, 0, 0],
            };
            self.push_instance(
                pass,
                inst,
                TextureId::GLYPHS,
                none,
                Blend::Premultiplied,
                t.stencil,
            );
        }
        self.glyphs = glyphs;
    }
}

/// Mat4 for a quad's transform from position, rotation (radians) and scale.
pub fn trs(position: Vec3, rotation: f32, scale: Vec2) -> Mat4 {
    Mat4::from_scale_rotation_translation(
        scale.extend(1.0),
        glam::Quat::from_rotation_z(rotation),
        position,
    )
}
