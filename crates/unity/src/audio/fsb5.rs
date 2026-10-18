//! FSB5 sound bank container.
//!
//! Layout (little-endian): `"FSB5"`, `u32` version, `u32` sample count, `u32`
//! sample-header bytes, `u32` name-table bytes, `u32` data bytes, `u32` codec,
//! then reserved fields up to a header size of 60 (version 1) or 64 bytes
//! (version 0). Each sample header is a `u64` bit field — bit 0 "has chunks",
//! bits 1..5 frequency index, bits 5..7 channel code, bits 7..34 data offset
//! / 32, bits 34..64 sample count — followed by chunks, each a `u32` (bit 0
//! "more chunks", bits 1..25 size, bits 25..32 type) and its payload. Chunk 1
//! overrides the channel count, chunk 2 the frequency, chunk 11 holds the
//! Vorbis setup CRC32 (and a seek table).

use crate::bytes::Reader;
use crate::{Error, Result};

/// Codec ID of Vorbis.
pub const CODEC_VORBIS: u32 = 15;

/// One sample (sub-sound) of a bank.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sample {
    /// Sample rate in Hz.
    pub frequency: u32,
    /// Channel count.
    pub channels: u8,
    /// Offset of the sample's data inside the data section.
    pub data_offset: usize,
    /// Samples per channel.
    pub num_samples: u32,
    /// CRC32 of the stripped Vorbis setup header.
    pub vorbis_crc: Option<u32>,
}

/// A parsed bank.
#[derive(Clone, Debug)]
pub struct Bank<'a> {
    /// FSB5 version (0 or 1).
    pub version: u32,
    /// Codec of all samples.
    pub codec: u32,
    /// Sample headers.
    pub samples: Vec<Sample>,
    data: &'a [u8],
}

const FREQUENCIES: [u32; 11] = [
    4000, 8000, 11000, 11025, 16000, 22050, 24000, 32000, 44100, 48000, 96000,
];

impl<'a> Bank<'a> {
    /// Parses a bank.
    pub fn parse(buf: &'a [u8]) -> Result<Self> {
        let mut r = Reader::new(buf);
        if r.take(4)? != b"FSB5" {
            return Err(Error::Format("not an FSB5 bank".into()));
        }
        let version = r.u32()?;
        let count = r.u32()? as usize;
        let headers_size = r.u32()? as usize;
        let names_size = r.u32()? as usize;
        let data_size = r.u32()? as usize;
        let codec = r.u32()?;
        let header_size = if version == 0 { 64 } else { 60 };
        let data_start = header_size + headers_size + names_size;
        if data_start + data_size > buf.len() {
            return Err(Error::Format("FSB5 sections exceed the bank".into()));
        }
        r.seek(header_size)?;
        let mut samples = Vec::with_capacity(count.min(1024));
        for _ in 0..count {
            let bits = r.u64()?;
            let mut more = bits & 1 != 0;
            let freq_index = ((bits >> 1) & 0xf) as usize;
            let mut s = Sample {
                frequency: *FREQUENCIES.get(freq_index).unwrap_or(&44100),
                channels: match (bits >> 5) & 3 {
                    0 => 1,
                    1 => 2,
                    2 => 6,
                    _ => 8,
                },
                data_offset: ((bits >> 7) & 0x7ff_ffff) as usize * 32,
                num_samples: (bits >> 34) as u32,
                vorbis_crc: None,
            };
            while more {
                let c = r.u32()?;
                more = c & 1 != 0;
                let size = ((c >> 1) & 0xff_ffff) as usize;
                let kind = c >> 25;
                let payload = r.take(size)?;
                let mut p = Reader::new(payload);
                match kind {
                    1 => s.channels = p.u8()?,
                    2 => s.frequency = p.u32()?,
                    11 => s.vorbis_crc = Some(p.u32()?),
                    _ => {}
                }
            }
            samples.push(s);
        }
        Ok(Bank {
            version,
            codec,
            samples,
            data: &buf[data_start..data_start + data_size],
        })
    }

    /// The encoded bytes of sample `i`.
    pub fn sample_data(&self, i: usize) -> Result<&'a [u8]> {
        let s = self
            .samples
            .get(i)
            .ok_or_else(|| Error::NotFound(format!("FSB5 sample {i}")))?;
        let end = self
            .samples
            .get(i + 1)
            .map_or(self.data.len(), |n| n.data_offset);
        self.data
            .get(s.data_offset..end)
            .ok_or_else(|| Error::Format("FSB5 sample data out of range".into()))
    }
}

/// Splits FMOD Vorbis sample data (`u16` length + packet, repeated; a length
/// of 0 or 0xffff ends the stream) into packets.
pub fn vorbis_packets(mut data: &[u8]) -> Result<Vec<&[u8]>> {
    let mut out = Vec::new();
    while data.len() >= 2 {
        let n = u16::from_le_bytes([data[0], data[1]]) as usize;
        if n == 0 || n == 0xffff {
            break;
        }
        let p = data
            .get(2..2 + n)
            .ok_or_else(|| Error::Format("truncated FSB5 Vorbis packet".into()))?;
        out.push(p);
        data = &data[2 + n..];
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a one-sample Vorbis bank with the given packet payloads.
    pub(crate) fn bank(crc: u32, packets: &[&[u8]], num_samples: u32) -> Vec<u8> {
        let mut data = Vec::new();
        for p in packets {
            data.extend_from_slice(&(p.len() as u16).to_le_bytes());
            data.extend_from_slice(p);
        }
        // freq index 9 (48000), mono, offset 0, chunks present.
        let bits: u64 = 1 | (9 << 1) | ((num_samples as u64) << 34);
        let mut sh = bits.to_le_bytes().to_vec();
        let chunk: u32 = (4 << 1) | (11 << 25);
        sh.extend_from_slice(&chunk.to_le_bytes());
        sh.extend_from_slice(&crc.to_le_bytes());
        let mut b = b"FSB5".to_vec();
        for v in [1u32, 1, sh.len() as u32, 0, data.len() as u32, CODEC_VORBIS] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.resize(60, 0);
        b.extend_from_slice(&sh);
        b.extend_from_slice(&data);
        b
    }

    #[test]
    fn parses_header_and_packets() {
        let b = bank(0xdead_beef, &[&[1, 2, 3], &[4]], 1000);
        let bank = Bank::parse(&b).unwrap();
        assert_eq!(bank.codec, CODEC_VORBIS);
        let s = &bank.samples[0];
        assert_eq!((s.frequency, s.channels, s.num_samples), (48000, 1, 1000));
        assert_eq!(s.vorbis_crc, Some(0xdead_beef));
        let p = vorbis_packets(bank.sample_data(0).unwrap()).unwrap();
        assert_eq!(p, [&[1u8, 2, 3][..], &[4][..]]);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(Bank::parse(b"RIFF....").is_err());
        let mut b = bank(1, &[&[1]], 1);
        b.truncate(70);
        assert!(Bank::parse(&b).is_err());
        assert!(vorbis_packets(&[5, 0, 1]).is_err());
    }
}
