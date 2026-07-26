# Onionskin in Rust

A port of the Python implementation in `../onionskin/`, in progress.

## Adding words to a scanned page

This is the workflow Rust does end to end today. You have a sheet in your hand
and an image of it; point at a spot on the scan and Onionskin writes a delta
that puts those words on the paper.

```bash
onionskin inspect scan.png --page a4
onionskin add scan.png -o delta.pdf --at '527,916:J. Bezzina' --preview proof.png
onionskin add form.jpg -o delta.pdf --page letter --at-mm '60,150:Approved'
```

`--at` takes coordinates read straight off an image viewer, in scan pixels.
`--at-mm` takes millimetres measured on the paper with a ruler. Either way the
delta comes out at the sheet's true size, to be printed onto the sheet itself.

### Why a scan needs registering

A scan knows nothing. The sheet sits a few millimetres off the corner of the
glass, turned by a degree or so, and the image is in pixels rather than
millimetres — so a point picked on the scan is *not* where that ink sits on the
paper. `src/scan.rs` works out the mapping: where the sheet is, how far it is
turned, and how many pixels make a millimetre.

Words follow the paper's edges, not the scan's tilt. The skew is the scanner's
doing and the sheet itself is straight, so copying it would print crooked text
onto a straight page. `--follow-skew` if the printing really is askew.

Across 360 combinations of page size, resolution, skew, margin and target
position, a point picked on the scan lands within **0.30 mm** of where it
belongs on the paper. It is also tested against the states real scans turn up
in — a brightness gradient from a lid left ajar or a photograph taken under a
window, a shadow down one side, sensor noise, and dust on the glass.

### What it refuses

A confident wrong answer puts ink in the wrong place; an error does not. So
Onionskin declines rather than guessing when:

* the sheet is turned **and** runs off the edge of the scan — its outline is
  cut off, so it cannot say how big the paper is;
* no sheet can be found at all;
* the sheet found is the wrong shape for the page size given — the usual causes
  are naming the wrong paper or leaving two sheets on the glass;
* the text uses characters the built-in fonts cannot write, which would
  otherwise print as a row of solid blocks.

## Port status

| module | ported | verified against Python |
|---|---|---|
| `geometry` — units, page sizes, the calibration transform | yes | 582 values, identical to 5e-10 |
| `pdf` — writing the delta | yes | ink measured in place, 0.2 mm |
| `scan` — registering a scanned sheet | yes | new; no Python counterpart |
| `render` — LibreOffice conversion, page frames, rasterising | no | |
| `diff` — added/removed masks, region labelling | no | |
| `delta` — raster and vector writers, calibration, frame conforming | no | |
| `compose` — text placement with wrapping and alignment | no | |
| `safety` — the checks that stop wasted paper | no | |
| `calibrate` — target, profiles, solving | no | |
| `pipeline`, `web` | no | |

The Python version stays authoritative for the two-document and typed-page
workflows until every row says yes.

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
