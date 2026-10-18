//! Offscreen rendering of known scenes, checked pixel by pixel.
//!
//! Needs a GPU adapter (any Vulkan/GL device, or lavapipe). Without one the
//! tests print a note and pass, so machines without GPUs are not blocked.

use bopl_render::*;
use glam::{Vec2, Vec3};

const W: u32 = 160;
const H: u32 = 90;

fn setup() -> Option<(Gpu, Renderer, OffscreenTarget)> {
    let gpu = match Gpu::headless() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("skipping: {e}");
            return None;
        }
    };
    let renderer = Renderer::new(&gpu, OFFSCREEN_FORMAT);
    let target = OffscreenTarget::new(&gpu, (W, H), OFFSCREEN_FORMAT);
    Some((gpu, renderer, target))
}

fn close(a: [u8; 4], b: [u8; 4], tol: i32) -> bool {
    a.iter()
        .zip(b)
        .all(|(&x, y)| (x as i32 - y as i32).abs() <= tol)
}

#[track_caller]
fn assert_px(img: &Image, x: u32, y: u32, want: [u8; 4]) {
    let got = img.pixel(x, y);
    assert!(
        close(got, want, 2),
        "pixel ({x},{y}) = {got:?}, want {want:?}"
    );
}

fn save(img: &Image, name: &str) {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    img.save_png(dir.join(name)).unwrap();
}

/// 10 px per world unit, origin at the centre of the 160x90 target.
fn main_pass() -> Pass {
    let mut p = Pass::new(
        "main",
        PassTarget::Output,
        View::Camera(Camera::ortho(Vec2::ZERO, 4.5)),
    );
    p.clear = Some(Color::rgba8(32, 48, 64, 255));
    p
}

fn render(gpu: &Gpu, r: &mut Renderer, t: &OffscreenTarget, passes: Vec<Pass>) -> Image {
    let mut f = Frame::new();
    f.passes = passes;
    r.render(gpu, t.target(), &f);
    t.capture(gpu).unwrap()
}

#[test]
fn shapes_and_blending() {
    let Some((gpu, mut r, target)) = setup() else {
        return;
    };
    let mut p = main_pass();
    // Red 2x2 square centred at (-4, 0): pixels x 30..50, y 35..55.
    p.push(Quad::rect(
        Vec2::new(-4.0, 0.0),
        Vec2::splat(2.0),
        Color::RED,
    ));
    // Half-transparent white over it (premultiplied blend, gamma space).
    p.push(Quad::rect(
        Vec2::new(-4.0, 0.0),
        Vec2::new(2.0, 1.0),
        Color::rgba(1.0, 1.0, 1.0, 0.5),
    ));
    // Green circle radius 1.5 at (4, 1): centre pixel (120, 35).
    p.push(Quad::circle(Vec2::new(4.0, 1.0), 1.5, Color::GREEN));
    // Additive blue onto the background.
    p.push(Quad {
        blend: Blend::Additive,
        ..Quad::rect(
            Vec2::new(0.0, -3.0),
            Vec2::splat(1.0),
            Color::rgb(0.0, 0.0, 0.5),
        )
    });
    let img = render(&gpu, &mut r, &target, vec![p]);
    save(&img, "shapes.png");
    assert_px(&img, 40, 38, [255, 0, 0, 255]);
    assert_px(&img, 40, 45, [255, 128, 128, 255]);
    assert_px(&img, 120, 35, [0, 255, 0, 255]);
    assert_px(&img, 10, 10, [32, 48, 64, 255]);
    assert_px(&img, 80, 75, [32, 48, 64 + 128, 255]);
}

#[test]
fn gradient_inverse_alpha() {
    let Some((gpu, mut r, target)) = setup() else {
        return;
    };
    let mut p = main_pass();
    p.clear = Some(Color::rgb(0.0, 0.0, 1.0));
    // Full-screen gradient: alpha 1 at the bottom keeps the background,
    // alpha 0 at the top shows the gradient colour.
    p.push(Quad {
        size: Vec2::new(16.0, 9.0),
        shading: Shading::Gradient {
            top: Color::RED,
            bottom: Color::GREEN,
            alpha_top: 0.0,
            alpha_bottom: 1.0,
            height: 0.0,
        },
        blend: Blend::InverseAlpha,
        ..Default::default()
    });
    let img = render(&gpu, &mut r, &target, vec![p]);
    let top = img.pixel(80, 0);
    let bottom = img.pixel(80, H - 1);
    assert!(top[0] > 240 && top[2] < 15, "{top:?}");
    assert!(bottom[2] > 240 && bottom[1] < 15, "{bottom:?}");
}

