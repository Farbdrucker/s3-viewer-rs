//! Decompresses one tile or strip and turns its samples into `f32`.

use std::io::Read;

use super::ifd::ByteOrder;

pub struct ChunkLayout<'a> {
    /// Pixels per row in the chunk (tile width, or image width for strips).
    pub width: usize,
    pub rows: usize,
    /// Samples per pixel stored in this chunk (1 for planar images).
    pub samples: usize,
    pub bits: u16,
    /// 1 unsigned, 2 signed, 3 floating point.
    pub format: u16,
    pub order: ByteOrder,
    pub compression: u16,
    pub predictor: u16,
    pub photometric: u16,
    pub jpeg_tables: Option<&'a [u8]>,
}

pub fn compression_name(code: u16) -> String {
    match code {
        1 => "None".into(),
        2 => "CCITT RLE".into(),
        3 => "CCITT G3".into(),
        4 => "CCITT G4".into(),
        5 => "LZW".into(),
        6 => "Old JPEG".into(),
        7 => "JPEG".into(),
        8 | 32946 => "Deflate".into(),
        32773 => "PackBits".into(),
        34712 => "JPEG 2000".into(),
        34887 => "LERC".into(),
        34925 => "LZMA".into(),
        50000 => "ZSTD".into(),
        50001 => "WebP".into(),
        50002 => "JPEG XL".into(),
        other => format!("#{other}"),
    }
}

pub fn supported_compression(code: u16) -> bool {
    matches!(code, 1 | 5 | 7 | 8 | 32946 | 32773 | 50000)
}

/// Decoded samples, interleaved, `width * rows * samples` long.
pub fn decode_chunk(raw: &[u8], layout: &ChunkLayout<'_>) -> Result<Vec<f32>, String> {
    if layout.compression == 7 {
        return decode_jpeg(raw, layout);
    }
    let row_bytes = (layout.width * layout.samples * layout.bits as usize).div_ceil(8);
    let expected = row_bytes * layout.rows;
    let mut data = match layout.compression {
        1 => raw.to_vec(),
        5 => lzw(raw, expected)?,
        8 | 32946 => inflate(raw, expected)?,
        32773 => packbits(raw, expected),
        50000 => zstd(raw, expected)?,
        other => return Err(format!("unsupported compression {}", compression_name(other))),
    };
    if data.len() < expected {
        // Tolerate short chunks (some writers trim the last strip).
        data.resize(expected, 0);
    }
    data.truncate(expected);
    let bytes_per_sample = layout.bits as usize / 8;
    match layout.predictor {
        2 if layout.bits >= 8 => {
            horizontal_predictor(&mut data, layout, row_bytes, bytes_per_sample);
        }
        3 if layout.format == 3 && matches!(layout.bits, 16 | 32 | 64) => {
            return Ok(float_predictor(&data, layout, row_bytes, bytes_per_sample));
        }
        _ => {}
    }
    samples_to_f32(&data, layout, row_bytes)
}

fn lzw(raw: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(expected);
    let mut decoder = weezl::decode::Decoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8);
    let result = decoder.into_vec(&mut out).decode_all(raw);
    match result.status {
        Ok(_) => Ok(out),
        // Some writers omit the end code; keep what was decoded.
        Err(_) if !out.is_empty() => Ok(out),
        Err(error) => Err(format!("LZW: {error}")),
    }
}

fn inflate(raw: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(expected);
    let mut decoder = flate2::read::ZlibDecoder::new(raw);
    match decoder.read_to_end(&mut out) {
        Ok(_) => Ok(out),
        Err(_) if out.len() >= expected => Ok(out),
        Err(error) => {
            // A few writers emit raw deflate without the zlib header.
            let mut raw_out = Vec::with_capacity(expected);
            if flate2::read::DeflateDecoder::new(raw).read_to_end(&mut raw_out).is_ok() && !raw_out.is_empty() {
                return Ok(raw_out);
            }
            Err(format!("Deflate: {error}"))
        }
    }
}

fn zstd(raw: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    let mut slice = raw;
    let mut decoder = ruzstd::decoding::StreamingDecoder::new(&mut slice).map_err(|error| format!("ZSTD: {error}"))?;
    let mut out = Vec::with_capacity(expected);
    decoder.read_to_end(&mut out).map_err(|error| format!("ZSTD: {error}"))?;
    Ok(out)
}

fn packbits(raw: &[u8], expected: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(expected);
    let mut i = 0;
    while i < raw.len() && out.len() < expected {
        let n = raw[i] as i8;
        i += 1;
        if n >= 0 {
            let count = n as usize + 1;
            let end = (i + count).min(raw.len());
            out.extend_from_slice(&raw[i..end]);
            i = end;
        } else if n != -128 {
            let count = (-(n as isize)) as usize + 1;
            if let Some(&byte) = raw.get(i) {
                out.extend(std::iter::repeat_n(byte, count));
            }
            i += 1;
        }
    }
    out
}

