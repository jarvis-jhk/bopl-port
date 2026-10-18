"""Dev-only: build crates/unity/data/fsb5_vorbis_setup.bin.

FMOD strips the Vorbis setup header from FSB5 streams and stores only its
CRC32; decoders need the original header. The `fsbex` crate (MIT OR
Apache-2.0, https://github.com/astral4/fsbex) ships the known FMOD setup
headers as a Rust table; this script converts that table into a compact
binary blob that `unity-assets` embeds:

  u32 count, then per entry: u32 crc32, u32 length, `length` bytes (LE)

The headers are FMOD codec data (codebooks), not game content.
Usage: python -I scripts/dev/gen_fsb_vorbis_setup.py  (needs fsbex 0.3.0 in
tools/cargo/registry, e.g. after `cargo info fsbex`)
"""
import glob
import os
import re
import struct

ROOT = os.environ["BOPL_ROOT"]
src = glob.glob(os.path.join(ROOT, "tools/cargo/registry/src/*/fsbex-0.3.0/src/encode/vorbis_lookup.rs"))[0]
text = open(src).read()
arrays = {m.group(1): bytes(int(x, 16) for x in re.findall(r"0x([0-9A-Fa-f]{2})", m.group(2)))
          for m in re.finditer(r"const (FVS_[0-9A-F]+): \[u8; \d+\] = \[(.*?)\];", text, re.S)}
entries = [(int(c.replace("_", ""), 16), arrays[n])
           for c, n in re.findall(r"0x([0-9A-F_]+)u32 => &(FVS_[0-9A-F]+)", text)]
assert entries and all(e[1][:7] == b"\x05vorbis" for e in entries)
out = os.path.join(ROOT, "crates/unity/data/fsb5_vorbis_setup.bin")
with open(out, "wb") as f:
    f.write(struct.pack("<I", len(entries)))
    for crc, data in sorted(entries):
        f.write(struct.pack("<II", crc, len(data)))
        f.write(data)
print(len(entries), "setup headers,", os.path.getsize(out), "bytes")
