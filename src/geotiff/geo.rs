//! GeoTIFF georeferencing: GeoKeys, the pixel-to-map transform and a few
//! inverse projections to show WGS84 coordinates.

use std::f64::consts::PI;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GeoInfo {
    /// GDAL-style affine transform: x = t0 + col*t1 + row*t2, y = t3 + col*t4 + row*t5.
    pub transform: Option<[f64; 6]>,
    pub epsg: Option<u32>,
    /// 1 projected, 2 geographic, 3 geocentric.
    pub model_type: Option<u16>,
    pub citation: String,
    pub pixel_is_point: bool,
    pub linear_unit: Option<u16>,
}

pub const KEY_MODEL_TYPE: u16 = 1024;
pub const KEY_RASTER_TYPE: u16 = 1025;
pub const KEY_CITATION: u16 = 1026;
pub const KEY_GEOGRAPHIC_TYPE: u16 = 2048;
pub const KEY_GEOG_CITATION: u16 = 2049;
pub const KEY_PROJECTED_TYPE: u16 = 3072;
pub const KEY_PCS_CITATION: u16 = 3073;
pub const KEY_LINEAR_UNITS: u16 = 3076;

impl GeoInfo {
    pub fn parse(
        keys: Option<&[u64]>,
        ascii: Option<&str>,
        scale: Option<&[f64]>,
        tiepoints: Option<&[f64]>,
        matrix: Option<&[f64]>,
    ) -> Option<GeoInfo> {
        if keys.is_none() && scale.is_none() && matrix.is_none() {
            return None;
        }
        let mut info = GeoInfo::default();
        let mut geographic = None;
        let mut projected = None;
        let mut citations = Vec::new();
        if let Some(keys) = keys.filter(|keys| keys.len() >= 4) {
            let count = keys[3] as usize;
            for entry in keys[4..].as_chunks::<4>().0.iter().take(count) {
                let (id, location, count, value) = (entry[0] as u16, entry[1], entry[2] as usize, entry[3]);
                let ascii_value = || -> Option<String> {
                    let text = ascii?;
                    let start = value as usize;
                    let piece = text.get(start..start + count)?;
                    Some(piece.trim_end_matches(['|', '\0']).trim().to_owned())
                };
                let short = (location == 0).then_some(value);
                match id {
                    KEY_MODEL_TYPE => info.model_type = short.map(|v| v as u16),
                    KEY_RASTER_TYPE => info.pixel_is_point = short == Some(2),
                    KEY_GEOGRAPHIC_TYPE => geographic = short.filter(|&v| v > 0 && v < 32767).map(|v| v as u32),
                    KEY_PROJECTED_TYPE => projected = short.filter(|&v| v > 0 && v < 32767).map(|v| v as u32),
                    KEY_LINEAR_UNITS => info.linear_unit = short.map(|v| v as u16),
                    KEY_CITATION | KEY_GEOG_CITATION | KEY_PCS_CITATION if location == 34737 => {
                        if let Some(text) = ascii_value().filter(|text| !text.is_empty()) {
                            citations.push(text);
                        }
                    }
                    _ => {}
                }
            }
        }
        info.epsg = match info.model_type {
            Some(2) => geographic.or(projected),
            _ => projected.or(geographic),
        };
        info.citation = citations.first().cloned().unwrap_or_default();

        if let Some(m) = matrix.filter(|m| m.len() >= 16) {
            info.transform = Some([m[3], m[0], m[1], m[7], m[4], m[5]]);
        } else if let (Some(scale), Some(tie)) = (scale.filter(|s| s.len() >= 2), tiepoints.filter(|t| t.len() >= 6)) {
            let (i, j, x, y) = (tie[0], tie[1], tie[3], tie[4]);
            let (sx, sy) = (scale[0], scale[1]);
            info.transform = Some([x - i * sx, sx, 0.0, y + j * sy, 0.0, -sy]);
        }
        if info.pixel_is_point
            && let Some(t) = info.transform.as_mut()
        {
            // GDAL convention: shift by half a pixel so the transform refers to pixel corners.
            t[0] -= 0.5 * t[1] + 0.5 * t[2];
            t[3] -= 0.5 * t[4] + 0.5 * t[5];
        }
        Some(info)
    }

