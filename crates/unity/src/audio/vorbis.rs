//! Vorbis header reconstruction for FMOD streams.
//!
//! The setup headers FMOD strips are identified by CRC32. The table of known
//! headers (`data/fsb5_vorbis_setup.bin`, see `scripts/dev/gen_fsb_vorbis_setup.py`)
//! comes from the `fsbex` crate (MIT OR Apache-2.0); it is FMOD codec data
//! (codebooks), not game content.
//!
//! To compute Ogg granule positions we need each audio packet's block size,
//! which depends on the setup header's mode table, so this module contains a
//! small forward parser of the setup header (Vorbis I spec, section 4.2.4)
//! that skips codebooks, floors, residues and mappings to reach the modes.

use std::collections::HashMap;
use std::sync::OnceLock;

use super::ogg;
use crate::{Error, Result};

const SETUP_TABLE: &[u8] = include_bytes!("../../data/fsb5_vorbis_setup.bin");

/// FMOD's Vorbis block sizes (log2): 256 and 2048 samples.
pub const BLOCKSIZE_EXP: (u8, u8) = (8, 11);

fn table() -> &'static HashMap<u32, &'static [u8]> {
    static T: OnceLock<HashMap<u32, &'static [u8]>> = OnceLock::new();
    T.get_or_init(|| {
        let mut m = HashMap::new();
        let rd = |p: usize| u32::from_le_bytes(SETUP_TABLE[p..p + 4].try_into().unwrap());
        let n = rd(0) as usize;
        let mut p = 4;
        for _ in 0..n {
            let (crc, len) = (rd(p), rd(p + 4) as usize);
            m.insert(crc, &SETUP_TABLE[p + 8..p + 8 + len]);
            p += 8 + len;
        }
        m
    })
}

/// The full setup header packet for an FMOD setup CRC32.
pub fn setup_header(crc: u32) -> Option<&'static [u8]> {
    table().get(&crc).copied()
}

/// Identification header packet.
pub fn ident_header(channels: u8, rate: u32) -> Vec<u8> {
    let mut d = vec![1];
    d.extend_from_slice(b"vorbis");
    d.extend_from_slice(&0u32.to_le_bytes());
    d.push(channels);
    d.extend_from_slice(&rate.to_le_bytes());
    d.extend_from_slice(&[0; 12]); // bitrates: unset
    d.push(BLOCKSIZE_EXP.0 | (BLOCKSIZE_EXP.1 << 4));
    d.push(1);
    d
}

/// Comment header packet with our vendor string and no comments.
pub fn comment_header() -> Vec<u8> {
    let vendor = b"bopl-port fsb5 remux";
    let mut d = vec![3];
    d.extend_from_slice(b"vorbis");
    d.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
    d.extend_from_slice(vendor);
    d.extend_from_slice(&0u32.to_le_bytes());
    d.push(1);
    d
}

/// LSB-first bit reader (Vorbis bit packing).
struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn read(&mut self, n: u32) -> Result<u32> {
        let mut v = 0u32;
        for i in 0..n {
            let byte = *self
                .d
                .get(self.pos >> 3)
                .ok_or_else(|| Error::Format("Vorbis setup header truncated".into()))?;
            v |= (((byte >> (self.pos & 7)) & 1) as u32) << i;
            self.pos += 1;
        }
        Ok(v)
    }

    fn skip(&mut self, n: u64) -> Result<()> {
        self.pos += n as usize;
        if self.pos > self.d.len() * 8 {
            return Err(Error::Format("Vorbis setup header truncated".into()));
        }
        Ok(())
    }
}

fn ilog(v: u32) -> u32 {
    32 - v.leading_zeros()
}

fn lookup1_values(entries: u32, dims: u32) -> u64 {
    let mut r = (entries as f64).powf(1.0 / dims as f64).floor() as u64;
    while (r + 1)
        .checked_pow(dims)
        .is_some_and(|x| x <= entries as u64)
    {
        r += 1;
    }
    while r > 0 && r.checked_pow(dims).is_none_or(|x| x > entries as u64) {
        r -= 1;
    }
    r
}

