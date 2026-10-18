//! `bopl-render`: the wgpu renderer of bopl-port.
//!
//! The presentation builds a [`Frame`] each frame: one [`Pass`] per camera
//! of the original (background, reflection, wave particles, platform mask,
//! slime trails, main camera, overlays), each with its items already sorted
//! like Unity sorts sprites. [`Renderer::render`] draws the passes in order
//! into their targets: the output (a window surface, [`SurfaceTarget`], or
//! an [`OffscreenTarget`] that can be captured to an [`Image`]) or named
//! render textures ([`Renderer::render_target`]) that later passes sample.
//! The same code path runs headless for screenshots and tests.
//!
//! Conventions: world space is y-up with Unity cameras ([`Camera`]);
//! [`View::Screen`] passes use pixels with a top-left origin. Bopl Battle is
//! a gamma-space Unity project, so colours and texels are used as stored
//! (no sRGB conversion) and blending happens on those values.
//!
//! ```no_run
//! use bopl_render::*;
//! use glam::Vec2;
//! let gpu = Gpu::headless().unwrap();
//! let mut renderer = Renderer::new(&gpu, OFFSCREEN_FORMAT);
//! let target = OffscreenTarget::new(&gpu, (640, 360), OFFSCREEN_FORMAT);
//! let mut pass = Pass::new("main", PassTarget::Output, View::Camera(Camera::ortho(Vec2::ZERO, 5.0)));
//! pass.clear = Some(Color::hex(0x202030));
//! pass.push(Quad::circle(Vec2::ZERO, 1.0, Color::hex(0xff8800)));
//! let mut frame = Frame::new();
//! frame.passes.push(pass);
//! renderer.render(&gpu, target.target(), &frame);
//! target.capture(&gpu).unwrap().save_png("scratch/example.png").unwrap();
//! ```

mod camera;
mod color;
mod draw;
mod gpu;
mod image;
mod renderer;
mod sdf;
mod target;
mod text;
mod texture;

pub use camera::{Camera, Projection, screen_proj};
pub use color::{Color, linear_to_srgb, srgb_to_linear};
pub use draw::{
    AuroraParams, BeamKind, BeamParams, Blend, DistortionParams, Frame, Item, LightningParams,
    Pass, PassTarget, Quad, Shading, SpriteDistortParams, Stencil, Text, UvRect, View, WaterParams,
    mat4_from_affine2,
};
pub use gpu::{Gpu, new_instance};
pub use image::Image;
pub use renderer::{FrameStats, Renderer, trs};
pub use sdf::{DynamicSdfDesc, SdfFontDesc, SdfGlyph};
pub use target::{OFFSCREEN_FORMAT, OffscreenTarget, SurfaceFrame, SurfaceTarget, TargetView};
pub use text::{DEFAULT_FONT_BYTES, FontId};
pub use texture::{Filter, TextureId, TextureOptions, Wrap};

/// Re-export so dependents use the same wgpu version.
pub use wgpu;

/// Renderer errors.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("no suitable GPU adapter: {0}")]
    NoAdapter(String),
    #[error("GPU device error: {0}")]
    Device(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("unknown texture id")]
    BadTexture,
    #[error("font error: {0}")]
    Font(String),
    #[error("PNG error: {0}")]
    Png(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