    pub fn pixel_to_map(&self, col: f64, row: f64) -> Option<(f64, f64)> {
        let t = self.transform?;
        Some((t[0] + col * t[1] + row * t[2], t[3] + col * t[4] + row * t[5]))
    }

    pub fn is_geographic(&self) -> bool {
        self.model_type == Some(2) || self.epsg.is_some_and(is_geographic_code)
    }

    /// Longitude and latitude in degrees, when the CRS is one we can invert.
    pub fn to_wgs84(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        if self.is_geographic() {
            return Some((x, y)).filter(|(lon, lat)| lon.abs() <= 360.0 && lat.abs() <= 90.0);
        }
        let epsg = self.epsg?;
        let (lon, lat) = inverse(epsg, x, y)?;
        (lon.is_finite() && lat.is_finite() && lat.abs() <= 90.0).then_some((lon, lat))
    }

    pub fn crs_label(&self) -> String {
        let mut label = match self.epsg {
            Some(code) => format!("EPSG:{code}"),
            None if self.model_type == Some(2) => "geographic".to_owned(),
            None => "—".to_owned(),
        };
        let name = self
            .epsg
            .and_then(crs_name)
            .map(str::to_owned)
            .or_else(|| (!self.citation.is_empty()).then(|| self.citation.clone()));
        if let Some(name) = name {
            label.push_str(&format!(" · {name}"));
        }
        label
    }

    pub fn unit_label(&self) -> &'static str {
        if self.is_geographic() {
            "°"
        } else {
            match self.linear_unit {
                Some(9002) => " ft",
                Some(9003) => " US ft",
                _ => " m",
            }
        }
    }
}

fn is_geographic_code(code: u32) -> bool {
    (4000..5000).contains(&code)
}

fn crs_name(code: u32) -> Option<&'static str> {
    Some(match code {
        4326 => "WGS 84",
        4258 => "ETRS89",
        4269 => "NAD83",
        4267 => "NAD27",
        3857 | 900913 => "WGS 84 / Pseudo-Mercator",
        3035 => "ETRS89 / LAEA Europe",
        31466 => "DHDN / Gauss-Krüger zone 2",
        31467 => "DHDN / Gauss-Krüger zone 3",
        31468 => "DHDN / Gauss-Krüger zone 4",
        31469 => "DHDN / Gauss-Krüger zone 5",
        32601..=32660 => "WGS 84 / UTM north",
        32701..=32760 => "WGS 84 / UTM south",
        25828..=25838 => "ETRS89 / UTM",
        26901..=26923 => "NAD83 / UTM",
        _ => return None,
    })
}

struct Ellipsoid {
    a: f64,
    f: f64,
}

const WGS84: Ellipsoid = Ellipsoid { a: 6_378_137.0, f: 1.0 / 298.257_223_563 };
const GRS80: Ellipsoid = Ellipsoid { a: 6_378_137.0, f: 1.0 / 298.257_222_101 };
const BESSEL: Ellipsoid = Ellipsoid { a: 6_377_397.155, f: 1.0 / 299.152_812_8 };

fn inverse(epsg: u32, x: f64, y: f64) -> Option<(f64, f64)> {
    match epsg {
        3857 | 900913 | 3785 => {
            let lon = x / WGS84.a * 180.0 / PI;
            let lat = (2.0 * (y / WGS84.a).exp().atan() - PI / 2.0) * 180.0 / PI;
            Some((lon, lat))
        }
        32601..=32660 => Some(utm(&WGS84, (epsg - 32600) as i32, false, x, y)),
        32701..=32760 => Some(utm(&WGS84, (epsg - 32700) as i32, true, x, y)),
        25828..=25838 => Some(utm(&GRS80, (epsg - 25800) as i32, false, x, y)),
        26901..=26923 => Some(utm(&GRS80, (epsg - 26900) as i32, false, x, y)),
        // Gauss-Krüger without the datum shift; good to a few hundred metres.
        31466..=31469 => {
            let zone = (epsg - 31464) as f64;
            Some(transverse_mercator_inverse(&BESSEL, zone * 3.0, 1.0, zone * 1_000_000.0 + 500_000.0, 0.0, x, y))
        }
        3035 => Some(laea_inverse(&GRS80, 10.0, 52.0, 4_321_000.0, 3_210_000.0, x, y)),
        _ => None,
    }
}

