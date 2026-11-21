# Twin plate VDM

Experimental concept. This is a first test of an idea, and it needs more development before it is a format anyone should depend on.

A solid is stored as two images that share one center. One face points up and the other points down. Each pixel is the vector from that center out to the surface. A mesh of any resolution is a resample of those two files.

The same plates can be 16-bit RGBA PNG or scanline OpenEXR. In the PNG, 32768 is zero displacement. In the EXR, the float itself is the displacement and zero means zero. Red, green, and blue are X, Y, and Z. Alpha is the mask. A miss is alpha 0.

A sculpt brush is one file, the upward plate. The downward plate is left empty. Horn, crater, and ridge are brushes. The seven formula solids are still pairs.

The page keeps each file under about 1.2 MB, so a plate stops at 256². A full film plate will not load. The reader accepts uncompressed, RLE, ZIPS, and ZIP scanlines, half or float, and extra channels are skipped. Tiled, deep, and multipart EXRs are refused.

## Examples

[`examples/`](examples/) holds generated plates:

- `horn.exr`, `crater.exr`, `ridge.exr` — single brushes
- `sphere-plus.exr`, `sphere-minus.exr` — one solid as a pair
- `blobbies-center.exr` — a 32² crop of an ILM test image, re-read here as XYZ. It is a film plate, not a sculpt brush. See [`examples/NOTICE`](examples/NOTICE).

## Look at it

Open [`web/index.html`](web/index.html) in a browser. The built module is already next to the page, so a local server is optional. Drop one EXR to treat it as a brush, or two files to load a pair.

## Build

[`build.sh`](build.sh) rebuilds the wasm module. It expects Homebrew Rust and `ld.lld` (`brew install lld`).

```
cargo test --offline -- --test-threads=1
```

The baker uses one global state, so the tests stay on a single thread. If `Blobbies.exr` from [openexr-images](https://github.com/AcademySoftwareFoundation/openexr-images) is at `/tmp/Blobbies.exr`, one test checks a known pixel. That file is not committed.

## Still open

- One ray in each direction stores one landing on the outer surface. Thickness and faces that point back toward the plate are not a finished solid.
- The PNGs use stored zlib so that codec has no compression dependency. EXR files written here are uncompressed float, which is larger and easy for other tools to open.
- There is no import from a modeling tool, and no path yet for painting a texture per console.
