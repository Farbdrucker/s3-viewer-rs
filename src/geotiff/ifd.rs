//! TIFF and BigTIFF image file directories.

use std::collections::HashMap;

use super::source::Cached;

pub mod tag {
    pub const NEW_SUBFILE_TYPE: u16 = 254;
    pub const IMAGE_WIDTH: u16 = 256;
    pub const IMAGE_LENGTH: u16 = 257;
    pub const BITS_PER_SAMPLE: u16 = 258;
    pub const COMPRESSION: u16 = 259;
    pub const PHOTOMETRIC: u16 = 262;
    pub const STRIP_OFFSETS: u16 = 273;
    pub const SAMPLES_PER_PIXEL: u16 = 277;
    pub const ROWS_PER_STRIP: u16 = 278;
    pub const STRIP_BYTE_COUNTS: u16 = 279;
    pub const PLANAR_CONFIGURATION: u16 = 284;
    pub const PREDICTOR: u16 = 317;
    pub const COLOR_MAP: u16 = 320;
    pub const TILE_WIDTH: u16 = 322;
    pub const TILE_LENGTH: u16 = 323;
    pub const TILE_OFFSETS: u16 = 324;
    pub const TILE_BYTE_COUNTS: u16 = 325;
    pub const EXTRA_SAMPLES: u16 = 338;
    pub const SAMPLE_FORMAT: u16 = 339;
    pub const JPEG_TABLES: u16 = 347;
    pub const MODEL_PIXEL_SCALE: u16 = 33550;
    pub const MODEL_TIEPOINT: u16 = 33922;
    pub const MODEL_TRANSFORMATION: u16 = 34264;
    pub const GEO_KEY_DIRECTORY: u16 = 34735;
    pub const GEO_DOUBLE_PARAMS: u16 = 34736;
    pub const GEO_ASCII_PARAMS: u16 = 34737;
    pub const GDAL_METADATA: u16 = 42112;
    pub const GDAL_NODATA: u16 = 42113;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ByteOrder {
    Little,
    Big,
}

impl ByteOrder {
    pub fn u16(self, b: &[u8]) -> u16 {
        let bytes = [b[0], b[1]];
        match self {
            ByteOrder::Little => u16::from_le_bytes(bytes),
            ByteOrder::Big => u16::from_be_bytes(bytes),
        }
    }

    pub fn u32(self, b: &[u8]) -> u32 {
        let bytes = [b[0], b[1], b[2], b[3]];
        match self {
            ByteOrder::Little => u32::from_le_bytes(bytes),
            ByteOrder::Big => u32::from_be_bytes(bytes),
        }
    }

    pub fn u64(self, b: &[u8]) -> u64 {
        let bytes: [u8; 8] = b[..8].try_into().expect("8 bytes");
        match self {
            ByteOrder::Little => u64::from_le_bytes(bytes),
            ByteOrder::Big => u64::from_be_bytes(bytes),
        }
    }
}

#[derive(Clone, Debug)]
struct Entry {
    kind: u16,
    count: u64,
    /// The value bytes when they fit into the entry, otherwise empty.
    inline: Vec<u8>,
    offset: u64,
}

fn type_size(kind: u16) -> u64 {
    match kind {
        1 | 2 | 6 | 7 => 1,
        3 | 8 => 2,
        4 | 9 | 11 | 13 => 4,
        5 | 10 | 12 | 16 | 17 | 18 => 8,
        _ => 0,
    }
}

#[derive(Clone, Debug)]
pub struct Ifd {
    entries: HashMap<u16, Entry>,
    order: ByteOrder,
}

pub struct Header {
    pub order: ByteOrder,
    pub big: bool,
    pub first_ifd: u64,
}

pub fn read_header(source: &Cached<'_>) -> Result<Header, String> {
    let head = source.read(0, 16.min(source.size()))?;
    if head.len() < 8 {
        return Err("not a TIFF file".into());
    }
    let order = match &head[..2] {
        b"II" => ByteOrder::Little,
        b"MM" => ByteOrder::Big,
        _ => return Err("not a TIFF file".into()),
    };
    match order.u16(&head[2..4]) {
        42 => Ok(Header { order, big: false, first_ifd: order.u32(&head[4..8]) as u64 }),
        43 if head.len() >= 16 => Ok(Header { order, big: true, first_ifd: order.u64(&head[8..16]) }),
        _ => Err("not a TIFF file".into()),
    }
}

/// Reads the chain of directories, at most `limit` of them.
pub fn read_ifds(source: &Cached<'_>, header: &Header, limit: usize) -> Result<Vec<Ifd>, String> {
    let mut ifds = Vec::new();
    let mut offset = header.first_ifd;
    let mut seen = std::collections::HashSet::new();
    while offset != 0 && ifds.len() < limit && seen.insert(offset) {
        let (ifd, next) = read_ifd(source, header, offset)?;
        ifds.push(ifd);
        offset = next;
    }
    if ifds.is_empty() {
        return Err("the TIFF has no images".into());
    }
    Ok(ifds)
}

fn read_ifd(source: &Cached<'_>, header: &Header, offset: u64) -> Result<(Ifd, u64), String> {
    let order = header.order;
    let (count_size, entry_size, inline_size, next_size) = if header.big { (8, 20, 8, 8) } else { (2, 12, 4, 4) };
    let count_bytes = source.read(offset, count_size)?;
    let count = if header.big { order.u64(&count_bytes) } else { order.u16(&count_bytes) as u64 };
    if count > 10_000 {
        return Err("broken TIFF directory".into());
    }
    let table = source.read(offset + count_size, count * entry_size + next_size)?;
    if (table.len() as u64) < count * entry_size + next_size {
        return Err("broken TIFF directory".into());
    }
    let mut entries = HashMap::new();
    for index in 0..count as usize {
        let raw = &table[index * entry_size as usize..(index + 1) * entry_size as usize];
        let tag = order.u16(&raw[0..2]);
        let kind = order.u16(&raw[2..4]);
        let (count, value) = if header.big {
            (order.u64(&raw[4..12]), &raw[12..20])
        } else {
            (order.u32(&raw[4..8]) as u64, &raw[8..12])
        };
        let size = type_size(kind).saturating_mul(count);
        let entry = if size <= inline_size {
            Entry { kind, count, inline: value[..size as usize].to_vec(), offset: 0 }
        } else {
            let offset = if header.big { order.u64(value) } else { order.u32(value) as u64 };
            Entry { kind, count, inline: Vec::new(), offset }
        };
        entries.insert(tag, entry);
    }
    let next_raw = &table[(count * entry_size) as usize..];
    let next = if header.big { order.u64(next_raw) } else { order.u32(next_raw) as u64 };
    Ok((Ifd { entries, order }, next))
}

impl Ifd {
    pub fn has(&self, tag: u16) -> bool {
        self.entries.contains_key(&tag)
    }

