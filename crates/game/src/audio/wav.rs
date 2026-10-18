//! Minimal 16-bit PCM stereo WAV writer (headless audio output).

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

use anyhow::Result;

pub struct WavWriter {
    out: BufWriter<File>,
    frames: u32,
    sample_rate: u32,
}

impl WavWriter {
    pub fn create(path: impl AsRef<Path>, sample_rate: u32) -> Result<Self> {
        let path = path.as_ref();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut w = Self {
            out: BufWriter::new(File::create(path)?),
            frames: 0,
            sample_rate,
        };
        w.write_header()?;
        Ok(w)
    }

    fn write_header(&mut self) -> Result<()> {
        let data_len = self.frames * 4;
        let rate = self.sample_rate;
        let o = &mut self.out;
        o.write_all(b"RIFF")?;
        o.write_all(&(36 + data_len).to_le_bytes())?;
        o.write_all(b"WAVEfmt ")?;
        o.write_all(&16u32.to_le_bytes())?;
        o.write_all(&1u16.to_le_bytes())?; // PCM
        o.write_all(&2u16.to_le_bytes())?; // channels
        o.write_all(&rate.to_le_bytes())?;
        o.write_all(&(rate * 4).to_le_bytes())?; // byte rate
        o.write_all(&4u16.to_le_bytes())?; // block align
        o.write_all(&16u16.to_le_bytes())?; // bits
        o.write_all(b"data")?;
        o.write_all(&data_len.to_le_bytes())?;
        Ok(())
    }

    /// Appends interleaved stereo samples in -1..1.
    pub fn write(&mut self, samples: &[f32]) -> Result<()> {
        for s in samples {
            let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
            self.out.write_all(&v.to_le_bytes())?;
        }
        self.frames += (samples.len() / 2) as u32;
        Ok(())
    }

    /// Patches the header sizes and flushes.
    pub fn finish(mut self) -> Result<()> {
        self.out.seek(SeekFrom::Start(0))?;
        self.write_header()?;
        self.out.flush()?;
        Ok(())
    }
}