fn utm(ellipsoid: &Ellipsoid, zone: i32, south: bool, x: f64, y: f64) -> (f64, f64) {
    let lon0 = zone as f64 * 6.0 - 183.0;
    let false_northing = if south { 10_000_000.0 } else { 0.0 };
    transverse_mercator_inverse(ellipsoid, lon0, 0.9996, 500_000.0, false_northing, x, y)
}

/// Inverse transverse Mercator (Snyder, USGS PP 1395, eq. 8-12 ff.).
fn transverse_mercator_inverse(
    e: &Ellipsoid,
    lon0: f64,
    k0: f64,
    false_easting: f64,
    false_northing: f64,
    x: f64,
    y: f64,
) -> (f64, f64) {
    let a = e.a;
    let e2 = e.f * (2.0 - e.f);
    let ep2 = e2 / (1.0 - e2);
    let m = (y - false_northing) / k0;
    let mu = m / (a * (1.0 - e2 / 4.0 - 3.0 * e2 * e2 / 64.0 - 5.0 * e2.powi(3) / 256.0));
    let e1 = (1.0 - (1.0 - e2).sqrt()) / (1.0 + (1.0 - e2).sqrt());
    let phi1 = mu
        + (3.0 * e1 / 2.0 - 27.0 * e1.powi(3) / 32.0) * (2.0 * mu).sin()
        + (21.0 * e1 * e1 / 16.0 - 55.0 * e1.powi(4) / 32.0) * (4.0 * mu).sin()
        + (151.0 * e1.powi(3) / 96.0) * (6.0 * mu).sin()
        + (1097.0 * e1.powi(4) / 512.0) * (8.0 * mu).sin();
    let (sin1, cos1, tan1) = (phi1.sin(), phi1.cos(), phi1.tan());
    let c1 = ep2 * cos1 * cos1;
    let t1 = tan1 * tan1;
    let n1 = a / (1.0 - e2 * sin1 * sin1).sqrt();
    let r1 = a * (1.0 - e2) / (1.0 - e2 * sin1 * sin1).powf(1.5);
    let d = (x - false_easting) / (n1 * k0);
    let lat = phi1
        - (n1 * tan1 / r1)
            * (d * d / 2.0 - (5.0 + 3.0 * t1 + 10.0 * c1 - 4.0 * c1 * c1 - 9.0 * ep2) * d.powi(4) / 24.0
                + (61.0 + 90.0 * t1 + 298.0 * c1 + 45.0 * t1 * t1 - 252.0 * ep2 - 3.0 * c1 * c1) * d.powi(6) / 720.0);
    let lon = (d - (1.0 + 2.0 * t1 + c1) * d.powi(3) / 6.0
        + (5.0 - 2.0 * c1 + 28.0 * t1 - 3.0 * c1 * c1 + 8.0 * ep2 + 24.0 * t1 * t1) * d.powi(5) / 120.0)
        / cos1;
    (lon0 + lon.to_degrees(), lat.to_degrees())
}

