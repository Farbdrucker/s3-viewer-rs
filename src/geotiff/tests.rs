//! Checks the reader against files written by GDAL (see tools/make_fixtures.py).

use super::*;
use serde_json::Value;

fn fixture(name: &str) -> MemorySource {
    let path = format!("{}/tests/data/{name}.tif", env!("CARGO_MANIFEST_DIR"));
    MemorySource(std::fs::read(&path).unwrap_or_else(|error| panic!("{path}: {error}")))
}

fn expected(name: &str) -> Value {
    let path = format!("{}/tests/data/expected.json", env!("CARGO_MANIFEST_DIR"));
    let all: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    all[name].clone()
}

fn full_resolution() -> Options {
    Options { edge: 100_000, ..Options::default() }
}

fn never() -> bool {
    false
}

/// Loads at full resolution and compares GDAL's sample pixels.
fn check_pixels(name: &str, tolerance: f32) -> TiffPreview {
    let preview = load(&fixture(name), &full_resolution(), &never).unwrap_or_else(|e| panic!("{name}: {e}"));
    let want = expected(name);
    let raster = &preview.raster;
    assert_eq!(raster.width as u64, want["width"].as_u64().unwrap(), "{name} width");
    assert_eq!(raster.height as u64, want["height"].as_u64().unwrap(), "{name} height");
    assert_eq!(preview.info.level, 0);
    for point in want["points"].as_array().unwrap() {
        let (x, y) = (point["x"].as_u64().unwrap() as usize, point["y"].as_u64().unwrap() as usize);
        for (band, value) in point["values"].as_array().unwrap().iter().enumerate() {
            let value = value.as_f64().unwrap() as f32;
            let got = raster.bands[band][y * raster.width + x];
            if want["nodata"].as_f64() == Some(value as f64) {
                assert!(got.is_nan(), "{name} ({x},{y}) band {band}: nodata should be NaN, got {got}");
            } else {
                assert!((got - value).abs() <= tolerance, "{name} ({x},{y}) band {band}: {got} vs GDAL {value}");
            }
        }
    }
    let transform = want["geotransform"].as_array().unwrap();
    if let Some(geo) = &preview.info.geo {
        let t = geo.transform.unwrap();
        for (i, value) in transform.iter().enumerate() {
            assert!((t[i] - value.as_f64().unwrap()).abs() < 1e-6, "{name} transform[{i}]");
        }
    }
    preview
}

#[test]
fn cog_rgb_jpeg_full_resolution() {
    let preview = check_pixels("cog_rgb_jpeg", 6.0);
    assert_eq!(preview.info.compression, "JPEG");
    assert_eq!(preview.info.geo.as_ref().unwrap().epsg, Some(25832));
    assert!(preview.raster.rgb_like);
    assert!(!preview.info.overviews.is_empty());
}

#[test]
fn cog_uses_overview_and_fetches_little() {
    let source = fixture("cog_rgb_jpeg");
    let options = Options { edge: 100, ..Options::default() };
    let preview = load(&source, &options, &never).unwrap();
    assert!(preview.info.level > 0, "an overview should be used");
    assert!(preview.raster.width <= 100 && preview.raster.height <= 100);
    // The file is smaller than one header block, so nothing beyond it is requested.
    assert_eq!(preview.info.requests, 1);
    // The colour gradient survives: red grows from left to right.
    let row = preview.raster.height / 2;
    let left = preview.raster.bands[0][row * preview.raster.width + 2];
    let right = preview.raster.bands[0][row * preview.raster.width + preview.raster.width - 3];
    assert!(right > left + 150.0, "{left} → {right}");
}

#[test]
fn dem_float_predictor_and_nodata() {
    let preview = check_pixels("dem_f32_deflate", 1e-3);
    assert_eq!(preview.info.dtype(), "Float32");
    assert_eq!(preview.info.nodata, Some(-9999.0));
    let geo = preview.info.geo.as_ref().unwrap();
    assert_eq!(geo.epsg, Some(3035));
    let (lon, lat) = geo.to_wgs84(4_321_000.0, 3_210_000.0).unwrap();
    assert!((lon - 10.0).abs() < 1e-9 && (lat - 52.0).abs() < 1e-9);
    // Averaged overview keeps NoData out of the stretch, and only a fraction of the file is read.
    let source = fixture("dem_f32_deflate");
    let small = load(&source, &Options { edge: 64, ..Options::default() }, &never).unwrap();
    assert!(small.info.level > 0);
    assert!(small.info.fetched < source.0.len() as u64 / 3, "fetched {} of {}", small.info.fetched, source.0.len());
    assert!(small.raster.stats[0].min > 0.0, "NoData leaked into the stats: {:?}", small.raster.stats[0]);
}