/// Parses a setup header and returns each mode's block flag.
pub fn mode_blockflags(setup: &[u8], channels: u8) -> Result<Vec<bool>> {
    let bad = |m: &str| Error::Format(format!("Vorbis setup header: {m}"));
    if setup.len() < 7 || setup[0] != 5 || &setup[1..7] != b"vorbis" {
        return Err(bad("bad packet header"));
    }
    let mut b = Bits {
        d: &setup[7..],
        pos: 0,
    };
    // Codebooks.
    for _ in 0..b.read(8)? + 1 {
        if b.read(24)? != 0x56_4342 {
            return Err(bad("codebook sync"));
        }
        let dims = b.read(16)?;
        let entries = b.read(24)?;
        if b.read(1)? == 1 {
            // Ordered lengths.
            let mut cur = 0;
            b.read(5)?;
            while cur < entries {
                cur += b.read(ilog(entries - cur))?;
            }
            if cur > entries {
                return Err(bad("codebook lengths overrun"));
            }
        } else {
            let sparse = b.read(1)? == 1;
            for _ in 0..entries {
                if !sparse || b.read(1)? == 1 {
                    b.read(5)?;
                }
            }
        }
        match b.read(4)? {
            0 => {}
            t @ (1 | 2) => {
                b.skip(64)?;
                let value_bits = b.read(4)? + 1;
                b.read(1)?;
                let n = if t == 1 {
                    lookup1_values(entries, dims)
                } else {
                    entries as u64 * dims as u64
                };
                b.skip(n * value_bits as u64)?;
            }
            _ => return Err(bad("codebook lookup type")),
        }
    }
    // Time domain transforms (placeholders).
    for _ in 0..b.read(6)? + 1 {
        b.read(16)?;
    }
    // Floors.
    for _ in 0..b.read(6)? + 1 {
        match b.read(16)? {
            0 => {
                b.skip(8 + 16 + 16 + 6 + 8)?;
                let books = b.read(4)? + 1;
                b.skip(books as u64 * 8)?;
            }
            1 => {
                let parts = b.read(5)?;
                let mut classes = Vec::with_capacity(parts as usize);
                for _ in 0..parts {
                    classes.push(b.read(4)?);
                }
                let max = classes.iter().copied().max().map_or(0, |m| m + 1);
                let mut dims = Vec::with_capacity(max as usize);
                for _ in 0..max {
                    dims.push(b.read(3)? + 1);
                    let sub = b.read(2)?;
                    if sub > 0 {
                        b.read(8)?;
                    }
                    b.skip((1u64 << sub) * 8)?;
                }
                b.read(2)?;
                let range_bits = b.read(4)?;
                for c in classes {
                    b.skip(dims[c as usize] as u64 * range_bits as u64)?;
                }
            }
            _ => return Err(bad("floor type")),
        }
    }
    // Residues.
    for _ in 0..b.read(6)? + 1 {
        if b.read(16)? > 2 {
            return Err(bad("residue type"));
        }
        b.skip(24 * 3)?;
        let classifications = b.read(6)? + 1;
        b.read(8)?;
        let mut books = 0u64;
        for _ in 0..classifications {
            let low = b.read(3)?;
            let high = if b.read(1)? == 1 { b.read(5)? } else { 0 };
            books += ((high << 3) | low).count_ones() as u64;
        }
        b.skip(books * 8)?;
    }
    // Mappings.
    let ch_bits = ilog(channels.saturating_sub(1) as u32);
    for _ in 0..b.read(6)? + 1 {
        if b.read(16)? != 0 {
            return Err(bad("mapping type"));
        }
        let submaps = if b.read(1)? == 1 { b.read(4)? + 1 } else { 1 };
        if b.read(1)? == 1 {
            let steps = b.read(8)? + 1;
            b.skip(steps as u64 * 2 * ch_bits as u64)?;
        }
        if b.read(2)? != 0 {
            return Err(bad("mapping reserved bits"));
        }
        if submaps > 1 {
            b.skip(channels as u64 * 4)?;
        }
        b.skip(submaps as u64 * 24)?;
    }
    // Modes.
    let n = b.read(6)? + 1;
    let mut flags = Vec::with_capacity(n as usize);
    for _ in 0..n {
        flags.push(b.read(1)? == 1);
        b.skip(16 + 16 + 8)?;
    }
    if b.read(1)? != 1 {
        return Err(bad("framing bit"));
    }
    Ok(flags)
}

/// Builds an Ogg Vorbis file from FMOD audio packets.
pub fn remux(
    channels: u8,
    rate: u32,
    crc: u32,
    num_samples: u64,
    packets: &[&[u8]],
) -> Result<Vec<u8>> {
    let setup = setup_header(crc)
        .ok_or_else(|| Error::Unsupported(format!("unknown FMOD Vorbis setup CRC {crc:08x}")))?;
    let modes = mode_blockflags(setup, channels)?;
    let mode_bits = ilog(modes.len() as u32 - 1);
    let sizes = (1u64 << BLOCKSIZE_EXP.0, 1u64 << BLOCKSIZE_EXP.1);
    let mut w = ogg::Writer::new(crc ^ rate);
    w.packet(&ident_header(channels, rate), 0, true);
    w.packet(&comment_header(), 0, false);
    w.packet(setup, 0, true);
    let mut granule = 0u64;
    let mut prev: Option<u64> = None;
    for p in packets {
        let Some(&first) = p.first() else { continue };
        if first & 1 != 0 {
            return Err(Error::Format("FSB5 Vorbis: non-audio packet".into()));
        }
        let mode = ((first >> 1) as u32) & ((1 << mode_bits) - 1);
        let long = *modes
            .get(mode as usize)
            .ok_or_else(|| Error::Format("FSB5 Vorbis: bad mode".into()))?;
        let bs = if long { sizes.1 } else { sizes.0 };
        if let Some(pb) = prev {
            granule += pb / 4 + bs / 4;
        }
        prev = Some(bs);
        w.packet(p, granule.min(num_samples), false);
    }
    Ok(w.finish(num_samples.min(granule)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_and_modes() {
        assert!(table().len() > 100);
        // Every known setup header parses (as mono or stereo: coupling
        // steps only exist in multi-channel headers) and agrees with lewton.
        for &s in table().values() {
            let ok: Vec<u8> = [1u8, 2]
                .into_iter()
                .filter(|&ch| mode_blockflags(s, ch).is_ok_and(|m| !m.is_empty()))
                .collect();
            assert!(!ok.is_empty());
            for ch in ok {
                assert!(lewton::header::read_header_setup(s, ch, BLOCKSIZE_EXP).is_ok());
            }
        }
    }

    #[test]
    fn lookup1() {
        assert_eq!(lookup1_values(81, 4), 3);
        assert_eq!(lookup1_values(80, 4), 2);
        assert_eq!(lookup1_values(1, 8), 1);
        assert_eq!(lookup1_values(1000, 1), 1000);
    }

    #[test]
    fn headers_parse_with_lewton() {
        let id = ident_header(2, 44100);
        let h = lewton::header::read_header_ident(&id).unwrap();
        assert_eq!((h.audio_channels, h.audio_sample_rate), (2, 44100));
        lewton::header::read_header_comment(&comment_header()).unwrap();
        let (_, &setup) = table().iter().next().unwrap();
        lewton::header::read_header_setup(setup, 2, BLOCKSIZE_EXP).unwrap();
    }
}
