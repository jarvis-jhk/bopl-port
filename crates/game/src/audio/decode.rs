//! Ogg Vorbis decoding (pure Rust, `lewton`).

use std::io::{Read, Seek};
use std::path::Path;

use anyhow::{Context, Result, bail};
use lewton::inside_ogg::OggStreamReader;

use super::mixer::SoundData;

/// Decodes a whole Ogg Vorbis stream to interleaved stereo `f32`
/// (mono is duplicated, channels beyond two are dropped).
pub fn decode_ogg<R: Read + Seek>(reader: R) -> Result<SoundData> {
    let mut ogg = OggStreamReader::new(reader).context("not an Ogg Vorbis stream")?;
    let channels = ogg.ident_hdr.audio_channels as usize;
    let sample_rate = ogg.ident_hdr.audio_sample_rate;
    if channels == 0 {
        bail!("Ogg stream without channels");
    }
    let mut samples = Vec::new();
    while let Some(packet) = ogg.read_dec_packet_itl().context("corrupt Vorbis packet")? {
        for frame in packet.chunks_exact(channels) {
            let l = frame[0] as f32 / 32768.0;
            let r = if channels > 1 {
                frame[1] as f32 / 32768.0
            } else {
                l
            };
            samples.extend_from_slice(&[l, r]);
        }
    }
    Ok(SoundData {
        sample_rate,
        samples,
    })
}

pub fn decode_ogg_file(path: impl AsRef<Path>) -> Result<SoundData> {
    let path = path.as_ref();
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    decode_ogg(std::io::BufReader::new(file))
        .with_context(|| format!("decoding {}", path.display()))
}
