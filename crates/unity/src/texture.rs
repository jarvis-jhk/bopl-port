//! Texture decoding to RGBA8, rows top-down.
//!
//! Unity stores texture rows bottom-up; [`decode`] flips them so row 0 is the
//! top of the image, as image files expect. Only the first mip level is
//! decoded. Supported formats (Unity `TextureFormat` values): Alpha8 (1),
//! ARGB4444 (2), RGB24 (3), RGBA32 (4), ARGB32 (5), RGB565 (7), DXT1/BC1 (10),
//! DXT5/BC3 (12), RGBA4444 (13), BGRA32 (14), BC7 (25, via `bcdec_rs`),
//! BC4 (26), BC5 (27), RG16 (62), R8 (63).

use crate::{Error, Result};

/// Unity `TextureFormat` values handled here.
pub mod format {
    #![allow(missing_docs)]
    pub const ALPHA8: i32 = 1;
    pub const ARGB4444: i32 = 2;
    pub const RGB24: i32 = 3;
    pub const RGBA32: i32 = 4;
    pub const ARGB32: i32 = 5;
    pub const RGB565: i32 = 7;
    pub const DXT1: i32 = 10;
    pub const DXT5: i32 = 12;
    pub const RGBA4444: i32 = 13;
    pub const BGRA32: i32 = 14;
    pub const BC7: i32 = 25;
    pub const BC4: i32 = 26;
    pub const BC5: i32 = 27;
    pub const RG16: i32 = 62;
    pub const R8: i32 = 63;
}

/// Human-readable name of a texture format.
pub fn format_name(f: i32) -> &'static str {
    use format::*;
    match f {
        ALPHA8 => "Alpha8",
        ARGB4444 => "ARGB4444",
        RGB24 => "RGB24",
        RGBA32 => "RGBA32",
        ARGB32 => "ARGB32",
        RGB565 => "RGB565",
        DXT1 => "DXT1",
        DXT5 => "DXT5",
        RGBA4444 => "RGBA4444",
        BGRA32 => "BGRA32",
        BC7 => "BC7",
        BC4 => "BC4",
        BC5 => "BC5",
        RG16 => "RG16",
        R8 => "R8",
        _ => "unknown",
    }
}

/// Size in bytes of the first mip level.
pub fn level0_size(fmt: i32, w: u32, h: u32) -> Option<usize> {
    use format::*;
    let (w, h) = (w as usize, h as usize);
    let blocks = w.div_ceil(4) * h.div_ceil(4);
    Some(match fmt {
        ALPHA8 | R8 => w * h,
        ARGB4444 | RGBA4444 | RGB565 | RG16 => w * h * 2,
        RGB24 => w * h * 3,
        RGBA32 | ARGB32 | BGRA32 => w * h * 4,
        DXT1 | BC4 => blocks * 8,
        DXT5 | BC5 | BC7 => blocks * 16,
        _ => return None,
    })
}

/// Decodes the first mip level of `data` into RGBA8, top row first.
pub fn decode(fmt: i32, w: u32, h: u32, data: &[u8]) -> Result<Vec<u8>> {
    use format::*;
    let need = level0_size(fmt, w, h)
        .ok_or_else(|| Error::Unsupported(format!("texture format {fmt}")))?;
    if data.len() < need {
        return Err(Error::Format(format!(
            "texture data {} bytes, {need} needed for {w}x{h} {}",
            data.len(),
            format_name(fmt)
        )));
    }
    let (wu, hu) = (w as usize, h as usize);
    let mut out = vec![0u8; wu * hu * 4];
    let px = |out: &mut [u8], f: &dyn Fn(usize) -> [u8; 4]| {
        for (i, p) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            *p = f(i);
        }
    };
    let ex4 = |v: u16| (v as u8 & 0xf) * 17;
    match fmt {
        ALPHA8 => px(&mut out, &|i| [255, 255, 255, data[i]]),
        R8 => px(&mut out, &|i| [data[i], 0, 0, 255]),
        RG16 => px(&mut out, &|i| [data[2 * i], data[2 * i + 1], 0, 255]),
        RGB24 => px(&mut out, &|i| {
            [data[3 * i], data[3 * i + 1], data[3 * i + 2], 255]
        }),
        RGBA32 => out.copy_from_slice(&data[..need]),
        ARGB32 => px(&mut out, &|i| {
            let s = &data[4 * i..];
            [s[1], s[2], s[3], s[0]]
        }),
        BGRA32 => px(&mut out, &|i| {
            let s = &data[4 * i..];
            [s[2], s[1], s[0], s[3]]
        }),
        RGB565 => px(&mut out, &|i| {
            let c = u16::from_le_bytes([data[2 * i], data[2 * i + 1]]);
            let [r, g, b] = rgb565(c);
            [r, g, b, 255]
        }),
        ARGB4444 => px(&mut out, &|i| {
            let c = u16::from_le_bytes([data[2 * i], data[2 * i + 1]]);
            [ex4(c >> 8), ex4(c >> 4), ex4(c), ex4(c >> 12)]
        }),
        RGBA4444 => px(&mut out, &|i| {
            let c = u16::from_le_bytes([data[2 * i], data[2 * i + 1]]);
            [ex4(c >> 12), ex4(c >> 8), ex4(c >> 4), ex4(c)]
        }),
        DXT1 | DXT5 | BC4 | BC5 => decode_blocks(fmt, wu, hu, data, &mut out),
        BC7 => decode_bc7(wu, hu, data, &mut out),
        _ => unreachable!(),
    }
    flip_rows(&mut out, wu * 4);
    Ok(out)
}

