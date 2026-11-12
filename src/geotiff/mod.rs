//! Previews of large (Geo)TIFF files over range requests.
//!
//! Only the header, the tile index and the tiles of one resolution level
//! are fetched. Cloud-optimised GeoTIFFs with overviews load in a few small
//! requests; files without overviews are sampled within a byte budget.

pub mod decode;
pub mod geo;
pub mod ifd;
pub mod render;
pub mod source;

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use decode::{ChunkLayout, compression_name, decode_chunk, supported_compression};
use geo::GeoInfo;
use ifd::{Ifd, tag};
pub use render::{BandStats, Colormap, Raster, Stretch, Style};
use source::Cached;
pub use source::{FnSource, MemorySource, RangeSource};

use crate::tr;

/// Bands beyond this are not kept for the preview.
const MAX_BANDS: usize = 12;
/// Requests closer than this are merged into one.
const MERGE_GAP: u64 = 256 * 1024;
const MERGE_MAX: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Options {
    /// Longest side of the preview raster.
    pub edge: u32,
    /// Upper bound for tile data fetched for one preview.
    pub budget: u64,
    pub threads: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options { edge: 1024, budget: 64 * 1024 * 1024, threads: 6 }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Layout {
    Tiled { width: u32, height: u32 },
    Stripped { rows: u32 },
}

#[derive(Clone, Debug)]
pub struct TiffInfo {
    pub width: u64,
    pub height: u64,
    pub samples: u16,
    pub bits: u16,
    pub sample_format: u16,
    pub compression: String,
    pub layout: Layout,
    pub planar_separate: bool,
    pub photometric: u16,
    pub overviews: Vec<(u64, u64)>,
    pub pages: usize,
    pub nodata: Option<f64>,
    pub geo: Option<GeoInfo>,
    pub band_names: Vec<String>,
    pub file_size: u64,
    pub fetched: u64,
    pub requests: u64,
    /// 0 is full resolution, n the n-th overview.
    pub level: usize,
    pub level_size: (u64, u64),
    /// (used, total) chunks when the preview had to be sampled.
    pub sampled: Option<(usize, usize)>,
    /// Compressed size of all chunks of the chosen level.
    pub level_bytes: u64,
}

impl TiffInfo {
    pub fn dtype(&self) -> String {
        let kind = match self.sample_format {
            2 => "Int",
            3 => "Float",
            _ => "UInt",
        };
        format!("{kind}{}", self.bits)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum TiffError {
    /// A valid file this reader cannot show; the text is meant for the user.
    Unsupported(String),
    /// Broken file or failed read, with technical detail.
    Failed(String),
    Cancelled,
}

impl From<String> for TiffError {
    fn from(detail: String) -> Self {
        TiffError::Failed(detail)
    }
}

impl From<&str> for TiffError {
    fn from(detail: &str) -> Self {
        TiffError::Failed(detail.to_owned())
    }
}

impl std::fmt::Display for TiffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TiffError::Unsupported(text) | TiffError::Failed(text) => f.write_str(text),
            TiffError::Cancelled => f.write_str("cancelled"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct TiffPreview {
    pub info: TiffInfo,
    pub raster: Raster,
    pub notice: String,
}

struct Image {
    width: u64,
    height: u64,
    samples: usize,
    bits: u16,
    format: u16,
    compression: u16,
    predictor: u16,
    photometric: u16,
    planar_separate: bool,
    tile: Option<(u64, u64)>,
    rows_per_strip: u64,
    offsets: Vec<u64>,
    byte_counts: Vec<u64>,
    jpeg_tables: Option<Vec<u8>>,
    colormap: Option<Vec<u64>>,
    extra_samples: Vec<u64>,
}

impl Image {
    fn read(source: &Cached<'_>, ifd: &Ifd) -> Result<Image, String> {
        let width = ifd.uint(source, tag::IMAGE_WIDTH)?.ok_or("TIFF without width")?;
        let height = ifd.uint(source, tag::IMAGE_LENGTH)?.ok_or("TIFF without height")?;
        let samples = ifd.uint(source, tag::SAMPLES_PER_PIXEL)?.unwrap_or(1).max(1) as usize;
        let bits = ifd.uint(source, tag::BITS_PER_SAMPLE)?.unwrap_or(1) as u16;
        let format = ifd.uint(source, tag::SAMPLE_FORMAT)?.unwrap_or(1) as u16;
        let compression = ifd.uint(source, tag::COMPRESSION)?.unwrap_or(1) as u16;
        let predictor = ifd.uint(source, tag::PREDICTOR)?.unwrap_or(1) as u16;
        let photometric = ifd.uint(source, tag::PHOTOMETRIC)?.unwrap_or(1) as u16;
        let planar_separate = ifd.uint(source, tag::PLANAR_CONFIGURATION)?.unwrap_or(1) == 2 && samples > 1;
        let tile = match (ifd.uint(source, tag::TILE_WIDTH)?, ifd.uint(source, tag::TILE_LENGTH)?) {
            (Some(w), Some(h)) if w > 0 && h > 0 => Some((w, h)),
            _ => None,
        };
        let (offsets, byte_counts) = if tile.is_some() {
            (ifd.uints(source, tag::TILE_OFFSETS)?, ifd.uints(source, tag::TILE_BYTE_COUNTS)?)
        } else {
            (ifd.uints(source, tag::STRIP_OFFSETS)?, ifd.uints(source, tag::STRIP_BYTE_COUNTS)?)
        };
        let offsets = offsets.ok_or("TIFF without data offsets")?;
        let byte_counts = byte_counts.ok_or("TIFF without data sizes")?;
        let rows_per_strip = ifd.uint(source, tag::ROWS_PER_STRIP)?.unwrap_or(height).clamp(1, height.max(1));
        Ok(Image {
            width,
            height,
            samples,
            bits,
            format,
            compression,
            predictor,
            photometric,
            planar_separate,
            tile,
            rows_per_strip,
            offsets,
            byte_counts,
            jpeg_tables: if compression == 7 { ifd.bytes(source, tag::JPEG_TABLES)? } else { None },
            colormap: if photometric == 3 { ifd.uints(source, tag::COLOR_MAP)? } else { None },
            extra_samples: ifd.uints(source, tag::EXTRA_SAMPLES)?.unwrap_or_default(),
        })
    }

    /// Chunk size and grid: (chunk width, chunk height, across, down).
    fn grid(&self) -> (u64, u64, u64, u64) {
        match self.tile {
            Some((w, h)) => (w, h, self.width.div_ceil(w), self.height.div_ceil(h)),
            None => (self.width, self.rows_per_strip, 1, self.height.div_ceil(self.rows_per_strip)),
        }
    }
}

struct Accumulator {
    width: usize,
    height: usize,
    sums: Vec<Vec<f32>>,
    counts: Vec<Vec<u32>>,
}

struct Plan<'a> {
    image: &'a Image,
    order: ifd::ByteOrder,
    scale: f64,
    stride: (u64, u64),
    bands: usize,
    nodata: Option<f32>,
    categorical: bool,
}

/// A merged read of one or more chunks.
struct Group {
    offset: u64,
    length: u64,
    /// (chunk index, offset inside the group, length)
    chunks: Vec<(usize, u64, u64)>,
}

pub fn load(
    source: &dyn RangeSource,
    options: &Options,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<TiffPreview, TiffError> {
    let cached = Cached::new(source);
    let header = ifd::read_header(&cached)?;
    let ifds = ifd::read_ifds(&cached, &header, 64)?;
    let main = &ifds[0];
    let main_image = Image::read(&cached, main)?;

    // Classify the other directories into overviews and pages.
    let mut overviews: Vec<(usize, u64, u64)> = Vec::new();
    let mut pages = 1;
    for (index, ifd) in ifds.iter().enumerate().skip(1) {
        let subfile = ifd.uint(&cached, tag::NEW_SUBFILE_TYPE)?.unwrap_or(0);
        if subfile & 4 != 0 {
            continue; // transparency mask
        }
        if subfile & 1 == 0 {
            pages += 1;
            continue;
        }
        let samples = ifd.uint(&cached, tag::SAMPLES_PER_PIXEL)?.unwrap_or(1) as usize;
        let (Some(w), Some(h)) = (ifd.uint(&cached, tag::IMAGE_WIDTH)?, ifd.uint(&cached, tag::IMAGE_LENGTH)?) else {
            continue;
        };
        if samples == main_image.samples && w < main_image.width {
            overviews.push((index, w, h));
        }
    }
    overviews.sort_by_key(|overview| std::cmp::Reverse(overview.1));

    let nodata = main.ascii(&cached, tag::GDAL_NODATA)?.and_then(|text| text.trim().parse::<f64>().ok());
    let geo = GeoInfo::parse(
        main.uints(&cached, tag::GEO_KEY_DIRECTORY)?.as_deref(),
        main.ascii(&cached, tag::GEO_ASCII_PARAMS)?.as_deref(),
        main.floats(&cached, tag::MODEL_PIXEL_SCALE)?.as_deref(),
        main.floats(&cached, tag::MODEL_TIEPOINT)?.as_deref(),
        main.floats(&cached, tag::MODEL_TRANSFORMATION)?.as_deref(),
    );
    let band_names = main
        .ascii(&cached, tag::GDAL_METADATA)?
        .map(|xml| band_descriptions(&xml, main_image.samples))
        .unwrap_or_default();

    // Smallest level that still covers the preview edge.
    let edge = options.edge.max(16) as u64;
    let mut level = 0;
    for (position, (_, w, h)) in overviews.iter().enumerate() {
        if (*w).max(*h) >= edge {
            level = position + 1;
        }
    }
    let level_image;
    let image = if level == 0 {
        &main_image
    } else {
        level_image = Image::read(&cached, &ifds[overviews[level - 1].0])?;
        &level_image
    };

    let mut info = TiffInfo {
        width: main_image.width,
        height: main_image.height,
        samples: main_image.samples as u16,
        bits: main_image.bits,
        sample_format: main_image.format,
        compression: compression_name(main_image.compression),
        layout: match main_image.tile {
            Some((w, h)) => Layout::Tiled { width: w as u32, height: h as u32 },
            None => Layout::Stripped { rows: main_image.rows_per_strip as u32 },
        },
        planar_separate: main_image.planar_separate,
        photometric: main_image.photometric,
        overviews: overviews.iter().map(|(_, w, h)| (*w, *h)).collect(),
        pages,
        nodata,
        geo,
        band_names,
        file_size: cached.size(),
        fetched: 0,
        requests: 0,
        level,
        level_size: (image.width, image.height),
        sampled: None,
        level_bytes: 0,
    };

    if !supported_compression(image.compression) {
        return Err(TiffError::Unsupported(tr!("tiff_compression", name = compression_name(image.compression))));
    }
    if !matches!(image.bits, 1 | 2 | 4 | 8 | 16 | 32 | 64) {
        return Err(TiffError::Unsupported(format!("{} bits per sample are not supported", image.bits)));
    }
    if image.width == 0 || image.height == 0 {
        return Err("empty TIFF image".into());
    }

    let (chunk_w, chunk_h, across, down) = image.grid();
    // A broken or hostile file must not make us allocate gigabytes for one tile.
    let chunk_samples = chunk_w.saturating_mul(chunk_h).saturating_mul(image.samples as u64);
    if chunk_samples > 256 * 1024 * 1024 {
        return Err(format!("tiles of {chunk_w}×{chunk_h} pixels are too large to preview").into());
    }
    let planes = if image.planar_separate { image.samples.min(MAX_BANDS) } else { 1 };
    let chunks_per_plane = (across * down) as usize;
    let needed = chunks_per_plane * planes;
    if image.offsets.len() < needed || image.byte_counts.len() < needed {
        return Err("TIFF tile index is incomplete".into());
    }
    let total_bytes: u64 = (0..planes)
        .flat_map(|plane| (0..chunks_per_plane).map(move |chunk| plane * chunks_per_plane + chunk))
        .map(|index| image.byte_counts[index])
        .sum();

    info.level_bytes = total_bytes;
    // Thin out the chunk grid when the level is larger than the budget.
    let mut stride = (1u64, 1u64);
    if total_bytes > options.budget {
        let ratio = total_bytes as f64 / options.budget.max(1) as f64;
        let sx = (ratio.sqrt().ceil() as u64).clamp(1, across);
        let sy = ((ratio / sx as f64).ceil() as u64).clamp(1, down);
        stride = (sx, sy);
        let used = across.div_ceil(sx) * down.div_ceil(sy);
        info.sampled = Some((used as usize, (across * down) as usize));
    }

    let scale = (image.width.max(image.height) as f64 / edge as f64).max(1.0);
    let out_w = ((image.width as f64 / scale).ceil() as usize).max(1);
    let out_h = ((image.height as f64 / scale).ceil() as usize).max(1);
    let bands = image.samples.min(MAX_BANDS);
    let plan = Plan {
        image,
        order: header.order,
        scale,
        stride,
        bands,
        nodata: nodata.map(|value| value as f32),
        categorical: image.colormap.is_some(),
    };

    let mut selected: Vec<usize> = Vec::new();
    for plane in 0..planes {
        for ty in (0..down).step_by(stride.1 as usize) {
            for tx in (0..across).step_by(stride.0 as usize) {
                let index = plane * chunks_per_plane + (ty * across + tx) as usize;
                if image.byte_counts[index] > 0 && image.offsets[index] > 0 {
                    selected.push(index);
                }
            }
        }
    }
    let groups = merge_reads(image, selected);

    let accumulator = Mutex::new(Accumulator {
        width: out_w,
        height: out_h,
        sums: vec![vec![0.0; out_w * out_h]; bands],
        counts: vec![vec![0; out_w * out_h]; bands],
    });
    let next = AtomicUsize::new(0);
    let failure: Mutex<Option<String>> = Mutex::new(None);
    let threads = options.threads.clamp(1, 16).min(groups.len().max(1));
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    if cancelled() || failure.lock().map(|f| f.is_some()).unwrap_or(true) {
                        return;
                    }
                    let index = next.fetch_add(1, Ordering::SeqCst);
                    let Some(group) = groups.get(index) else { return };
                    if let Err(error) =
                        process_group(&cached, &plan, group, chunk_w, chunk_h, across, chunks_per_plane, &accumulator)
                    {
                        let mut failure = failure.lock().unwrap_or_else(|e| e.into_inner());
                        failure.get_or_insert(error);
                        return;
                    }
                }
            });
        }
    });
    if cancelled() {
        return Err(TiffError::Cancelled);
    }
    if let Some(error) = failure.into_inner().unwrap_or_else(|e| e.into_inner()) {
        return Err(TiffError::Failed(error));
    }

    let accumulator = accumulator.into_inner().unwrap_or_else(|e| e.into_inner());
    let bands: Vec<Vec<f32>> = accumulator
        .sums
        .into_iter()
        .zip(accumulator.counts)
        .map(|(sums, counts)| {
            sums.into_iter()
                .zip(counts)
                .map(|(sum, count)| if count > 0 { sum / count as f32 } else { f32::NAN })
                .collect()
        })
        .collect();
    let stats = bands.iter().map(|band| BandStats::compute(band)).collect();
    let color_samples = main_image.samples.saturating_sub(main_image.extra_samples.len());
    let alpha = main_image
        .extra_samples
        .iter()
        .position(|&kind| kind == 1 || kind == 2)
        .map(|position| color_samples + position)
        .filter(|&index| index < bands.len());
    let palette = image.colormap.as_ref().or(main_image.colormap.as_ref()).and_then(|map| {
        let n = map.len() / 3;
        (n > 0).then(|| {
            (0..n).map(|i| [(map[i] >> 8) as u8, (map[n + i] >> 8) as u8, (map[2 * n + i] >> 8) as u8]).collect()
        })
    });
    let is_u8 = image.bits == 8 && image.format == 1;
    let raster = Raster {
        width: out_w,
        height: out_h,
        full_width: main_image.width,
        full_height: main_image.height,
        bands,
        stats,
        alpha,
        alpha_max: if image.bits == 16 { 65535.0 } else { 255.0 },
        palette,
        min_is_white: image.photometric == 0,
        is_u8,
        rgb_like: matches!(image.photometric, 2 | 6),
    };

    info.fetched = cached.fetched();
    info.requests = cached.requests();
    let mut notice = match (level, info.sampled) {
        (_, Some((used, total))) => tr!("tiff_sampled", used = used, total = total),
        (0, None) => tr!("tiff_full"),
        (level, None) => tr!("tiff_overview", level = level, width = image.width, height = image.height),
    };
    if pages > 1 {
        notice += &tr!("tiff_page", pages = pages);
    }
    Ok(TiffPreview { info, raster, notice })
}

