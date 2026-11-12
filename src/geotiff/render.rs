//! Turns decoded bands into a displayable image.

use crate::preview::RgbaImage;

/// A downsampled raster. Missing data is NaN.
#[derive(Clone, Debug)]
pub struct Raster {
    pub width: usize,
    pub height: usize,
    /// Size of the full-resolution image, for mapping preview pixels back.
    pub full_width: u64,
    pub full_height: u64,
    pub bands: Vec<Vec<f32>>,
    pub stats: Vec<BandStats>,
    pub alpha: Option<usize>,
    pub alpha_max: f32,
    pub palette: Option<Vec<[u8; 3]>>,
    pub min_is_white: bool,
    pub is_u8: bool,
    /// Colour image (RGB or YCbCr photometric).
    pub rgb_like: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BandStats {
    pub min: f32,
    pub max: f32,
    pub p2: f32,
    pub p98: f32,
    pub valid: usize,
}

impl BandStats {
    pub fn compute(values: &[f32]) -> BandStats {
        let step = (values.len() / 200_000).max(1);
        let mut sample: Vec<f32> = values.iter().step_by(step).copied().filter(|v| v.is_finite()).collect();
        if sample.is_empty() {
            return BandStats::default();
        }
        let valid = values.iter().filter(|v| v.is_finite()).count();
        let pick = |sample: &mut Vec<f32>, fraction: f64| {
            let index = ((sample.len() - 1) as f64 * fraction).round() as usize;
            *sample.select_nth_unstable_by(index, |a, b| a.total_cmp(b)).1
        };
        let min = pick(&mut sample, 0.0);
        let max = pick(&mut sample, 1.0);
        let p2 = pick(&mut sample, 0.02);
        let p98 = pick(&mut sample, 0.98);
        BandStats { min, max, p2, p98, valid }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stretch {
    /// Values as they are (0–255), only for 8-bit data.
    None,
    Percent,
    MinMax,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Colormap {
    Gray,
    Viridis,
    Magma,
    Turbo,
    Terrain,
}

impl Colormap {
    pub const ALL: [Colormap; 5] =
        [Colormap::Gray, Colormap::Viridis, Colormap::Magma, Colormap::Turbo, Colormap::Terrain];

    pub fn label(self) -> &'static str {
        match self {
            Colormap::Gray => "Gray",
            Colormap::Viridis => "Viridis",
            Colormap::Magma => "Magma",
            Colormap::Turbo => "Turbo",
            Colormap::Terrain => "Terrain",
        }
    }

    fn stops(self) -> &'static [(f32, [u8; 3])] {
        match self {
            Colormap::Gray => &[(0.0, [0, 0, 0]), (1.0, [255, 255, 255])],
            Colormap::Viridis => &[
                (0.0, [0x44, 0x01, 0x54]),
                (0.111, [0x48, 0x28, 0x78]),
                (0.222, [0x3e, 0x49, 0x89]),
                (0.333, [0x31, 0x68, 0x8e]),
                (0.444, [0x26, 0x82, 0x8e]),
                (0.555, [0x1f, 0x9e, 0x89]),
                (0.666, [0x35, 0xb7, 0x79]),
                (0.777, [0x6e, 0xce, 0x58]),
                (0.888, [0xb5, 0xde, 0x2b]),
                (1.0, [0xfd, 0xe7, 0x25]),
            ],
            Colormap::Magma => &[
                (0.0, [0x00, 0x00, 0x04]),
                (0.125, [0x1c, 0x10, 0x44]),
                (0.25, [0x4f, 0x12, 0x7b]),
                (0.375, [0x81, 0x25, 0x81]),
                (0.5, [0xb5, 0x36, 0x7a]),
                (0.625, [0xe5, 0x59, 0x64]),
                (0.75, [0xfb, 0x87, 0x61]),
                (0.875, [0xfe, 0xc2, 0x87]),
                (1.0, [0xfc, 0xfd, 0xbf]),
            ],
            Colormap::Turbo => &[
                (0.0, [0x30, 0x12, 0x3b]),
                (0.071, [0x41, 0x45, 0xab]),
                (0.143, [0x46, 0x75, 0xed]),
                (0.214, [0x39, 0xa2, 0xfc]),
                (0.286, [0x1b, 0xcf, 0xd4]),
                (0.357, [0x24, 0xec, 0xa6]),
                (0.429, [0x61, 0xfc, 0x6c]),
                (0.5, [0xa4, 0xfc, 0x3b]),
                (0.571, [0xd1, 0xe8, 0x34]),
                (0.643, [0xf3, 0xc6, 0x3a]),
                (0.714, [0xfe, 0x9b, 0x2d]),
                (0.786, [0xf3, 0x63, 0x15]),
                (0.857, [0xd9, 0x38, 0x06]),
                (0.929, [0xb1, 0x19, 0x01]),
                (1.0, [0x7a, 0x04, 0x03]),
            ],
            Colormap::Terrain => &[
                (0.0, [51, 51, 153]),
                (0.15, [0, 153, 255]),
                (0.25, [0, 204, 102]),
                (0.5, [255, 255, 153]),
                (0.75, [128, 92, 84]),
                (1.0, [255, 255, 255]),
            ],
        }
    }

    pub fn sample(self, t: f32) -> [u8; 3] {
        let t = t.clamp(0.0, 1.0);
        let stops = self.stops();
        let upper = stops.iter().position(|(position, _)| *position >= t).unwrap_or(stops.len() - 1).max(1);
        let (p0, c0) = stops[upper - 1];
        let (p1, c1) = stops[upper];
        let f = if p1 > p0 { (t - p0) / (p1 - p0) } else { 0.0 };
        std::array::from_fn(|i| (c0[i] as f32 + (c1[i] as f32 - c0[i] as f32) * f).round() as u8)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    pub rgb: bool,
    pub bands: [usize; 3],
    pub gray: usize,
    pub stretch: Stretch,
    pub colormap: Colormap,
}

impl Raster {
    /// Bands that carry colour or values, without the alpha band.
    pub fn data_bands(&self) -> usize {
        match self.alpha {
            Some(alpha) if alpha + 1 == self.bands.len() => self.bands.len() - 1,
            _ => self.bands.len(),
        }
    }

    pub fn default_style(&self) -> Style {
        let count = self.data_bands();
        let rgb = self.palette.is_none() && count >= 3;
        Style {
            rgb,
            bands: [0, 1.min(count.saturating_sub(1)), 2.min(count.saturating_sub(1))],
            gray: 0,
            stretch: if self.is_u8 && (rgb || self.rgb_like || count == 1) { Stretch::None } else { Stretch::Percent },
            colormap: Colormap::Gray,
        }
    }

    fn normalize(&self, band: usize, value: f32, stretch: Stretch) -> f32 {
        let stats = self.stats.get(band).copied().unwrap_or_default();
        let (low, high) = match stretch {
            Stretch::None if self.is_u8 => (0.0, 255.0),
            Stretch::Percent => (stats.p2, stats.p98),
            _ => (stats.min, stats.max),
        };
        if high > low { (value - low) / (high - low) } else { 0.0 }
    }

    pub fn render(&self, style: &Style) -> RgbaImage {
        let count = self.width * self.height;
        let mut pixels = vec![0u8; count * 4];
        let band = |index: usize| self.bands.get(index.min(self.bands.len().saturating_sub(1)));
        for (i, out) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let color = if let Some(palette) = &self.palette {
                let value = band(0).map_or(f32::NAN, |b| b[i]);
                value.is_finite().then(|| palette.get(value.max(0.0) as usize).copied().unwrap_or([0, 0, 0]))
            } else if style.rgb {
                let mut rgb = [0u8; 3];
                let mut valid = true;
                for (channel, &index) in style.bands.iter().enumerate() {
                    let value = band(index).map_or(f32::NAN, |b| b[i]);
                    if !value.is_finite() {
                        valid = false;
                        break;
                    }
                    rgb[channel] = (self.normalize(index, value, style.stretch).clamp(0.0, 1.0) * 255.0).round() as u8;
                }
                valid.then_some(rgb)
            } else {
                let value = band(style.gray).map_or(f32::NAN, |b| b[i]);
                value.is_finite().then(|| {
                    let mut t = self.normalize(style.gray, value, style.stretch);
                    if self.min_is_white && style.colormap == Colormap::Gray {
                        t = 1.0 - t;
                    }
                    style.colormap.sample(t)
                })
            };
            let Some(rgb) = color else { continue };
            let alpha = match self.alpha.and_then(|index| self.bands.get(index)) {
                Some(alpha) if alpha[i].is_finite() => (alpha[i] / self.alpha_max * 255.0).clamp(0.0, 255.0) as u8,
                Some(_) => 0,
                None => 255,
            };
            out.copy_from_slice(&[rgb[0], rgb[1], rgb[2], alpha]);
        }
        RgbaImage { width: self.width, height: self.height, pixels }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raster(bands: Vec<Vec<f32>>, is_u8: bool) -> Raster {
        let stats = bands.iter().map(|b| BandStats::compute(b)).collect();
        Raster {
            width: bands[0].len(),
            height: 1,
            full_width: bands[0].len() as u64,
            full_height: 1,
            bands,
            stats,
            alpha: None,
            alpha_max: 255.0,
            palette: None,
            min_is_white: false,
            is_u8,
            rgb_like: false,
        }
    }

    #[test]
    fn stats_percentiles() {
        let values: Vec<f32> = (0..=100).map(|v| v as f32).chain([f32::NAN]).collect();
        let stats = BandStats::compute(&values);
        assert_eq!((stats.min, stats.max, stats.p2, stats.p98, stats.valid), (0.0, 100.0, 2.0, 98.0, 101));
    }

    #[test]
    fn gray_minmax_and_nodata() {
        let r = raster(vec![vec![10.0, 20.0, f32::NAN]], false);
        let style = Style { stretch: Stretch::MinMax, ..r.default_style() };
        let image = r.render(&style);
        assert_eq!(&image.pixels[0..4], &[0, 0, 0, 255]);
        assert_eq!(&image.pixels[4..8], &[255, 255, 255, 255]);
        assert_eq!(image.pixels[11], 0, "NaN is transparent");
    }

    #[test]
    fn rgb_u8_unstretched() {
        let r = raster(vec![vec![255.0], vec![128.0], vec![0.0]], true);
        let style = r.default_style();
        assert!(style.rgb);
        assert_eq!(style.stretch, Stretch::None);
        assert_eq!(&r.render(&style).pixels, &[255, 128, 0, 255]);
    }

    #[test]
    fn colormap_ends() {
        assert_eq!(Colormap::Viridis.sample(0.0), [0x44, 0x01, 0x54]);
        assert_eq!(Colormap::Viridis.sample(1.0), [0xfd, 0xe7, 0x25]);
        assert_eq!(Colormap::Gray.sample(0.5), [128, 128, 128]);
    }
}
