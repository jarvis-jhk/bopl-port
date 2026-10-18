//! Audio: FSB5 sound banks (as embedded by Unity's `AudioClip`) and lossless
//! remuxing of their Vorbis streams into standalone Ogg Vorbis files.
//!
//! FMOD stores Vorbis audio packets without Ogg framing and replaces the three
//! Vorbis header packets by the sample rate, channel count and the CRC32 of
//! the setup header. [`fsb5_to_ogg`] rebuilds the identification and comment
//! headers, looks the setup header up by CRC32 in a table of FMOD's known
//! setup headers ([`vorbis::setup_header`]), computes granule positions from
//! the packets' block sizes and writes Ogg pages. No re-encoding happens.

pub mod fsb5;
pub mod ogg;
pub mod vorbis;

use crate::{Error, Result};

/// Converts the first stream of an FSB5 bank (Vorbis codec) to an Ogg file.
/// Returns the Ogg bytes and the number of samples per channel.
pub fn fsb5_to_ogg(bank: &[u8]) -> Result<(Vec<u8>, u64)> {
    let b = fsb5::Bank::parse(bank)?;
    if b.codec != fsb5::CODEC_VORBIS {
        return Err(Error::Unsupported(format!("FSB5 codec {}", b.codec)));
    }
    let s = b
        .samples
        .first()
        .ok_or_else(|| Error::Format("FSB5 without samples".into()))?;
    let crc = s
        .vorbis_crc
        .ok_or_else(|| Error::Format("FSB5 Vorbis stream without CRC".into()))?;
    let packets = fsb5::vorbis_packets(b.sample_data(0)?)?;
    let ogg = vorbis::remux(s.channels, s.frequency, crc, s.num_samples as u64, &packets)?;
    Ok((ogg, s.num_samples as u64))
}