fn merge_reads(image: &Image, mut selected: Vec<usize>) -> Vec<Group> {
    selected.sort_by_key(|&index| image.offsets[index]);
    let mut groups: Vec<Group> = Vec::new();
    for index in selected {
        let (offset, length) = (image.offsets[index], image.byte_counts[index]);
        if let Some(group) = groups.last_mut() {
            let group_end = group.offset + group.length;
            if offset >= group.offset && offset <= group_end + MERGE_GAP && offset + length - group.offset <= MERGE_MAX
            {
                group.length = group.length.max(offset + length - group.offset);
                group.chunks.push((index, offset - group.offset, length));
                continue;
            }
        }
        groups.push(Group { offset, length, chunks: vec![(index, 0, length)] });
    }
    groups
}

#[allow(clippy::too_many_arguments)]
fn process_group(
    source: &Cached<'_>,
    plan: &Plan<'_>,
    group: &Group,
    chunk_w: u64,
    chunk_h: u64,
    across: u64,
    chunks_per_plane: usize,
    accumulator: &Mutex<Accumulator>,
) -> Result<(), String> {
    let data = source.read_direct(group.offset, group.length)?;
    let image = plan.image;
    for &(index, start, length) in &group.chunks {
        let end = (start + length).min(data.len() as u64) as usize;
        let raw = &data[(start as usize).min(end)..end];
        let plane = index / chunks_per_plane;
        let local = (index % chunks_per_plane) as u64;
        let (tx, ty) = (local % across, local / across);
        let x0 = tx * chunk_w;
        let y0 = ty * chunk_h;
        // Strips at the bottom can be shorter than RowsPerStrip.
        let rows = if image.tile.is_some() { chunk_h } else { chunk_h.min(image.height - y0) };
        let samples = if image.planar_separate { 1 } else { image.samples };
        let layout = ChunkLayout {
            width: chunk_w as usize,
            rows: rows as usize,
            samples,
            bits: image.bits,
            format: image.format,
            order: plan.order,
            compression: image.compression,
            predictor: image.predictor,
            photometric: image.photometric,
            jpeg_tables: image.jpeg_tables.as_deref(),
        };
        let values = decode_chunk(raw, &layout)?;
        let valid_w = chunk_w.min(image.width - x0);
        let valid_h = rows.min(image.height - y0);
        let cell_w = (chunk_w * plan.stride.0).min(image.width - x0);
        let cell_h = (chunk_h * plan.stride.1).min(image.height - y0);
        let target_bands: Vec<(usize, usize)> =
            if image.planar_separate { vec![(0, plane)] } else { (0..plan.bands).map(|band| (band, band)).collect() };
        let mut acc = accumulator.lock().unwrap_or_else(|e| e.into_inner());
        accumulate(&mut acc, plan, &values, &layout, (x0, y0), (valid_w, valid_h), (cell_w, cell_h), &target_bands);
    }
    Ok(())
}