fn rgb565(c: u16) -> [u8; 3] {
    let r = ((c >> 11) & 31) as u8;
    let g = ((c >> 5) & 63) as u8;
    let b = (c & 31) as u8;
    [
        (r << 3) | (r >> 2),
        (g << 2) | (g >> 4),
        (b << 3) | (b >> 2),
    ]
}

/// BC1 colour block: 4 RGBA8 palette entries. `force4` = BC2/BC3 semantics.
fn bc1_palette(b: &[u8], force4: bool) -> [[u8; 4]; 4] {
    let c0 = u16::from_le_bytes([b[0], b[1]]);
    let c1 = u16::from_le_bytes([b[2], b[3]]);
    let (p0, p1) = (rgb565(c0), rgb565(c1));
    let mix = |a: u8, b: u8, wa: u16, wb: u16, d: u16| {
        ((a as u16 * wa + b as u16 * wb + d / 2) / d) as u8
    };
    let mut pal = [[0u8; 4]; 4];
    pal[0] = [p0[0], p0[1], p0[2], 255];
    pal[1] = [p1[0], p1[1], p1[2], 255];
    if c0 > c1 || force4 {
        for k in 0..3 {
            pal[2][k] = mix(p0[k], p1[k], 2, 1, 3);
            pal[3][k] = mix(p0[k], p1[k], 1, 2, 3);
        }
        pal[2][3] = 255;
        pal[3][3] = 255;
    } else {
        for k in 0..3 {
            pal[2][k] = mix(p0[k], p1[k], 1, 1, 2);
        }
        pal[2][3] = 255;
        pal[3] = [0, 0, 0, 0];
    }
    pal
}

/// BC4-style 8-value channel block (used for BC3 alpha, BC4, BC5).
fn bc4_values(b: &[u8]) -> ([u8; 8], u64) {
    let (a0, a1) = (b[0] as u32, b[1] as u32);
    let mut v = [0u8; 8];
    v[0] = a0 as u8;
    v[1] = a1 as u8;
    if a0 > a1 {
        for i in 1..7u32 {
            v[i as usize + 1] = (((7 - i) * a0 + i * a1 + 3) / 7) as u8;
        }
    } else {
        for i in 1..5u32 {
            v[i as usize + 1] = (((5 - i) * a0 + i * a1 + 2) / 5) as u8;
        }
        v[6] = 0;
        v[7] = 255;
    }
    let mut bits = 0u64;
    for (i, &x) in b[2..8].iter().enumerate() {
        bits |= (x as u64) << (8 * i);
    }
    (v, bits)
}

fn decode_blocks(fmt: i32, w: usize, h: usize, data: &[u8], out: &mut [u8]) {
    let bsize = if matches!(fmt, format::DXT1 | format::BC4) {
        8
    } else {
        16
    };
    let bw = w.div_ceil(4);
    for (bi, b) in data
        .chunks_exact(bsize)
        .take(bw * h.div_ceil(4))
        .enumerate()
    {
        let (bx, by) = (bi % bw * 4, bi / bw * 4);
        let mut texels = [[0u8; 4]; 16];
        match fmt {
            format::DXT1 | format::DXT5 => {
                let cb = if fmt == format::DXT1 { b } else { &b[8..] };
                let pal = bc1_palette(cb, fmt == format::DXT5);
                let idx = u32::from_le_bytes([cb[4], cb[5], cb[6], cb[7]]);
                for (t, texel) in texels.iter_mut().enumerate() {
                    *texel = pal[((idx >> (2 * t)) & 3) as usize];
                }
                if fmt == format::DXT5 {
                    let (av, bits) = bc4_values(b);
                    for (t, texel) in texels.iter_mut().enumerate() {
                        texel[3] = av[((bits >> (3 * t)) & 7) as usize];
                    }
                }
            }
            _ => {
                let (rv, rb) = bc4_values(b);
                let g = (fmt == format::BC5).then(|| bc4_values(&b[8..]));
                for (t, texel) in texels.iter_mut().enumerate() {
                    let r = rv[((rb >> (3 * t)) & 7) as usize];
                    let gg = g.map_or(0, |(gv, gb)| gv[((gb >> (3 * t)) & 7) as usize]);
                    *texel = [r, gg, 0, 255];
                }
            }
        }
        for (t, texel) in texels.iter().enumerate() {
            let (x, y) = (bx + t % 4, by + t / 4);
            if x < w && y < h {
                out[(y * w + x) * 4..][..4].copy_from_slice(texel);
            }
        }
    }
}