/// Inverse Lambert azimuthal equal-area, ellipsoidal (Snyder eq. 24-…).
fn laea_inverse(
    e: &Ellipsoid,
    lon0: f64,
    lat0: f64,
    false_easting: f64,
    false_northing: f64,
    x: f64,
    y: f64,
) -> (f64, f64) {
    let a = e.a;
    let e2 = e.f * (2.0 - e.f);
    let ecc = e2.sqrt();
    let q = |phi: f64| {
        let s = phi.sin();
        (1.0 - e2) * (s / (1.0 - e2 * s * s) - (1.0 / (2.0 * ecc)) * ((1.0 - ecc * s) / (1.0 + ecc * s)).ln())
    };
    let phi0 = lat0.to_radians();
    let qp = q(PI / 2.0);
    let q0 = q(phi0);
    let beta1 = (q0 / qp).asin();
    let rq = a * (qp / 2.0).sqrt();
    let m1 = phi0.cos() / (1.0 - e2 * phi0.sin().powi(2)).sqrt();
    let dd = a * m1 / (rq * beta1.cos());
    let (x, y) = (x - false_easting, y - false_northing);
    let rho = ((x / dd).powi(2) + (dd * y).powi(2)).sqrt();
    if rho < 1e-9 {
        return (lon0, lat0);
    }
    let ce = 2.0 * (rho / (2.0 * rq)).asin();
    let qq = qp * (ce.cos() * beta1.sin() + dd * y * ce.sin() * beta1.cos() / rho);
    let beta = (qq / qp).clamp(-1.0, 1.0).asin();
    let lat = beta
        + (e2 / 3.0 + 31.0 * e2 * e2 / 180.0 + 517.0 * e2.powi(3) / 5040.0) * (2.0 * beta).sin()
        + (23.0 * e2 * e2 / 360.0 + 251.0 * e2.powi(3) / 3780.0) * (4.0 * beta).sin()
        + (761.0 * e2.powi(3) / 45360.0) * (6.0 * beta).sin();
    let lon = (x * ce.sin()).atan2(dd * rho * beta1.cos() * ce.cos() - dd * dd * y * beta1.sin() * ce.sin());
    (lon0 + lon.to_degrees(), lat.to_degrees())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: (f64, f64), b: (f64, f64), tolerance: f64) -> bool {
        (a.0 - b.0).abs() < tolerance && (a.1 - b.1).abs() < tolerance
    }

    #[test]
    fn utm_32n_munich() {
        // Munich; reference from gdaltransform: 11.575544 E, 48.137219 N
        let result = inverse(25832, 691_607.0, 5_334_760.0).unwrap();
        assert!(close(result, (11.575_544, 48.137_219), 1e-6), "{result:?}");
    }

    #[test]
    fn utm_south_and_web_mercator() {
        // Equator at the central meridian of zone 33S
        let result = inverse(32733, 500_000.0, 10_000_000.0).unwrap();
        assert!(close(result, (15.0, 0.0), 1e-6), "{result:?}");
        let result = inverse(3857, 1_113_194.9, 6_446_275.8).unwrap();
        assert!(close(result, (10.0, 50.0), 1e-4), "{result:?}");
    }

    #[test]
    fn laea_europe_origin_and_point() {
        let origin = inverse(3035, 4_321_000.0, 3_210_000.0).unwrap();
        assert!(close(origin, (10.0, 52.0), 1e-9));
        // Reference values from gdaltransform.
        let berlin = inverse(3035, 4_552_036.450_15, 3_273_268.273_57).unwrap();
        assert!(close(berlin, (13.405, 52.52), 1e-6), "{berlin:?}");
    }

    #[test]
    fn gauss_krueger_is_close_without_datum_shift() {
        // gdaltransform with the DHDN datum shift: 11.57532 E, 48.13875 N
        let munich = inverse(31468, 4_468_500.0, 5_333_500.0).unwrap();
        assert!(close(munich, (11.57532, 48.13875), 0.003), "{munich:?}");
    }

    #[test]
    fn transform_from_tiepoint_and_scale() {
        let keys = [1, 1, 0, 2, 1024, 0, 1, 1, 3072, 0, 1, 25832];
        let info = GeoInfo::parse(
            Some(&keys),
            None,
            Some(&[10.0, 10.0, 0.0]),
            Some(&[0.0, 0.0, 0.0, 600_000.0, 5_400_000.0, 0.0]),
            None,
        )
        .unwrap();
        assert_eq!(info.epsg, Some(25832));
        assert_eq!(info.pixel_to_map(1.0, 2.0), Some((600_010.0, 5_399_980.0)));
        assert!(info.crs_label().starts_with("EPSG:25832"));
        assert!(!info.is_geographic());
    }

    #[test]
    fn geographic_citation_from_ascii() {
        let keys = [1, 1, 0, 3, 1024, 0, 1, 2, 2048, 0, 1, 4326, 2049, 34737, 7, 0];
        let info = GeoInfo::parse(
            Some(&keys),
            Some("WGS 84|"),
            Some(&[0.5, 0.5]),
            Some(&[0.0, 0.0, 0.0, -10.0, 50.0, 0.0]),
            None,
        )
        .unwrap();
        assert!(info.is_geographic());
        assert_eq!(info.citation, "WGS 84");
        assert_eq!(info.to_wgs84(5.0, 45.0), Some((5.0, 45.0)));
    }
}
