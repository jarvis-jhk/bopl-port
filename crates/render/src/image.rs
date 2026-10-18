//! CPU-side RGBA8 images and PNG I/O.

use std::io::{BufReader, BufWriter};
use std::path::Path;

use crate::RenderError;

/// An RGBA8 image, rows top to bottom (PNG order). For colour textures the
/// bytes are sRGB-encoded with straight alpha.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Image {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![0; (width * height * 4) as usize],
        }
    }

    pub fn from_pixels(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        assert_eq!(
            pixels.len(),
            (width * height * 4) as usize,
            "pixel buffer size"
        );
        Self {
            width,
            height,
            pixels,
        }
    }

    /// A solid colour image (sRGB bytes).
    pub fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Self {
        let mut img = Self::new(width, height);
        for px in img.pixels.as_chunks_mut::<4>().0 {
            px.copy_from_slice(&rgba);
        }
        img
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        self.pixels[i..i + 4].try_into().unwrap()
    }

    pub fn set_pixel(&mut self, x: u32, y: u32, rgba: [u8; 4]) {
        let i = ((y * self.width + x) * 4) as usize;
        self.pixels[i..i + 4].copy_from_slice(&rgba);
    }

    /// Writes an 8-bit RGBA PNG.
    pub fn save_png(&self, path: impl AsRef<Path>) -> Result<(), RenderError> {
        let path = path.as_ref();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let file = std::fs::File::create(path)?;
        self.encode_png(BufWriter::new(file))
    }

    /// Encodes as an 8-bit RGBA PNG into any writer.
    pub fn encode_png<W: std::io::Write>(&self, w: W) -> Result<(), RenderError> {
        let mut enc = png::Encoder::new(w, self.width, self.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        let mut writer = enc
            .write_header()
            .map_err(|e| RenderError::Png(e.to_string()))?;
        writer
            .write_image_data(&self.pixels)
            .map_err(|e| RenderError::Png(e.to_string()))?;
        Ok(())
    }

    /// Reads a PNG of any 8-bit colour type and expands it to RGBA8.
    pub fn load_png(path: impl AsRef<Path>) -> Result<Self, RenderError> {
        let file = std::fs::File::open(path)?;
        Self::decode_png(BufReader::new(file))
    }

    /// Decodes PNG data (8-bit greyscale/RGB/RGBA, palettes expanded).
    pub fn decode_png<R: std::io::BufRead + std::io::Seek>(r: R) -> Result<Self, RenderError> {
        let err = |e: png::DecodingError| RenderError::Png(e.to_string());
        let mut dec = png::Decoder::new(r);
        dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = dec.read_info().map_err(err)?;
        let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
        let info = reader.next_frame(&mut buf).map_err(err)?;
        let (w, h) = (info.width, info.height);
        let src = &buf[..info.buffer_size()];
        let pixels = match info.color_type {
            png::ColorType::Rgba => src.to_vec(),
            png::ColorType::Rgb => src
                .as_chunks::<3>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1], p[2], 255])
                .collect(),
            png::ColorType::GrayscaleAlpha => src
                .as_chunks::<2>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[0], p[0], p[1]])
                .collect(),
            png::ColorType::Grayscale => src.iter().flat_map(|&g| [g, g, g, 255]).collect(),
            png::ColorType::Indexed => return Err(RenderError::Png("unexpanded palette".into())),
        };
        Ok(Self::from_pixels(w, h, pixels))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_roundtrip() {
        let mut img = Image::solid(3, 2, [10, 20, 30, 255]);
        img.set_pixel(2, 1, [255, 0, 128, 64]);
        let mut bytes = Vec::new();
        img.encode_png(&mut bytes).unwrap();
        let back = Image::decode_png(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(img, back);
    }
}