/// BC7 blocks (mode 0..7) through `bcdec_rs`, clipped at the image edges.
fn decode_bc7(w: usize, h: usize, data: &[u8], out: &mut [u8]) {
    let bw = w.div_ceil(4);
    let mut block = [0u8; 64];
    for (bi, b) in data
        .as_chunks::<16>()
        .0
        .iter()
        .take(bw * h.div_ceil(4))
        .enumerate()
    {
        let (bx, by) = (bi % bw * 4, bi / bw * 4);
        bcdec_rs::bc7(b, &mut block, 16);
        for row in 0..4 {
            let y = by + row;
            if y >= h {
                break;
            }
            let n = 4.min(w - bx);
            out[(y * w + bx) * 4..][..n * 4].copy_from_slice(&block[row * 16..][..n * 4]);
        }
    }
}

/// Reverses the order of rows of `stride` bytes.
pub fn flip_rows(buf: &mut [u8], stride: usize) {
    let rows = buf.len() / stride;
    for y in 0..rows / 2 {
        let (top, bottom) = buf.split_at_mut((rows - 1 - y) * stride);
        top[y * stride..][..stride].swap_with_slice(&mut bottom[..stride]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba32_is_flipped() {
        // 1x2: bottom row red, top row blue (Unity order: bottom first).
        let data = [255, 0, 0, 255, 0, 0, 255, 255];
        let out = decode(format::RGBA32, 1, 2, &data).unwrap();
        assert_eq!(out, [0, 0, 255, 255, 255, 0, 0, 255]);
    }

    #[test]
    fn alpha8_and_rgb24() {
        assert_eq!(
            decode(format::ALPHA8, 1, 1, &[7]).unwrap(),
            [255, 255, 255, 7]
        );
        assert_eq!(
            decode(format::RGB24, 1, 1, &[1, 2, 3]).unwrap(),
            [1, 2, 3, 255]
        );
        assert!(decode(format::RGB24, 2, 2, &[0; 3]).is_err());
        assert!(decode(999, 1, 1, &[0; 64]).is_err());
    }

    #[test]
    fn dxt1_solid_and_transparent() {
        // c0 = white, c1 = black, all indices 0 -> white.
        let block = [0xff, 0xff, 0, 0, 0, 0, 0, 0];
        let out = decode(format::DXT1, 4, 4, &block).unwrap();
        assert!(out.chunks(4).all(|p| p == [255, 255, 255, 255]));
        // c0 <= c1 and index 3 -> transparent black.
        let block = [0, 0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];
        let out = decode(format::DXT1, 4, 4, &block).unwrap();
        assert!(out.chunks(4).all(|p| p == [0, 0, 0, 0]));
    }

    #[test]
    fn dxt5_alpha_interpolation() {
        // Alpha endpoints 255 / 0, all alpha indices 1 -> 0; colour red.
        let mut block = [0u8; 16];
        block[0] = 255;
        block[1] = 0;
        // index 1 for all 16 texels: bit pattern 001 repeated.
        let mut bits = 0u64;
        for t in 0..16 {
            bits |= 1 << (3 * t);
        }
        block[2..8].copy_from_slice(&bits.to_le_bytes()[..6]);
        block[8..10].copy_from_slice(&0xf800u16.to_le_bytes());
        let out = decode(format::DXT5, 4, 4, &block).unwrap();
        assert!(out.chunks(4).all(|p| p == [255, 0, 0, 0]));
        // Non-multiple-of-4 sizes use partial blocks.
        assert_eq!(
            decode(format::DXT5, 3, 5, &[0; 32]).unwrap().len(),
            3 * 5 * 4
        );
    }

    #[test]
    fn bc7_mode6_solid() {
        // Mode 6 (bit 6 set first), all endpoints and p-bits 1, indices 0:
        // every texel is the endpoint colour, 0xff in all channels.
        let mut block = [0u8; 16];
        // Bits: 6 mode bits (0b1000000), then 2 x 7 bits per channel R,G,B,A
        // and 2 p-bits, then indices (all 0).
        let mut bits: u128 = 1 << 6;
        let mut pos = 7;
        for _ in 0..8 {
            bits |= 0x7f << pos;
            pos += 7;
        }
        bits |= 0b11 << pos;
        block.copy_from_slice(&bits.to_le_bytes());
        let out = decode(format::BC7, 4, 4, &block).unwrap();
        assert!(out.chunks(4).all(|p| p == [255, 255, 255, 255]), "{out:?}");
        assert_eq!(
            decode(format::BC7, 5, 3, &[0; 32]).unwrap().len(),
            5 * 3 * 4
        );
    }

    #[test]
    fn flip_odd_rows() {
        let mut b = [1, 2, 3];
        flip_rows(&mut b, 1);
        assert_eq!(b, [3, 2, 1]);
    }
}