    fn raw(&self, source: &Cached<'_>, tag: u16) -> Result<Option<(u16, u64, Vec<u8>)>, String> {
        let Some(entry) = self.entries.get(&tag) else { return Ok(None) };
        if type_size(entry.kind) == 0 {
            return Ok(None);
        }
        let size = type_size(entry.kind).saturating_mul(entry.count);
        let bytes = if entry.inline.len() as u64 == size {
            entry.inline.clone()
        } else {
            if size > 64 * 1024 * 1024 {
                return Err("TIFF tag too large".into());
            }
            source.read(entry.offset, size)?
        };
        Ok(Some((entry.kind, entry.count, bytes)))
    }

    /// Integer values of a tag (BYTE, SHORT, LONG, LONG8 and signed variants).
    pub fn uints(&self, source: &Cached<'_>, tag: u16) -> Result<Option<Vec<u64>>, String> {
        let Some((kind, count, bytes)) = self.raw(source, tag)? else { return Ok(None) };
        let order = self.order;
        let size = type_size(kind) as usize;
        let mut values = Vec::with_capacity(count as usize);
        for chunk in bytes.chunks_exact(size).take(count as usize) {
            values.push(match kind {
                1 | 7 => chunk[0] as u64,
                6 => chunk[0] as i8 as i64 as u64,
                3 => order.u16(chunk) as u64,
                8 => order.u16(chunk) as i16 as i64 as u64,
                4 | 13 => order.u32(chunk) as u64,
                9 => order.u32(chunk) as i32 as i64 as u64,
                16 | 18 => order.u64(chunk),
                17 => order.u64(chunk),
                _ => return Ok(None),
            });
        }
        Ok(Some(values))
    }

    pub fn uint(&self, source: &Cached<'_>, tag: u16) -> Result<Option<u64>, String> {
        Ok(self.uints(source, tag)?.and_then(|values| values.first().copied()))
    }

    pub fn floats(&self, source: &Cached<'_>, tag: u16) -> Result<Option<Vec<f64>>, String> {
        let Some((kind, count, bytes)) = self.raw(source, tag)? else { return Ok(None) };
        let order = self.order;
        let size = type_size(kind) as usize;
        let mut values = Vec::with_capacity(count as usize);
        for chunk in bytes.chunks_exact(size).take(count as usize) {
            values.push(match kind {
                11 => f32::from_bits(order.u32(chunk)) as f64,
                12 => f64::from_bits(order.u64(chunk)),
                5 => order.u32(&chunk[0..4]) as f64 / order.u32(&chunk[4..8]).max(1) as f64,
                10 => order.u32(&chunk[0..4]) as i32 as f64 / (order.u32(&chunk[4..8]) as i32).max(1) as f64,
                3 => order.u16(chunk) as f64,
                4 => order.u32(chunk) as f64,
                _ => return Ok(None),
            });
        }
        Ok(Some(values))
    }

    pub fn ascii(&self, source: &Cached<'_>, tag: u16) -> Result<Option<String>, String> {
        let Some((_, _, bytes)) = self.raw(source, tag)? else { return Ok(None) };
        let text = String::from_utf8_lossy(&bytes);
        Ok(Some(text.trim_end_matches('\0').to_owned()))
    }

    pub fn bytes(&self, source: &Cached<'_>, tag: u16) -> Result<Option<Vec<u8>>, String> {
        Ok(self.raw(source, tag)?.map(|(_, _, bytes)| bytes))
    }
}
