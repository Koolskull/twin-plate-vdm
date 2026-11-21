# Twin plate VDM

Experimental concept. This is a first test of an idea, and it needs more development before it is a format anyone should depend on.

A solid is stored as two PNG images that share one center. One face points up and the other points down. Each pixel is the vector from that center out to the surface. A mesh of any resolution is a resample of those two files.

The shapes in this repo are formulas: a sphere, a box, a torus, a cone, a capsule, a mushroom, and a rippled ball. The page is a small WebAssembly explainer for that idea.

## Look at it

Open [`web/index.html`](web/index.html) in a browser. The built module is already next to the page, so a local server is optional.

## Build

[`build.sh`](build.sh) rebuilds the wasm module. It expects Homebrew Rust and `ld.lld` (`brew install lld`).

```
cargo test --offline -- --test-threads=1
```

The baker uses one global state, so the tests stay on a single thread.

## Still open

- One ray in each direction stores one landing on the outer surface. Thickness, overhangs, and faces that point back toward the plate are not a finished solid.
- The PNGs use stored zlib so the codec has no compression dependency. The files are the model, and they are not a smaller substitute for a coarse mesh.
- There is no import from a modeling tool, and no path yet for painting a texture per console.
