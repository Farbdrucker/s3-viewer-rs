"""Writes the GeoTIFF test fixtures in tests/data with GDAL.

Run inside the GDAL container from the project root:

    docker run --rm -v "$PWD:/work" -w /work ghcr.io/osgeo/gdal:ubuntu-small-latest \
        python3 tools/make_fixtures.py

Besides the files it writes tests/data/expected.json with pixel values that
GDAL reads back, so the Rust reader is checked against GDAL itself.
"""

import json
import os

import numpy as np
from osgeo import gdal, osr

gdal.UseExceptions()
OUT = "tests/data"
os.makedirs(OUT, exist_ok=True)


def srs(epsg):
    ref = osr.SpatialReference()
    ref.ImportFromEPSG(epsg)
    return ref.ExportToWkt()


def mem(width, height, bands, dtype, epsg, origin, pixel):
    ds = gdal.GetDriverByName("MEM").Create("", width, height, bands, dtype)
    ds.SetGeoTransform((origin[0], pixel, 0, origin[1], 0, -pixel))
    if epsg:
        ds.SetProjection(srs(epsg))
    return ds


def rgb_pattern(width, height):
    y, x = np.mgrid[0:height, 0:width]
    r = (x * 255 // max(width - 1, 1)).astype(np.uint8)
    g = (y * 255 // max(height - 1, 1)).astype(np.uint8)
    b = (((x // 32) + (y // 32)) % 2 * 200 + 30).astype(np.uint8)
    return r, g, b


expected = {}
SAMPLES = [(0, 0), (17, 9), (123, 77), (199, 99)]


def record(name, path, points=SAMPLES):
    ds = gdal.Open(path)
    arrays = [ds.GetRasterBand(i + 1).ReadAsArray() for i in range(ds.RasterCount)]
    expected[name] = {
        "width": ds.RasterXSize,
        "height": ds.RasterYSize,
        "bands": ds.RasterCount,
        "overviews": ds.GetRasterBand(1).GetOverviewCount(),
        "geotransform": list(ds.GetGeoTransform()),
        "nodata": ds.GetRasterBand(1).GetNoDataValue(),
        "points": [
            {"x": x, "y": y, "values": [float(a[y, x]) for a in arrays]}
            for (x, y) in points
            if x < ds.RasterXSize and y < ds.RasterYSize
        ],
    }


# 1. Cloud-optimised RGB with JPEG compression and overviews.
r, g, b = rgb_pattern(600, 400)
src = mem(600, 400, 3, gdal.GDT_Byte, 25832, (600000, 5400000), 10)
for i, band in enumerate((r, g, b)):
    src.GetRasterBand(i + 1).WriteArray(band)
gdal.Translate(f"{OUT}/cog_rgb_jpeg.tif", src, format="COG",
               creationOptions=["COMPRESS=JPEG", "QUALITY=95", "BLOCKSIZE=128", "OVERVIEW_RESAMPLING=AVERAGE"])
record("cog_rgb_jpeg", f"{OUT}/cog_rgb_jpeg.tif")

# 2. Float32 elevation model, floating point predictor, NoData hole, COG.
y, x = np.mgrid[0:300, 0:500]
dem = (500 + 300 * np.sin(x / 60.0) * np.cos(y / 45.0) + x * 0.5).astype(np.float32)
dem[100:140, 200:260] = -9999
src = mem(500, 300, 1, gdal.GDT_Float32, 3035, (4321000, 3210000), 25)
src.GetRasterBand(1).WriteArray(dem)
src.GetRasterBand(1).SetNoDataValue(-9999)
gdal.Translate(f"{OUT}/dem_f32_deflate.tif", src, format="COG",
               creationOptions=["COMPRESS=DEFLATE", "PREDICTOR=YES", "BLOCKSIZE=128"])
record("dem_f32_deflate", f"{OUT}/dem_f32_deflate.tif", SAMPLES + [(230, 120)])

# 3. Two-band uint16 in strips, LZW with horizontal predictor, geographic CRS, band names.
y, x = np.mgrid[0:200, 0:300]
src = mem(300, 200, 2, gdal.GDT_UInt16, 4326, (11.0, 48.5), 0.001)
src.GetRasterBand(1).WriteArray((x * 200 + y).astype(np.uint16))
src.GetRasterBand(2).WriteArray((60000 - x * y).astype(np.uint16))
src.GetRasterBand(1).SetDescription("Red edge")
src.GetRasterBand(2).SetDescription("NIR")
gdal.Translate(f"{OUT}/u16_lzw_strips.tif", src, format="GTiff",
               creationOptions=["COMPRESS=LZW", "PREDICTOR=2", "TILED=NO", "BLOCKYSIZE=7"])
record("u16_lzw_strips", f"{OUT}/u16_lzw_strips.tif")

# 4. Palette image with PackBits.
y, x = np.mgrid[0:128, 0:256]
src = mem(256, 128, 1, gdal.GDT_Byte, None, (0, 0), 1)
src.GetRasterBand(1).WriteArray(((x // 32) + (y // 32) * 8).astype(np.uint8))
table = gdal.ColorTable()
for i in range(256):
    table.SetColorEntry(i, (i * 7 % 256, i * 13 % 256, i * 29 % 256, 255))
src.GetRasterBand(1).SetRasterColorTable(table)
src.GetRasterBand(1).SetRasterColorInterpretation(gdal.GCI_PaletteIndex)
gdal.Translate(f"{OUT}/palette_packbits.tif", src, format="GTiff", creationOptions=["COMPRESS=PACKBITS"])
record("palette_packbits", f"{OUT}/palette_packbits.tif")
expected["palette_packbits"]["palette"] = [list(table.GetColorEntry(i)[:3]) for i in range(32)]

# 5. Big-endian BigTIFF, separate planes, ZSTD, tiled.
r, g, b = rgb_pattern(300, 200)
src = mem(300, 200, 3, gdal.GDT_Byte, 32633, (500000, 6000000), 2)
for i, band in enumerate((r, g, b)):
    src.GetRasterBand(i + 1).WriteArray(band)
gdal.Translate(f"{OUT}/planar_zstd_bigtiff.tif", src, format="GTiff",
               creationOptions=["COMPRESS=ZSTD", "INTERLEAVE=BAND", "TILED=YES", "BLOCKXSIZE=64",
                                "BLOCKYSIZE=64", "BIGTIFF=YES", "ENDIANNESS=BIG"])
record("planar_zstd_bigtiff", f"{OUT}/planar_zstd_bigtiff.tif")

# 6. RGBA with alpha, Deflate and horizontal predictor.
r, g, b = rgb_pattern(200, 100)
alpha = np.full((100, 200), 255, np.uint8)
alpha[:, :50] = 0
src = mem(200, 100, 4, gdal.GDT_Byte, 3857, (1113194, 6446275), 30)
for i, band in enumerate((r, g, b, alpha)):
    src.GetRasterBand(i + 1).WriteArray(band)
src.GetRasterBand(4).SetColorInterpretation(gdal.GCI_AlphaBand)
gdal.Translate(f"{OUT}/rgba_deflate.tif", src, format="GTiff",
               creationOptions=["COMPRESS=DEFLATE", "PREDICTOR=2", "ALPHA=YES", "PHOTOMETRIC=RGB"])
record("rgba_deflate", f"{OUT}/rgba_deflate.tif")

# 7. Tiled int16 without overviews, used to test sampling under a byte budget.
y, x = np.mgrid[0:1024, 0:1024]
src = mem(1024, 1024, 1, gdal.GDT_Int16, 25833, (300000, 5800000), 1)
src.GetRasterBand(1).WriteArray(((x // 64) * 100 - (y // 64) * 10 - 500).astype(np.int16))
gdal.Translate(f"{OUT}/i16_no_overviews.tif", src, format="GTiff",
               creationOptions=["COMPRESS=DEFLATE", "TILED=YES", "BLOCKXSIZE=64", "BLOCKYSIZE=64"])
record("i16_no_overviews", f"{OUT}/i16_no_overviews.tif", [(0, 0), (100, 900), (1000, 70)])

with open(f"{OUT}/expected.json", "w") as handle:
    json.dump(expected, handle, indent=1)
for name in sorted(os.listdir(OUT)):
    print(f"{name:28} {os.path.getsize(os.path.join(OUT, name)):>8} bytes")
