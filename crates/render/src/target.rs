//! Render targets: window surfaces and offscreen textures (with capture).

use crate::RenderError;
use crate::gpu::Gpu;
use crate::image::Image;

/// The format offscreen targets and render textures use: plain 8-bit RGBA
/// without sRGB conversion. Bopl Battle is a *gamma-space* Unity project
/// (`m_ActiveColorSpace` 0): colours, textures and blending all work on the
/// stored (sRGB-encoded) values directly, so no conversion happens anywhere.
pub const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// A borrowed colour target for one [`Renderer::render`](crate::Renderer::render) call.
#[derive(Clone, Copy)]
pub struct TargetView<'a> {
    pub view: &'a wgpu::TextureView,
    pub size: (u32, u32),
    pub format: wgpu::TextureFormat,
}

/// An offscreen colour texture that can be rendered to, captured to an
/// [`Image`], and sampled as a texture.
pub struct OffscreenTarget {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub size: (u32, u32),
    pub format: wgpu::TextureFormat,
}

impl OffscreenTarget {
    pub fn new(gpu: &Gpu, size: (u32, u32), format: wgpu::TextureFormat) -> Self {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen target"),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Self {
            texture,
            view,
            size,
            format,
        }
    }

    pub fn target(&self) -> TargetView<'_> {
        TargetView {
            view: &self.view,
            size: self.size,
            format: self.format,
        }
    }

    /// Copies the texture back to the CPU as RGBA8 (bytes as stored).
    /// Blocks until the GPU is done.
    pub fn capture(&self, gpu: &Gpu) -> Result<Image, RenderError> {
        capture_texture(gpu, &self.texture, self.size, self.format)
    }
}

/// Copies a texture (with `COPY_SRC` usage) back to the CPU as RGBA8.
pub(crate) fn capture_texture(
    gpu: &Gpu,
    texture: &wgpu::Texture,
    size: (u32, u32),
    format: wgpu::TextureFormat,
) -> Result<Image, RenderError> {
    let (w, h) = size;
    let bgra = match format {
        wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb => false,
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => true,
        f => return Err(RenderError::Unsupported(format!("capture from {f:?}"))),
    };
    let row = w * 4;
    let padded =
        row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("capture buffer"),
        size: (padded * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit([enc.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    buffer.map_async(wgpu::MapMode::Read, .., move |r| {
        let _ = tx.send(r);
    });
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| RenderError::Device(e.to_string()))?;
    rx.recv()
        .map_err(|e| RenderError::Device(e.to_string()))?
        .map_err(|e| RenderError::Device(e.to_string()))?;
    let mut pixels = Vec::with_capacity((row * h) as usize);
    {
        let data = buffer
            .get_mapped_range(..)
            .map_err(|e| RenderError::Device(format!("{e:?}")))?;
        for y in 0..h {
            let start = (y * padded) as usize;
            pixels.extend_from_slice(&data[start..start + row as usize]);
        }
    }
    buffer.unmap();
    if bgra {
        for px in pixels.as_chunks_mut::<4>().0 {
            px.swap(0, 2);
        }
    }
    Ok(Image::from_pixels(w, h, pixels))
}

/// A configured window surface.
pub struct SurfaceTarget {
    pub surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,
    /// The last image was suboptimal: reconfigure before the next one (a
    /// surface must not be configured while one of its images is alive).
    stale: bool,
}

/// A swapchain image acquired for one frame; call [`SurfaceFrame::present`]
/// after rendering.
pub struct SurfaceFrame {
    texture: wgpu::SurfaceTexture,
    pub view: wgpu::TextureView,
    pub size: (u32, u32),
    pub format: wgpu::TextureFormat,
}

impl SurfaceFrame {
    pub fn target(&self) -> TargetView<'_> {
        TargetView {
            view: &self.view,
            size: self.size,
            format: self.format,
        }
    }

    pub fn present(self, gpu: &Gpu) {
        gpu.queue.present(self.texture);
    }
}

impl SurfaceTarget {
    /// Configures `surface` for `size`. Prefers a non-sRGB 8-bit format
    /// (gamma-space output, see [`OFFSCREEN_FORMAT`]); `vsync` selects FIFO,
    /// otherwise Mailbox/Immediate when available.
    pub fn new(
        gpu: &Gpu,
        surface: wgpu::Surface<'static>,
        size: (u32, u32),
        vsync: bool,
    ) -> Result<Self, RenderError> {
        let caps = surface.get_capabilities(&gpu.adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| {
                matches!(
                    f,
                    wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm
                )
            })
            .or_else(|| caps.formats.iter().copied().find(|f| !f.is_srgb()))
            .or_else(|| caps.formats.first().copied())
            .ok_or_else(|| RenderError::Unsupported("surface has no formats".into()))?;
        let mut config = surface
            .get_default_config(&gpu.adapter, size.0.max(1), size.1.max(1))
            .ok_or_else(|| RenderError::Unsupported("surface not supported by adapter".into()))?;
        config.format = format;
        config.present_mode = choose_present_mode(&caps.present_modes, vsync);
        config.desired_maximum_frame_latency = 2;
        surface.configure(&gpu.device, &config);
        Ok(Self {
            surface,
            config,
            stale: false,
        })
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    /// The configured size in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    pub fn resize(&mut self, gpu: &Gpu, size: (u32, u32)) {
        if size.0 == 0 || size.1 == 0 || size == (self.config.width, self.config.height) {
            return;
        }
        self.config.width = size.0;
        self.config.height = size.1;
        self.surface.configure(&gpu.device, &self.config);
    }

    pub fn set_vsync(&mut self, gpu: &Gpu, vsync: bool) {
        let caps = self.surface.get_capabilities(&gpu.adapter);
        self.config.present_mode = choose_present_mode(&caps.present_modes, vsync);
        self.surface.configure(&gpu.device, &self.config);
    }

    /// Acquires the next image; `None` when the frame should be skipped
    /// (minimised, timeout, or the surface had to be reconfigured).
    pub fn acquire(&mut self, gpu: &Gpu) -> Option<SurfaceFrame> {
        if std::mem::take(&mut self.stale) {
            self.surface.configure(&gpu.device, &self.config);
        }
        let texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => t,
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => {
                // Use it, but reconfigure before the next frame (after this
                // image was presented).
                self.stale = true;
                t
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return None;
            }
            other => {
                log::warn!("surface lost or outdated ({other:?}); reconfiguring");
                self.surface.configure(&gpu.device, &self.config);
                return None;
            }
        };
        Some(self.wrap(texture))
    }

    fn wrap(&self, texture: wgpu::SurfaceTexture) -> SurfaceFrame {
        let view = texture.texture.create_view(&Default::default());
        SurfaceFrame {
            texture,
            view,
            size: (self.config.width, self.config.height),
            format: self.config.format,
        }
    }
}

fn choose_present_mode(modes: &[wgpu::PresentMode], vsync: bool) -> wgpu::PresentMode {
    if vsync {
        return wgpu::PresentMode::Fifo;
    }
    [wgpu::PresentMode::Mailbox, wgpu::PresentMode::Immediate]
        .into_iter()
        .find(|m| modes.contains(m))
        .unwrap_or(wgpu::PresentMode::Fifo)
}
