# S3 Viewer

[![CI](https://github.com/Farbdrucker/s3-viewer-rs/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/Farbdrucker/s3-viewer-rs/actions/workflows/ci.yml)
[![Tests](https://github.com/Farbdrucker/s3-viewer-rs/actions/workflows/tests.yml/badge.svg?branch=main)](https://github.com/Farbdrucker/s3-viewer-rs/actions/workflows/tests.yml)
[![Builds](https://github.com/Farbdrucker/s3-viewer-rs/actions/workflows/release.yml/badge.svg?branch=main)](https://github.com/Farbdrucker/s3-viewer-rs/actions/workflows/release.yml)
[![Latest release](https://img.shields.io/github/v/release/Farbdrucker/s3-viewer-rs?label=release)](https://github.com/Farbdrucker/s3-viewer-rs/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/Farbdrucker/s3-viewer-rs/total)](https://github.com/Farbdrucker/s3-viewer-rs/releases)
[![Rust 1.95+](https://img.shields.io/badge/rust-1.95%2B-orange?logo=rust)](https://www.rust-lang.org)
[![Platforms](https://img.shields.io/badge/platforms-Linux%20%7C%20Windows%20%7C%20macOS-blue)](#download)
[![License: MIT](https://img.shields.io/github/license/Farbdrucker/s3-viewer-rs)](LICENSE)

Desktop program for browsing buckets of OpenStack and other S3-compatible object stores,
written in pure Rust. Runs on Linux, Windows and macOS, on x86_64 and ARM64.

It replaces the earlier Python/Qt version and keeps all of its features. Both versions
share the same configuration folder (see [Files](#files)), so saved connections work in
either one.

## Download

Builds of the latest release:

| | x86_64 (Intel/AMD) | ARM64 |
| --- | --- | --- |
| **Linux** | [s3-viewer-linux-x86_64.tar.gz](https://github.com/Farbdrucker/s3-viewer-rs/releases/latest/download/s3-viewer-linux-x86_64.tar.gz) | [s3-viewer-linux-arm64.tar.gz](https://github.com/Farbdrucker/s3-viewer-rs/releases/latest/download/s3-viewer-linux-arm64.tar.gz) |
| **Windows** | [s3-viewer-windows-x86_64.zip](https://github.com/Farbdrucker/s3-viewer-rs/releases/latest/download/s3-viewer-windows-x86_64.zip) | [s3-viewer-windows-arm64.zip](https://github.com/Farbdrucker/s3-viewer-rs/releases/latest/download/s3-viewer-windows-arm64.zip) |
| **macOS** | [s3-viewer-macos-x86_64.zip](https://github.com/Farbdrucker/s3-viewer-rs/releases/latest/download/s3-viewer-macos-x86_64.zip) (Intel) | [s3-viewer-macos-arm64.zip](https://github.com/Farbdrucker/s3-viewer-rs/releases/latest/download/s3-viewer-macos-arm64.zip) (Apple Silicon) |

Older versions are on the [releases page](https://github.com/Farbdrucker/s3-viewer-rs/releases). Unpack and start `s3-viewer`
(`s3-viewer.exe`, or `S3 Viewer.app` on macOS: the app is not notarised, so open it the
first time with right-click → Open).

## Features

- **Connection**: endpoint, region, access/secret key, optional bucket (for credentials
  that cannot list buckets), signature `s3v4` or `s3 (SigV2)` for older OpenStack/Ceph,
  path-style addressing, TLS verification (switchable for self-signed certificates; the
  operating system's certificate store is used). Named connections can be saved, loaded
  and removed. The card folds up to a one-line summary.
- **Browsing**: bucket list with filter, folder table with path links, `..` row, name
  filter as plain text or, with the `.*` switch, as a case-insensitive regular expression
  (e.g. `\.tiff?$`, `^tile_\d+_\d+\.png$`, `2024|2025`),
  sorting by name/size/date (folders first), multi-selection with Ctrl/Shift, keyboard use
  (arrows, Enter, Backspace, F5, Ctrl+F, Ctrl+A). A click selects and previews, a
  double-click opens a folder. New folders can be created. Listings are capped at 5000 entries;
  endpoints without ListObjectsV2 fall back to ListObjects.
- **Preview** (docked left or right, or in its own window):
  - text with line numbers; JSON and XML pretty-printed; JSON/NDJSON/GeoJSON, YAML,
    XML/HTML/SVG highlighted; first 1 MB of large files
  - CSV/TSV as a table (delimiter detected, header detected, first 2000 rows)
  - PNG, JPEG, GIF, WebP, BMP images (up to 32 MB) with zoom
  - **(Geo)TIFF**, any size, see below
  - edit text files and write them back to the bucket, copy text, save the file
- **Transfers**: download selected files or whole folders (structure kept, existing local
  files are not overwritten), upload files or folders into the open folder with a question
  before replacing existing objects. Delete selected files and folders (folders with all
  their contents, after a question). Progress bar with byte or object counts; transfers
  and deletions can be cancelled. Large uploads use multipart upload.
- **Styles**: light, dark and example styles built in; your own styles as TOML files,
  reloaded while you edit them.
- German and English interface (button in the connection card).
- Optional notice when a newer release is available on GitHub (see [Update check](#update-check)).

## GeoTIFF preview

Selecting a `.tif`/`.tiff` reads only what is needed, using HTTP range requests:

1. the TIFF/BigTIFF header and image directories,
2. the **smallest overview** that still covers the preview size (1024 px),
3. only the tiles of that level, merged into few requests and decoded in parallel.

A Cloud-Optimised GeoTIFF of many gigabytes therefore previews in about a second. Files
without overviews are read completely when the level fits into a byte budget (64 MB);
larger ones are sampled from a subset of their tiles or strips, marked as coarse, with a
button to load the level completely.

Supported: TIFF and BigTIFF, little and big endian, tiles and strips, chunky and planar
layout; no compression, LZW, Deflate, ZSTD, PackBits and JPEG (incl. YCbCr);
horizontal and floating-point predictors; 1–64 bit integers, 16/32/64 bit floats;
palettes, alpha bands, `GDAL_NODATA` (shown transparent), band names from GDAL metadata.
Not supported: LERC, WebP, JPEG 2000 and CCITT compressions.

Next to the image:

- the value(s) under the mouse, pixel position, map coordinates and WGS84 latitude/longitude,
- RGB band selection or a single band with colour map (gray, viridis, magma, turbo,
  terrain), stretch none / 2–98 % / min–max,
- details: size, bands, data type, compression, tiling, overviews, CRS, pixel size,
  origin, extent, NoData, bytes fetched, and a link that opens the image centre in
  OpenStreetMap.

Latitude/longitude are computed for geographic CRS, Web Mercator (EPSG:3857), UTM
(WGS84 326xx/327xx, ETRS89 258xx, NAD83 269xx), LAEA Europe (EPSG:3035) and German
Gauss-Krüger zones (approximate, without datum shift). Other systems show map coordinates.

Two settings in `settings.json` control the preview: `geotiff_edge` (preview size in
pixels, default 1024) and `geotiff_budget_mb` (bytes of tile data per preview, default 64).

## Files

Everything lives in the usual configuration folder of the system, the same one the Python
version uses:

| System | Folder |
| --- | --- |
| Linux | `~/.config/s3-viewer` (or `$XDG_CONFIG_HOME/s3-viewer`) |
| macOS | `~/Library/Application Support/s3-viewer` |
| Windows | `%AppData%\s3-viewer` |

`S3VIEWER_HOME` overrides the folder. On first start, files from the old `~/.s3-viewer`
folder (connections, settings, styles) are copied over; the old folder is left as it is.
Settings of the Python version (`settings.ini`) are taken over when no `settings.json`
exists yet.

| File | Content |
| --- | --- |
| `connections.json` | saved connections including secret keys, readable only by you (0600) |
| `settings.json` | last form values (never the secret key), language, style, panel layout, preview settings |
| `themes/*.toml` | your styles |

Window size and position are stored by the GUI toolkit in the platform's data directory.

## Styles

Open the style menu (🎨 in the connection card):

- pick a style,
- **Save the current style as a template** writes the complete style to
  `themes/my-<name>.toml` in the configuration folder and selects it,
- **Open the styles folder**.

Edit and save the file; the window updates within a second. Mistakes (unknown keys,
invalid colours) are shown in the error banner. A style can start from another one and
change only a few values:

```toml
name = "Company"
extends = "dark"          # light, dark, nord, high-contrast or one of your files

[colors]
accent = "#e11d48"
section = "#fda4af"

[shape]
radius = 12.0
font_size = 15.0

[fonts]
proportional = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"
```

[`themes/light.toml`](themes/light.toml) lists and explains every key. A file in the
`themes` folder with the name of a built-in style (for example `dark.toml`)
replaces it and may `extends = "dark"` to inherit from the original.

## Update check

At start, the viewer asks GitHub's public release API at most once a day whether a newer
version exists. If so, a "Version x.y.z available" button appears in the connection card
and opens the download page; nothing is downloaded or installed automatically. The check
runs in the background with short timeouts, honours `HTTPS_PROXY`, and fails silently, so
the program works the same without access to GitHub. The ⚙ menu shows the installed
version and the last result, and has **Check now**.

To turn it off:

- untick **Check for a new version at start** in the ⚙ menu, or set
  `"update_check": false` in `settings.json`;
- for all users of a machine, set the environment variable `S3VIEWER_UPDATE_CHECK=0`.
  The app then never contacts GitHub and the setting cannot be switched on.

## Connection notes

- **Endpoint**: the S3 API address, e.g. `https://s3.cloud.example.com`, not the Keystone URL.
- **Keys**: the S3/EC2 credentials from OpenStack, not the login password.
- **Region**: on OpenStack often the region name, e.g. `RegionOne`.
- **Path style**: leave on for OpenStack.
- **Signature**: try `s3 (SigV2)` when sign-in fails on older installations.
- **Proxy**: `HTTPS_PROXY`/`HTTP_PROXY`/`NO_PROXY` are honoured.

## Building

Requires Rust 1.95 or newer (`rustup` from <https://rustup.rs>).

```sh
cargo build --release
# binary: target/release/s3-viewer   (s3-viewer.exe on Windows)
```

No system libraries are needed to build on any platform. At runtime Linux needs X11 or
Wayland and Vulkan or OpenGL drivers (standard on desktops); file dialogs use the XDG
desktop portal, or `zenity` when no portal is running. The program renders with wgpu
(Vulkan/Metal/DirectX 12) and falls back to OpenGL automatically; `S3VIEWER_RENDERER=glow`
forces OpenGL.

## Versions and releases

The version in `Cargo.toml` follows [semantic versioning](https://semver.org): every
merged fix raises the patch number (2.0.0 → 2.0.1), every new feature the minor number
(2.0.1 → 2.1.0), incompatible changes the major number.

GitHub Actions (`.github/workflows/`):

- **CI** checks formatting and runs clippy on every push and pull request.
- **Tests** runs the unit tests and the S3 integration tests against SeaweedFS.
- **Builds** runs on every merge to `main`: tests, then release builds for Linux, Windows
  and macOS on x86_64 and ARM64. If `v<version>` is not tagged yet, it creates that tag
  and a GitHub release with the archives, which the download links above point to.
  Without a version bump the builds are only kept as workflow artifacts.

## Tests

```sh
cargo test
```

runs the unit tests, including the GeoTIFF reader against files written by GDAL
(`tests/data`, regenerate with `tools/make_fixtures.py` in the GDAL Docker image).

The S3 client is also tested against a real endpoint when these variables are set, for
example with SeaweedFS:

```sh
docker run -d --name s3 -p 8333:8333 -v "$PWD/tools/seaweedfs-s3.json:/etc/s3.json" \
    chrislusf/seaweedfs server -s3 -s3.config=/etc/s3.json -s3.port=8333
echo "s3.bucket.create -name viewer-test" | docker exec -i s3 weed shell
S3VIEWER_TEST_ENDPOINT=http://localhost:8333 S3VIEWER_TEST_ACCESS=testkey \
S3VIEWER_TEST_SECRET=testsecret123 S3VIEWER_TEST_BUCKET=viewer-test cargo test
```

`cargo run --example seed -- <endpoint> <access> <secret> <bucket> <folder>` uploads a
local folder, handy for trying the viewer against a test endpoint.

## Layout of the sources

| Path | Content |
| --- | --- |
| `src/s3/` | S3 client: SigV4/SigV2 signing, listing, ranged reads, streamed downloads, multipart uploads |
| `src/geotiff/` | TIFF/BigTIFF reader over range requests, decoders, georeferencing, rendering |
| `src/preview/` | format detection, JSON/XML/CSV preparation, syntax highlighting |
| `src/app/` | the window: connection card, browser, preview panel, dialogs, background tasks |
| `src/theme.rs`, `themes/` | style files and how they map onto the interface |
| `src/profiles.rs`, `src/settings.rs` | saved connections and settings |
| `src/i18n.rs` | German and English texts |
