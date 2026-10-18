//! Colours.
//!
//! Bopl Battle runs in Unity's *gamma* colour space (`m_ActiveColorSpace = 0`):
//! colours, texture texels and blending all use the stored sRGB-encoded values
//! directly, without any linearisation. [`Color`] holds those values (what a
//! Unity `Color` field gives you) and the renderer passes them to the GPU
//! unchanged.

/// An sRGB-encoded colour with straight (non-premultiplied) alpha, like
/// Unity's `Color`. Components are nominally 0..1;
/// values above 1 are allowed for HDR tints (light colours, glows).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const WHITE: Color = Color::rgb(1.0, 1.0, 1.0);
    pub const BLACK: Color = Color::rgb(0.0, 0.0, 0.0);
    pub const TRANSPARENT: Color = Color::rgba(0.0, 0.0, 0.0, 0.0);
    pub const RED: Color = Color::rgb(1.0, 0.0, 0.0);
    pub const GREEN: Color = Color::rgb(0.0, 1.0, 0.0);
    pub const BLUE: Color = Color::rgb(0.0, 0.0, 1.0);

    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self::rgba(r, g, b, 1.0)
    }

    /// From 8-bit sRGB components.
    pub fn rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self::rgba(
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            a as f32 / 255.0,
        )
    }

    /// From `0xRRGGBB` (opaque).
    pub fn hex(rgb: u32) -> Self {
        Self::rgba8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 255)
    }

    pub fn with_alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }

    /// Linear interpolation in sRGB space (like Unity's `Color.Lerp`).
    pub fn lerp(self, o: Color, t: f32) -> Self {
        Self::rgba(
            self.r + (o.r - self.r) * t,
            self.g + (o.g - self.g) * t,
            self.b + (o.b - self.b) * t,
            self.a + (o.a - self.a) * t,
        )
    }

    /// From a Unity `[r, g, b, a]` array.
    pub fn from_array(c: [f32; 4]) -> Self {
        Self::rgba(c[0], c[1], c[2], c[3])
    }

    /// `[r, g, b, a]` as stored (gamma space).
    pub fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }

    /// Premultiplied by alpha, as stored (gamma space): what the shaders get.
    pub fn premul(self) -> [f32; 4] {
        [self.r * self.a, self.g * self.a, self.b * self.a, self.a]
    }

    /// Linear-light RGB with straight alpha.
    pub fn to_linear(self) -> [f32; 4] {
        [
            srgb_to_linear(self.r),
            srgb_to_linear(self.g),
            srgb_to_linear(self.b),
            self.a,
        ]
    }

    /// Linear-light RGB premultiplied by alpha: the form every shader uses.
    pub fn to_linear_premul(self) -> [f32; 4] {
        let [r, g, b, a] = self.to_linear();
        [r * a, g * a, b * a, a]
    }
}

/// Componentwise multiply (like Unity's `Color * Color`).
impl std::ops::Mul for Color {
    type Output = Color;
    fn mul(self, o: Color) -> Color {
        Color::rgba(self.r * o.r, self.g * o.g, self.b * o.b, self.a * o.a)
    }
}

/// sRGB transfer function, decoding (values above 1 are extended linearly
/// along the curve's power segment so HDR tints stay monotonic).
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// sRGB transfer function, encoding.
pub fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_roundtrip() {
        for i in 0..=255 {
            let c = i as f32 / 255.0;
            assert!((linear_to_srgb(srgb_to_linear(c)) - c).abs() < 1e-5);
        }
        assert!((srgb_to_linear(0.5) - 0.214).abs() < 1e-3);
    }

    #[test]
    fn premul() {
        let c = Color::rgba(1.0, 0.5, 0.0, 0.5).to_linear_premul();
        assert_eq!(c[0], 0.5);
        assert!((c[1] - 0.107).abs() < 1e-3);
        assert_eq!(c[3], 0.5);
        assert_eq!(Color::hex(0xff8000), Color::rgba8(255, 128, 0, 255));
    }
}