#[test]
fn lzw_strips_with_predictor_and_band_names() {
    let preview = check_pixels("u16_lzw_strips", 0.0);
    assert_eq!(preview.info.layout, Layout::Stripped { rows: 7 });
    assert_eq!(preview.info.band_names, vec!["Red edge", "NIR"]);
    assert!(preview.info.geo.as_ref().unwrap().is_geographic());
}

#[test]
fn palette_packbits() {
    let preview = check_pixels("palette_packbits", 0.0);
    let palette = preview.raster.palette.as_ref().unwrap();
    let want = expected("palette_packbits");
    for (index, color) in want["palette"].as_array().unwrap().iter().enumerate() {
        let color: Vec<u8> = color.as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u8).collect();
        assert_eq!(palette[index].to_vec(), color, "palette entry {index}");
    }
    let image = preview.raster.render(&preview.raster.default_style());
    let index = preview.raster.bands[0][0] as usize;
    assert_eq!(&image.pixels[..3], &palette[index]);
}

#[test]
fn big_endian_bigtiff_planar_zstd() {
    let preview = check_pixels("planar_zstd_bigtiff", 0.0);
    assert!(preview.info.planar_separate);
    assert_eq!(preview.info.geo.as_ref().unwrap().epsg, Some(32633));
}

#[test]
fn rgba_alpha_becomes_transparent() {
    let preview = check_pixels("rgba_deflate", 0.0);
    assert_eq!(preview.raster.alpha, Some(3));
    let image = preview.raster.render(&preview.raster.default_style());
    assert_eq!(image.pixels[3], 0, "left columns are transparent");
    assert_eq!(image.pixels[(preview.raster.width - 1) * 4 + 3], 255);
}

#[test]
fn sampling_respects_budget() {
    let source = fixture("i16_no_overviews");
    check_pixels("i16_no_overviews", 0.0);
    let options = Options { edge: 128, budget: 2_000, threads: 3 };
    let preview = load(&source, &options, &never).unwrap();
    let (used, total) = preview.info.sampled.expect("should be sampled");
    assert!(used < total, "{used} of {total}");
    assert!(preview.raster.bands[0].iter().all(|v| v.is_finite()), "every output pixel gets a value");
    // Values stay within the range of the data.
    let stats = preview.raster.stats[0];
    assert!(stats.min >= -500.0 - 150.0 && stats.max <= 1500.0, "{stats:?}");
}

#[test]
fn cancel_stops_loading() {
    let result = load(&fixture("cog_rgb_jpeg"), &full_resolution(), &|| true);
    assert_eq!(result.unwrap_err(), TiffError::Cancelled);
}

#[test]
fn rejects_non_tiff() {
    assert!(load(&MemorySource(b"not a tiff at all".to_vec()), &Options::default(), &never).is_err());
    assert!(load(&MemorySource(Vec::new()), &Options::default(), &never).is_err());
}

#[test]
fn truncated_file_reports_error() {
    let mut bytes = fixture("u16_lzw_strips").0;
    bytes.truncate(bytes.len() / 3);
    // Must not panic; either an error or a partial image is fine.
    let _ = load(&MemorySource(bytes), &full_resolution(), &never);
}

#[test]
fn refuses_absurd_tile_sizes() {
    // Classic TIFF, one IFD: 100000×100000, one strip of all rows, uncompressed.
    let mut tiff = b"II*\0\x08\0\0\0".to_vec();
    let entries: [(u16, u16, u32, u32); 6] = [
        (256, 4, 1, 100_000),
        (257, 4, 1, 100_000),
        (258, 3, 1, 8),
        (273, 4, 1, 200),
        (278, 4, 1, 100_000),
        (279, 4, 1, 10),
    ];
    tiff.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for (tag, kind, count, value) in entries {
        tiff.extend_from_slice(&tag.to_le_bytes());
        tiff.extend_from_slice(&kind.to_le_bytes());
        tiff.extend_from_slice(&count.to_le_bytes());
        tiff.extend_from_slice(&value.to_le_bytes());
    }
    tiff.extend_from_slice(&0u32.to_le_bytes());
    tiff.resize(260, 0);
    let error = load(&MemorySource(tiff), &Options::default(), &never).unwrap_err();
    assert!(matches!(error, TiffError::Failed(ref text) if text.contains("too large")), "{error:?}");
}
