# Onionskin in Rust

A port of the Python implementation in `../onionskin/`, in progress.

## Status

| module | ported | verified against Python |
|---|---|---|
| `geometry` — units, page sizes, the calibration transform | yes | 582 values, identical to 5e-10 |
| `render` — LibreOffice conversion, page frames, rasterising | no | |
| `diff` — added/removed masks, region labelling | no | |
| `delta` — raster and vector writers, calibration, frame conforming | no | |
| `compose` — text placement | no | |
| `safety` — the checks that stop wasted paper | no | |
| `calibrate` — target, profiles, solving | no | |
| `pipeline`, `cli`, `web` | no | |

The Python version stays authoritative and shippable until every row says yes.

## How this port is kept honest

A rewrite earns trust by agreeing with the implementation it replaces. Its own
unit tests cannot do that on their own — they can encode the same
misunderstanding twice, and the subtle parts here (a clockwise page rotation
becoming a counter-clockwise PDF rotation, the y-axis flip, the inverse of a
similarity) are exactly where that happens.

So each module gets an `examples/dump_*.rs` that prints its results as JSON, and
`tools/diff_check.py` recomputes the same values in Python and compares:

```bash
cargo run --quiet --example dump_geometry | python3 tools/diff_check.py geometry
```

Any new mismatch fails the check.

## Dependencies

`pdfium-render` binds the same pdfium engine the Python version uses through
pypdfium2, so rasters match rather than merely resembling each other — which is
what makes a pixel diff of two documents comparable across the two
implementations. `lopdf` replaces pikepdf for reading page boxes and rewriting
content streams. Everything stays permissively licensed.

## Building

```bash
cargo test
cargo build --release
```

pdfium is loaded at runtime. If it is not on the system library path, point
`PDFIUM_DYNAMIC_LIB_PATH` at a directory containing `libpdfium.so`
(`.dylib` / `.dll` on macOS and Windows) — the copy shipped inside pypdfium2
works.