fn decode_jpeg(raw: &[u8], layout: &ChunkLayout<'_>) -> Result<Vec<f32>, String> {
    let stream: Vec<u8> = match layout.jpeg_tables {
        // Tables end with EOI and the tile starts with SOI; splice them together.
        Some(tables) if tables.len() >= 4 && raw.len() >= 2 => {
            let mut joined = tables[..tables.len() - 2].to_vec();
            joined.extend_from_slice(&raw[2..]);
            joined
        }
        _ => raw.to_vec(),
    };
    let mut decoder = jpeg_decoder::Decoder::new(stream.as_slice());
    match layout.photometric {
        6 => decoder.set_color_transform(jpeg_decoder::ColorTransform::YCbCr),
        2 => decoder.set_color_transform(jpeg_decoder::ColorTransform::RGB),
        _ => {}
    }
    let pixels = decoder.decode().map_err(|error| format!("JPEG: {error}"))?;
    let info = decoder.info().ok_or("JPEG without header")?;
    let components = match info.pixel_format {
        jpeg_decoder::PixelFormat::L8 => 1,
        jpeg_decoder::PixelFormat::L16 => return Err("16-bit JPEG is not supported".into()),
        jpeg_decoder::PixelFormat::RGB24 => 3,
        jpeg_decoder::PixelFormat::CMYK32 => 4,
    };
    let (jw, jh) = (info.width as usize, info.height as usize);
    let samples = layout.samples;
    let mut out = vec![0.0f32; layout.width * layout.rows * samples];
    let take = components.min(samples);
    for y in 0..layout.rows.min(jh) {
        for x in 0..layout.width.min(jw) {
            let src = (y * jw + x) * components;
            let dst = (y * layout.width + x) * samples;
            for s in 0..take {
                out[dst + s] = pixels[src + s] as f32;
            }
        }
    }
    Ok(out)
}

fn horizontal_predictor(data: &mut [u8], layout: &ChunkLayout<'_>, row_bytes: usize, bps: usize) {
    let stride = layout.samples;
    let order = layout.order;
    for row in data.chunks_mut(row_bytes) {
        let count = row.len() / bps;
        match bps {
            1 => {
                for i in stride..count {
                    row[i] = row[i].wrapping_add(row[i - stride]);
                }
            }
            2 => {
                for i in stride..count {
                    let previous = order.u16(&row[(i - stride) * 2..]);
                    let value = order.u16(&row[i * 2..]).wrapping_add(previous);
                    let bytes = match order {
                        ByteOrder::Little => value.to_le_bytes(),
                        ByteOrder::Big => value.to_be_bytes(),
                    };
                    row[i * 2..i * 2 + 2].copy_from_slice(&bytes);
                }
            }
            4 => {
                for i in stride..count {
                    let previous = order.u32(&row[(i - stride) * 4..]);
                    let value = order.u32(&row[i * 4..]).wrapping_add(previous);
                    let bytes = match order {
                        ByteOrder::Little => value.to_le_bytes(),
                        ByteOrder::Big => value.to_be_bytes(),
                    };
                    row[i * 4..i * 4 + 4].copy_from_slice(&bytes);
                }
            }
            8 => {
                for i in stride..count {
                    let previous = order.u64(&row[(i - stride) * 8..]);
                    let value = order.u64(&row[i * 8..]).wrapping_add(previous);
                    let bytes = match order {
                        ByteOrder::Little => value.to_le_bytes(),
                        ByteOrder::Big => value.to_be_bytes(),
                    };
                    row[i * 8..i * 8 + 8].copy_from_slice(&bytes);
                }
            }
            _ => {}
        }
    }
}

/// Floating point predictor (Adobe TIFF tech note 3): byte differences over
/// the row, then the bytes of each value are stored in planes, most
/// significant plane first.
fn float_predictor(data: &[u8], layout: &ChunkLayout<'_>, row_bytes: usize, bps: usize) -> Vec<f32> {
    let stride = layout.samples;
    let words = layout.width * layout.samples;
    let mut out = Vec::with_capacity(words * layout.rows);
    let mut row = vec![0u8; row_bytes];
    for chunk in data.chunks(row_bytes) {
        row[..chunk.len()].copy_from_slice(chunk);
        for i in stride..row_bytes {
            row[i] = row[i].wrapping_add(row[i - stride]);
        }
        for word in 0..words {
            let mut be = [0u8; 8];
            for byte in 0..bps {
                be[byte] = row[byte * words + word];
            }
            out.push(match bps {
                2 => f16_to_f32(u16::from_be_bytes([be[0], be[1]])),
                4 => f32::from_be_bytes([be[0], be[1], be[2], be[3]]),
                _ => f64::from_be_bytes(be) as f32,
            });
        }
    }
    out
}

fn f16_to_f32(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = (bits >> 10) & 0x1f;
    let fraction = (bits & 0x3ff) as f32;
    match exponent {
        0 => sign * fraction * 2f32.powi(-24),
        31 => {
            if fraction == 0.0 {
                sign * f32::INFINITY
            } else {
                f32::NAN
            }
        }
        _ => sign * (1.0 + fraction / 1024.0) * 2f32.powi(exponent as i32 - 15),
    }
}