/// Box-averages the chunk into the output pixels covering its cell.
#[allow(clippy::too_many_arguments)]
fn accumulate(
    acc: &mut Accumulator,
    plan: &Plan<'_>,
    values: &[f32],
    layout: &ChunkLayout<'_>,
    origin: (u64, u64),
    valid: (u64, u64),
    cell: (u64, u64),
    bands: &[(usize, usize)],
) {
    let scale = plan.scale;
    let (x0, y0) = (origin.0 as f64, origin.1 as f64);
    let (cell_w, cell_h) = (cell.0 as f64, cell.1 as f64);
    let (valid_w, valid_h) = (valid.0 as f64, valid.1 as f64);
    let ox_start = (x0 / scale).floor() as usize;
    let ox_end = (((x0 + cell_w) / scale).ceil() as usize).min(acc.width);
    let oy_start = (y0 / scale).floor() as usize;
    let oy_end = (((y0 + cell_h) / scale).ceil() as usize).min(acc.height);
    // Chunk pixels per level pixel; below 1 when the chunk is stretched over a sampled cell.
    let (fx, fy) = (valid_w / cell_w, valid_h / cell_h);
    for oy in oy_start..oy_end {
        let top = (oy as f64 * scale).max(y0);
        let bottom = ((oy + 1) as f64 * scale).min(y0 + cell_h);
        if bottom <= top {
            continue;
        }
        let ny = if plan.categorical { 1 } else { (((bottom - top) * fy).ceil() as usize).clamp(1, 4) };
        for ox in ox_start..ox_end {
            let left = (ox as f64 * scale).max(x0);
            let right = ((ox + 1) as f64 * scale).min(x0 + cell_w);
            if right <= left {
                continue;
            }
            let nx = if plan.categorical { 1 } else { (((right - left) * fx).ceil() as usize).clamp(1, 4) };
            let out = oy * acc.width + ox;
            for sy in 0..ny {
                let y = top + (bottom - top) * (sy as f64 + 0.5) / ny as f64;
                let v = (((y - y0) * fy) as usize).min(valid.1 as usize - 1);
                for sx in 0..nx {
                    let x = left + (right - left) * (sx as f64 + 0.5) / nx as f64;
                    let u = (((x - x0) * fx) as usize).min(valid.0 as usize - 1);
                    let base = (v * layout.width + u) * layout.samples;
                    for &(sample, band) in bands {
                        let Some(&value) = values.get(base + sample) else { continue };
                        if !value.is_finite() || plan.nodata == Some(value) {
                            continue;
                        }
                        acc.sums[band][out] += value;
                        acc.counts[band][out] += 1;
                    }
                }
            }
        }
    }
}

/// Band descriptions from GDAL's metadata XML.
fn band_descriptions(xml: &str, samples: usize) -> Vec<String> {
    let Ok(document) = crate::xml::parse(xml) else { return Vec::new() };
    let mut names = vec![String::new(); samples];
    for item in document.root.children_named("Item") {
        let attr = |name: &str| item.attrs.iter().find(|(key, _)| key == name).map(|(_, value)| value.as_str());
        if attr("role") == Some("description")
            && let Some(sample) = attr("sample").and_then(|s| s.parse::<usize>().ok())
            && sample < samples
        {
            names[sample] = item.text();
        }
    }
    if names.iter().all(String::is_empty) { Vec::new() } else { names }
}

#[cfg(test)]
mod tests;
