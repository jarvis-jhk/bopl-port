//! Dev tool: converts every AudioClip of the game to Ogg Vorbis in memory,
//! decodes the result with lewton and compares sample counts against the
//! FSB5 header and `m_Length`. lewton does not trim the final packet to the
//! last granule position, so it may return up to one block more than the
//! header's count; the last granule must equal the header's count.
//!
//! Usage: `cargo run --release -p unity-assets --example audio_check -- [--game DIR] [--write DIR N]`
//! (`--write` also stores the first N files for inspection with ffprobe).

use std::io::Cursor;

use rayon::prelude::*;
use unity_assets::AssetDir;
use unity_assets::audio::fsb5_to_ogg;
use unity_assets::classes::AudioClip;

fn decode_samples(ogg: &[u8]) -> Result<(u64, u8, u32, u64), String> {
    let mut r =
        lewton::inside_ogg::OggStreamReader::new(Cursor::new(ogg)).map_err(|e| e.to_string())?;
    let (ch, rate) = (r.ident_hdr.audio_channels, r.ident_hdr.audio_sample_rate);
    let mut n = 0u64;
    while let Some(p) = r.read_dec_packet_itl().map_err(|e| e.to_string())? {
        n += (p.len() / ch as usize) as u64;
    }
    Ok((n, ch, rate, r.get_last_absgp().unwrap_or(0)))
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut game = std::env::var("BOPL_GAME_DIR").unwrap_or_default();
    let mut write: Option<(String, usize)> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--game" => game = args.next().unwrap(),
            "--write" => {
                write = Some((args.next().unwrap(), args.next().unwrap().parse().unwrap()))
            }
            _ => panic!("unknown argument {a}"),
        }
    }
    let dir = AssetDir::open(&game).unwrap();
    let mut clips = Vec::new();
    for name in dir.serialized_file_names().unwrap() {
        let f = dir.file(&name).unwrap();
        for o in f.objects_of(83) {
            clips.push((f.clone(), o.path_id));
        }
    }
    let t = std::time::Instant::now();
    let results: Vec<_> = clips
        .par_iter()
        .map(|(f, id)| {
            let clip = AudioClip::from_value(&dir.read(f, *id).unwrap());
            let r = clip.resource.clone();
            let res = dir
                .stream(&r.path, r.offset, r.size)
                .map_err(|e| e.to_string())
                .and_then(|bank| fsb5_to_ogg(&bank).map_err(|e| e.to_string()))
                .and_then(|(ogg, n)| decode_samples(&ogg).map(|d| (ogg, n, d)));
            (f.name.clone(), *id, clip, res)
        })
        .collect();
    let (mut ok, mut bad, mut bytes) = (0, 0, 0usize);
    let mut worst = 0f64;
    for (i, (file, id, clip, res)) in results.iter().enumerate() {
        match res {
            Ok((ogg, n, (decoded, ch, rate, last_gp))) => {
                bytes += ogg.len();
                let expect_len = (clip.length as f64 * clip.frequency as f64).round() as i64;
                let d_len = (*last_gp as i64 - expect_len).abs();
                let consistent = *decoded >= *n
                    && *decoded - *n < 2048
                    && *last_gp == *n
                    && (*n as i64 - expect_len).abs() <= 1
                    && *ch as u32 == clip.channels
                    && *rate == clip.frequency
                    && d_len <= 1;
                worst = worst.max(d_len as f64 / clip.frequency.max(1) as f64);
                if consistent {
                    ok += 1;
                } else {
                    bad += 1;
                    println!(
                        "{file}:{id} {}: decoded {decoded} granule {last_gp} header {n} m_Length*rate {expect_len} ch {ch}/{} rate {rate}/{}",
                        clip.name, clip.channels, clip.frequency
                    );
                }
                if let Some((dir, max)) = &write
                    && i < *max
                {
                    std::fs::create_dir_all(dir).unwrap();
                    std::fs::write(format!("{dir}/{}_{}.ogg", clip.name, id), ogg).unwrap();
                }
            }
            Err(e) => {
                bad += 1;
                println!("{file}:{id} {}: {e}", clip.name);
            }
        }
    }
    println!(
        "{} clips: {ok} consistent, {bad} not; {:.1} MB of Ogg; worst |length - m_Length| {:.4}s; {:.1}s",
        results.len(),
        bytes as f64 / 1e6,
        worst,
        t.elapsed().as_secs_f64()
    );
}
