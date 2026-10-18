//! Texture handles and the GPU texture store.

/// Handle to a texture owned by the [`Renderer`](crate::Renderer).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextureId(pub(crate) u32);

impl TextureId {
    /// A 1x1 opaque white texture (used by untextured shapes).
    pub const WHITE: TextureId = TextureId(0);
    /// The glyph atlas of the built-in text system.
    pub(crate) const GLYPHS: TextureId = TextureId(1);
    /// A 1x1 fully transparent texture (unbound optional inputs such as a
    /// platform's slime-trail render texture before anything was drawn).
    pub const TRANSPARENT: TextureId = TextureId(2);
    /// The atlas of dynamically generated SDF glyphs (TMP fonts whose glyphs
    /// are rasterised from the TTF at run time).
    pub(crate) const DYN_SDF: TextureId = TextureId(3);
    /// Number of built-in textures (ids below this are never freed).
    pub(crate) const BUILTIN: u32 = 4;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Filter {
    #[default]
    Linear,
    Nearest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Wrap {
    #[default]
    Clamp,
    Repeat,
    /// Mirrored repeat (Unity `TextureWrapMode.Mirror`).
    Mirror,
}

/// How a texture is sampled and interpreted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextureOptions {
    pub filter: Filter,
    pub wrap: Wrap,
    /// Decode sRGB to linear on sampling. Off by default: the original is
    /// a gamma-space project, so texture bytes are used as stored.
    pub srgb: bool,
}

impl Default for TextureOptions {
    fn default() -> Self {
        Self {
            filter: Filter::Linear,
            wrap: Wrap::Clamp,
            srgb: false,
        }
    }
}

impl TextureOptions {
    /// Options from Unity's texture settings (`filter_mode` 0 point, else
    /// bilinear; `wrap_mode` 0 repeat, 1 clamp, 2 mirror).
    pub fn unity(filter_mode: i32, wrap_mode: i32) -> Self {
        Self {
            filter: if filter_mode == 0 {
                Filter::Nearest
            } else {
                Filter::Linear
            },
            wrap: match wrap_mode {
                0 => Wrap::Repeat,
                2 | 3 => Wrap::Mirror,
                _ => Wrap::Clamp,
            },
            srgb: false,
        }
    }
}

pub(crate) struct GpuTexture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub bind_group: wgpu::BindGroup,
    pub size: (u32, u32),
    pub opts: TextureOptions,
    /// Bumped when the texture is replaced (render targets resized), so
    /// cached bind groups that reference it are rebuilt.
    pub generation: u32,
}

pub(crate) fn texture_format(opts: &TextureOptions) -> wgpu::TextureFormat {
    if opts.srgb {
        wgpu::TextureFormat::Rgba8UnormSrgb
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    }
}

pub(crate) fn sampler_desc(opts: &TextureOptions) -> wgpu::SamplerDescriptor<'static> {
    let filter = match opts.filter {
        Filter::Linear => wgpu::FilterMode::Linear,
        Filter::Nearest => wgpu::FilterMode::Nearest,
    };
    let wrap = match opts.wrap {
        Wrap::Clamp => wgpu::AddressMode::ClampToEdge,
        Wrap::Repeat => wgpu::AddressMode::Repeat,
        Wrap::Mirror => wgpu::AddressMode::MirrorRepeat,
    };
    wgpu::SamplerDescriptor {
        label: Some("texture sampler"),
        address_mode_u: wrap,
        address_mode_v: wrap,
        address_mode_w: wrap,
        mag_filter: filter,
        min_filter: filter,
        ..Default::default()
    }
}

/// Writes rows `[rows.0, rows.1)` of a full-width RGBA8 pixel buffer into
/// `texture`.
pub(crate) fn write_rows(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    pixels: &[u8],
    width: u32,
    rows: (u32, u32),
) {
    let (y0, y1) = rows;
    if y1 <= y0 {
        return;
    }
    let stride = width * 4;
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d { x: 0, y: y0, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        &pixels[(y0 * stride) as usize..(y1 * stride) as usize],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(stride),
            rows_per_image: Some(y1 - y0),
        },
        wgpu::Extent3d {
            width,
            height: y1 - y0,
            depth_or_array_layers: 1,
        },
    );
}
