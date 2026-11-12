//! A small PNG reader for the reference pictures (non-interlaced, types 0/2/3/6, depth 1-8):
//! gameboy-python `tests/pngread.py`, for the suite runners.

use std::io::Read;
use std::path::Path;

/// Decoded pixels (each 1 or 3 channels, as the reference's reader returns them).
struct Decoded {
    pixels: Vec<Vec<u8>>,
    width: usize,
    height: usize,
    depth: u32,
    ctype: u8,
}

fn decode(path: &Path) -> Result<Decoded, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let mut pos = 8;
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let mut idat = Vec::new();
    let (mut width, mut height, mut depth, mut ctype) = (0usize, 0usize, 0u32, 0u8);
    while pos + 8 <= data.len() {
        let length = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        let kind = &data[pos + 4..pos + 8];
        let body = &data[pos + 8..pos + 8 + length];
        pos += 12 + length;
        match kind {
            b"IHDR" => {
                width = u32::from_be_bytes(body[0..4].try_into().unwrap()) as usize;
                height = u32::from_be_bytes(body[4..8].try_into().unwrap()) as usize;
                depth = body[8] as u32;
                ctype = body[9];
                if depth > 8 || body[12] != 0 {
                    return Err("unsupported PNG".into());
                }
            }
            b"PLTE" => {
                palette = body.chunks(3).map(|c| [c[0], c[1], c[2]]).collect();
            }
            b"IDAT" => idat.extend_from_slice(body),
            _ => {}
        }
    }
    let channels = match ctype {
        0 => 1,
        2 => 3,
        3 => 1,
        6 => 4,
        _ => return Err("unsupported colour type".into()),
    };
    let mut raw = Vec::new();
    flate2::read::ZlibDecoder::new(&idat[..])
        .read_to_end(&mut raw)
        .map_err(|e| e.to_string())?;
    let stride = (width * channels * depth as usize).div_ceil(8);
    let step = (channels * depth as usize / 8).max(1);
    let mut prev = vec![0u8; stride];
    let mut p = 0;
    let mut pixels = Vec::with_capacity(width * height);
    for _ in 0..height {
        let ft = raw[p];
        let mut line = raw[p + 1..p + 1 + stride].to_vec();
        p += 1 + stride;
        for i in 0..stride {
            let a = if i >= step { line[i - step] as i32 } else { 0 };
            let b = prev[i] as i32;
            let c = if i >= step { prev[i - step] as i32 } else { 0 };
            let add = match ft {
                1 => a,
                2 => b,
                3 => (a + b) >> 1,
                4 => {
                    let (pa, pb, pc) = ((b - c).abs(), (a - c).abs(), (a + b - 2 * c).abs());
                    if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    }
                }
                _ => 0,
            };
            line[i] = (line[i] as i32 + add) as u8;
        }
        for x in 0..width {
            if depth < 8 {
                let per = 8 / depth as usize;
                let value = (line[x / per] >> (8 - depth as usize * (x % per + 1)))
                    & ((1u16 << depth) - 1) as u8;
                pixels.push(if ctype == 3 {
                    palette[value as usize].to_vec()
                } else {
                    vec![value]
                });
            } else if ctype == 3 {
                pixels.push(palette[line[x] as usize].to_vec());
            } else {
                pixels.push(line[x * channels..x * channels + channels.min(3)].to_vec());
            }
        }
        prev = line;
    }
    Ok(Decoded {
        pixels,
        width,
        height,
        depth,
        ctype,
    })
}

fn top(d: &Decoded) -> u32 {
    if d.ctype == 0 {
        (1 << d.depth) - 1
    } else {
        255
    }
}

/// (shades 0-3, 0 the lightest, width, height) of a DMG-grey picture.
pub fn read_shades(path: &Path) -> Result<(Vec<u8>, usize, usize), String> {
    let d = decode(path)?;
    let top = top(&d);
    let mut shades = Vec::with_capacity(d.pixels.len());
    for pixel in &d.pixels {
        let grey = pixel[0] as u32 * 255 / top;
        let same = pixel.iter().take(3).all(|&c| c == pixel[0]);
        let rounded = ((grey as f64 / 85.0).round() as u32) * 85;
        if !same || rounded != grey {
            return Err(format!("not a DMG grey: {pixel:?}"));
        }
        shades.push((3 - grey / 85) as u8);
    }
    Ok((shades, d.width, d.height))
}

/// (RGB555 words, width, height): each 8-bit channel scaled back to 5 bits the way the
/// reference pictures were made (v * 255 / 31).
pub fn read_rgb555(path: &Path) -> Result<(Vec<u16>, usize, usize), String> {
    let d = decode(path)?;
    let top = top(&d);
    let mut words = Vec::with_capacity(d.pixels.len());
    for pixel in &d.pixels {
        let p: Vec<u32> = if pixel.len() == 1 {
            vec![pixel[0] as u32 * 255 / top; 3]
        } else {
            pixel.iter().map(|&c| c as u32).collect()
        };
        // Python's round(): half to even
        let five = |c: u32| -> u16 { round_half_even(c as f64 * 31.0 / 255.0) as u16 };
        words.push(five(p[0]) | (five(p[1]) << 5) | (five(p[2]) << 10));
    }
    Ok((words, d.width, d.height))
}

fn round_half_even(x: f64) -> f64 {
    let r = x.round();
    if (x - x.trunc()).abs() == 0.5 && r % 2.0 != 0.0 {
        r - x.signum()
    } else {
        r
    }
}

/// (0xRRGGBB pixels, width, height).
pub fn read_rgb888(path: &Path) -> Result<(Vec<u32>, usize, usize), String> {
    let d = decode(path)?;
    let top = top(&d);
    let mut out = Vec::with_capacity(d.pixels.len());
    for pixel in &d.pixels {
        let p: Vec<u32> = if pixel.len() == 1 {
            vec![pixel[0] as u32 * 255 / top; 3]
        } else {
            pixel.iter().map(|&c| c as u32).collect()
        };
        out.push(p[0] << 16 | p[1] << 8 | p[2]);
    }
    Ok((out, d.width, d.height))
}