fn samples_to_f32(data: &[u8], layout: &ChunkLayout<'_>, row_bytes: usize) -> Result<Vec<f32>, String> {
    let order = layout.order;
    let count = layout.width * layout.samples * layout.rows;
    let format = layout.format;
    let mut out = Vec::with_capacity(count);
    match layout.bits {
        1 | 2 | 4 => {
            let bits = layout.bits as usize;
            let per_row = layout.width * layout.samples;
            let mask = (1u8 << bits) - 1;
            for row in data.chunks(row_bytes) {
                for i in 0..per_row {
                    let bit = i * bits;
                    let byte = row.get(bit / 8).copied().unwrap_or(0);
                    let shift = 8 - bits - bit % 8;
                    out.push(((byte >> shift) & mask) as f32);
                }
            }
        }
        8 => out.extend(data.iter().map(|&b| if format == 2 { b as i8 as f32 } else { b as f32 })),
        16 => {
            for chunk in data.as_chunks::<2>().0 {
                let raw = order.u16(chunk);
                out.push(match format {
                    2 => raw as i16 as f32,
                    3 => f16_to_f32(raw),
                    _ => raw as f32,
                });
            }
        }
        32 => {
            for chunk in data.as_chunks::<4>().0 {
                let raw = order.u32(chunk);
                out.push(match format {
                    2 => raw as i32 as f32,
                    3 => f32::from_bits(raw),
                    _ => raw as f32,
                });
            }
        }
        64 => {
            for chunk in data.as_chunks::<8>().0 {
                let raw = order.u64(chunk);
                out.push(match format {
                    2 => raw as i64 as f32,
                    3 => f64::from_bits(raw) as f32,
                    _ => raw as f32,
                });
            }
        }
        other => return Err(format!("{other} bits per sample are not supported")),
    }
    out.resize(count, 0.0);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(bits: u16, format: u16, compression: u16, predictor: u16) -> ChunkLayout<'static> {
        ChunkLayout {
            width: 4,
            rows: 2,
            samples: 1,
            bits,
            format,
            order: ByteOrder::Little,
            compression,
            predictor,
            photometric: 1,
            jpeg_tables: None,
        }
    }

    #[test]
    fn packbits_runs() {
        // literal run of 3, repeat of 4
        assert_eq!(packbits(&[2, 1, 2, 3, 0xfd, 9], 7), vec![1, 2, 3, 9, 9, 9, 9]);
    }

    #[test]
    fn horizontal_predictor_16bit() {
        let mut raw = Vec::new();
        for value in [10u16, 1, 1, 1, 100, 0, 0, 65535] {
            raw.extend_from_slice(&value.to_le_bytes());
        }
        let out = decode_chunk(&raw, &layout(16, 1, 1, 2)).unwrap();
        assert_eq!(out, vec![10.0, 11.0, 12.0, 13.0, 100.0, 100.0, 100.0, 99.0]);
    }

    #[test]
    fn float_predictor_round_trip() {
        let values = [1.5f32, -2.25, 3.0, 1e6, 0.0, 7.0, 8.0, 9.0];
        // Encode like libtiff: split bytes into planes (big endian), then difference.
        let words = 4;
        let mut encoded = Vec::new();
        for row in values.chunks(words) {
            let mut planes = vec![0u8; words * 4];
            for (word, value) in row.iter().enumerate() {
                for (byte, b) in value.to_be_bytes().iter().enumerate() {
                    planes[byte * words + word] = *b;
                }
            }
            for i in (1..planes.len()).rev() {
                planes[i] = planes[i].wrapping_sub(planes[i - 1]);
            }
            encoded.extend(planes);
        }
        let out = decode_chunk(&encoded, &layout(32, 3, 1, 3)).unwrap();
        assert_eq!(out, values);
    }

    #[test]
    fn deflate_and_signed() {
        use std::io::Write as _;
        let raw: Vec<u8> = [-5i16, 7, 0, -1, 1, 2, 3, 4].iter().flat_map(|v| v.to_le_bytes()).collect();
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&raw).unwrap();
        let out = decode_chunk(&encoder.finish().unwrap(), &layout(16, 2, 8, 1)).unwrap();
        assert_eq!(out[..4], [-5.0, 7.0, 0.0, -1.0]);
    }

    #[test]
    fn lzw_round_trip() {
        let raw: Vec<u8> = (0..8u8).collect();
        let encoded = weezl::encode::Encoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8).encode(&raw).unwrap();
        let out = decode_chunk(&encoded, &layout(8, 1, 5, 1)).unwrap();
        assert_eq!(out, (0..8).map(|v| v as f32).collect::<Vec<_>>());
    }

    #[test]
    fn one_bit_rows_are_padded() {
        // width 4, 1 bit: each row uses one byte
        let out = decode_chunk(&[0b1010_0000, 0b0101_0000], &layout(1, 1, 1, 1)).unwrap();
        assert_eq!(out, vec![1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0]);
    }

    #[test]
    fn half_floats() {
        assert_eq!(f16_to_f32(0x3c00), 1.0);
        assert_eq!(f16_to_f32(0xc000), -2.0);
        assert!(f16_to_f32(0x7e00).is_nan());
    }
}