#[test]
fn sprite_mask_stencil() {
    let Some((gpu, mut r, target)) = setup() else {
        return;
    };
    let mut p = main_pass();
    // Mask over the lower half of the screen.
    let mask = Quad {
        transform: glam::Mat4::from_translation(Vec3::new(0.0, -2.25, 0.0)),
        size: Vec2::new(16.0, 4.5),
        shading: Shading::Mask { cutoff: 0.2 },
        stencil: Stencil::MaskAdd,
        ..Default::default()
    };
    p.push(mask);
    // A white bar crossing the mask edge, visible only outside the mask.
    p.push(Quad {
        stencil: Stencil::VisibleOutside,
        ..Quad::rect(Vec2::new(-4.0, 0.0), Vec2::new(2.0, 6.0), Color::WHITE)
    });
    // A yellow bar visible only inside.
    p.push(Quad {
        stencil: Stencil::VisibleInside,
        ..Quad::rect(
            Vec2::new(4.0, 0.0),
            Vec2::new(2.0, 6.0),
            Color::rgb(1.0, 1.0, 0.0),
        )
    });
    p.push(Quad {
        stencil: Stencil::MaskRemove,
        ..mask
    });
    // After removing the mask, "outside" draws everywhere again.
    p.push(Quad {
        stencil: Stencil::VisibleOutside,
        ..Quad::rect(Vec2::new(0.0, -3.0), Vec2::new(1.0, 1.0), Color::RED)
    });
    let img = render(&gpu, &mut r, &target, vec![p]);
    save(&img, "mask.png");
    assert_px(&img, 40, 30, [255, 255, 255, 255]);
    assert_px(&img, 40, 60, [32, 48, 64, 255]);
    assert_px(&img, 120, 30, [32, 48, 64, 255]);
    assert_px(&img, 120, 60, [255, 255, 0, 255]);
    assert_px(&img, 80, 75, [255, 0, 0, 255]);
}

#[test]
fn render_target_feeds_a_later_pass() {
    let Some((gpu, mut r, target)) = setup() else {
        return;
    };
    let rt = r.render_target(&gpu, "_TestRT", (W, H), Wrap::Clamp);
    // Pass 1: a green left half into the render texture.
    let mut a = Pass::new(
        "rt",
        PassTarget::Texture(rt),
        View::Camera(Camera::ortho(Vec2::ZERO, 4.5)),
    );
    a.clear = Some(Color::TRANSPARENT);
    a.push(Quad::rect(
        Vec2::new(-4.0, 0.0),
        Vec2::new(8.0, 9.0),
        Color::GREEN,
    ));
    // Pass 2: the render texture as a full-screen sprite.
    let mut b = main_pass();
    b.push(Quad {
        texture: rt,
        size: Vec2::new(16.0, 9.0),
        ..Default::default()
    });
    let img = render(&gpu, &mut r, &target, vec![a, b]);
    assert_px(&img, 20, 45, [0, 255, 0, 255]);
    assert_px(&img, 140, 45, [32, 48, 64, 255]);
    // The render texture keeps its contents and can be read back.
    let back = r.capture(&gpu, rt).unwrap();
    assert_px(&back, 20, 45, [0, 255, 0, 255]);
    assert_eq!(back.pixel(140, 45)[3], 0);
}

#[test]
fn water_shows_reflection_below_horizon() {
    let Some((gpu, mut r, target)) = setup() else {
        return;
    };
    let cam = Camera::ortho(Vec2::ZERO, 4.5);
    let refl = r.render_target(&gpu, "_ReflectionRT", (W, H), Wrap::Repeat);
    let bg = r.render_target(&gpu, "_BackgroundRT", (W, H), Wrap::Mirror);
    let mut a = Pass::new("refl", PassTarget::Texture(refl), View::Camera(cam));
    a.clear = Some(Color::rgba(1.0, 1.0, 1.0, 0.0));
    a.push(Quad::rect(Vec2::ZERO, Vec2::new(16.0, 9.0), Color::RED));
    let mut b = Pass::new("bg", PassTarget::Texture(bg), View::Camera(cam));
    b.clear = Some(Color::rgb(0.0, 0.0, 1.0));
    let mut m = main_pass();
    m.push(Quad {
        size: Vec2::new(16.0, 9.0),
        shading: Shading::ReflectiveWater(WaterParams {
            alpha: 1.0,
            distortion: 0.0,
            horizon_height: 0.0,
            surface_gradient: 1000.0,
            extra_noise: 0.0,
            gradient_height: 0.0,
            horizon_gradient_height: -10.0,
            reflection_height: 0.0,
            gradient_top: Color::BLACK,
            gradient_bottom: Color::BLACK,
            water_color: Color::GREEN,
            wave: TextureId::TRANSPARENT,
            reflection: refl,
            background: bg,
        }),
        ..Default::default()
    });
    let img = render(&gpu, &mut r, &target, vec![a, b, m]);
    save(&img, "water.png");
    // Above the horizon (y = 0): the background texture; below: reflection.
    assert_px(&img, 80, 10, [0, 0, 255, 255]);
    assert_px(&img, 80, 80, [255, 0, 0, 255]);
}

#[test]
fn sdf_text_draws_dynamic_glyphs() {
    let Some((gpu, mut r, target)) = setup() else {
        return;
    };
    let font = r
        .add_dynamic_sdf_font(DynamicSdfDesc {
            ttf: DEFAULT_FONT_BYTES.to_vec(),
            point_size: 48.0,
            padding: 5.0,
            line_metrics: None,
        })
        .unwrap();
    let mut p = Pass::new("text", PassTarget::Output, View::Screen);
    p.clear = Some(Color::BLACK);
    p.push(Text {
        text: "BOPL".into(),
        font,
        size: 40.0,
        position: Vec2::new(80.0, 45.0),
        anchor: Vec2::splat(0.5),
        color: Color::WHITE,
        ..Default::default()
    });
    let img = render(&gpu, &mut r, &target, vec![p]);
    save(&img, "sdf.png");
    let lit = img.pixels.chunks(4).filter(|p| p[0] > 200).count();
    assert!(lit > 300, "{lit} lit pixels");
    assert_px(&img, 2, 2, [0, 0, 0, 255]);
    let size = r.measure_text(font, "BOPL", 40.0);
    assert!(size.x > 60.0 && size.x < 150.0, "{size}");
}
