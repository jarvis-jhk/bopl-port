//! Minimal Ogg page writer (RFC 3533) for a single logical stream.

/// Header type flag: packet continued from the previous page.
const CONTINUED: u8 = 1;
/// Header type flag: first page of the stream.
const BOS: u8 = 2;
/// Header type flag: last page of the stream.
const EOS: u8 = 4;

/// Target payload size of a page before it is flushed.
const PAGE_TARGET: usize = 4096;

fn crc_table() -> &'static [u32; 256] {
    static T: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let mut r = (i as u32) << 24;
            for _ in 0..8 {
                r = if r & 0x8000_0000 != 0 {
                    (r << 1) ^ 0x04c1_1db7
                } else {
                    r << 1
                };
            }
            *e = r;
        }
        t
    })
}

/// Ogg's CRC32 (polynomial 0x04c11db7, no reflection, zero init).
pub fn crc32(data: &[u8]) -> u32 {
    let t = crc_table();
    data.iter()
        .fold(0u32, |c, &b| (c << 8) ^ t[((c >> 24) as u8 ^ b) as usize])
}

/// Writes packets into Ogg pages.
pub struct Writer {
    serial: u32,
    seq: u32,
    out: Vec<u8>,
    segs: Vec<u8>,
    body: Vec<u8>,
    /// Whether the current page starts with a continued packet.
    continued: bool,
    /// Granule of the last packet completed on the current page.
    granule: Option<u64>,
    first: bool,
}

impl Writer {
    /// A writer for stream `serial`.
    pub fn new(serial: u32) -> Self {
        Writer {
            serial,
            seq: 0,
            out: Vec::new(),
            segs: Vec::new(),
            body: Vec::new(),
            continued: false,
            granule: None,
            first: true,
        }
    }

    /// Appends a packet ending at `granule`; `flush` forces a page break after it.
    pub fn packet(&mut self, data: &[u8], granule: u64, flush: bool) {
        let mut rest = data;
        loop {
            let take = rest.len().min(255);
            if self.segs.len() == 255 {
                self.flush_page(false);
                self.continued = true;
            }
            self.segs.push(take as u8);
            self.body.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            if take < 255 {
                break;
            }
            if rest.is_empty() {
                // A packet that is a multiple of 255 bytes ends with a 0 lacing value.
                if self.segs.len() == 255 {
                    self.flush_page(false);
                    self.continued = true;
                }
                self.segs.push(0);
                break;
            }
        }
        self.granule = Some(granule);
        if flush || self.body.len() >= PAGE_TARGET {
            self.flush_page(false);
        }
    }

    fn flush_page(&mut self, eos: bool) {
        if self.segs.is_empty() && !eos {
            return;
        }
        let mut flags = 0;
        if self.continued {
            flags |= CONTINUED;
        }
        if self.first {
            flags |= BOS;
        }
        if eos {
            flags |= EOS;
        }
        let granule = self.granule.unwrap_or(u64::MAX);
        let start = self.out.len();
        self.out.extend_from_slice(b"OggS");
        self.out.push(0);
        self.out.push(flags);
        self.out.extend_from_slice(&granule.to_le_bytes());
        self.out.extend_from_slice(&self.serial.to_le_bytes());
        self.out.extend_from_slice(&self.seq.to_le_bytes());
        self.out.extend_from_slice(&[0; 4]);
        self.out.push(self.segs.len() as u8);
        self.out.extend_from_slice(&self.segs);
        self.out.extend_from_slice(&self.body);
        let crc = crc32(&self.out[start..]);
        self.out[start + 22..start + 26].copy_from_slice(&crc.to_le_bytes());
        self.seq += 1;
        self.first = false;
        self.continued = false;
        self.granule = None;
        self.segs.clear();
        self.body.clear();
    }

    /// Writes the pending page with the end-of-stream flag and returns the file.
    pub fn finish(mut self, last_granule: u64) -> Vec<u8> {
        if self.granule.is_some() {
            self.granule = Some(last_granule);
        }
        self.flush_page(true);
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_reference() {
        // CRC of "OggS" with the Ogg polynomial.
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0x89a1_897f);
    }

    #[test]
    fn pages_and_lacing() {
        let mut w = Writer::new(7);
        w.packet(&[1; 30], 0, true);
        w.packet(&[2; 510], 10, false);
        w.packet(&[3; 70_000], 20, false);
        let out = w.finish(25);
        // Walk the pages and reassemble.
        let (mut pos, mut pages, mut payload) = (0, 0, 0usize);
        let mut last_flags = 0;
        while pos < out.len() {
            assert_eq!(&out[pos..pos + 4], b"OggS");
            let n = out[pos + 26] as usize;
            let body: usize = out[pos + 27..pos + 27 + n]
                .iter()
                .map(|&s| s as usize)
                .sum();
            let mut page = out[pos..pos + 27 + n + body].to_vec();
            let crc = u32::from_le_bytes(page[22..26].try_into().unwrap());
            page[22..26].fill(0);
            assert_eq!(crc32(&page), crc);
            last_flags = out[pos + 5];
            if pages == 0 {
                assert_eq!(last_flags & BOS, BOS);
            }
            payload += body;
            pos += 27 + n + body;
            pages += 1;
        }
        assert_eq!(payload, 30 + 510 + 70_000);
        assert!(pages >= 3);
        assert_eq!(last_flags & EOS, EOS);
    }
}
