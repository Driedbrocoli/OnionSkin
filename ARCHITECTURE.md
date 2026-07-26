# How Onionskin is built

Notes on the parts that are not obvious, and on why several of them are the
shape they are. For what the program *does*, see [README.md](README.md).

Everything is Rust. There was a Python implementation; it was ported module by
module and removed, and the differential harness that checked the two agreed
went with it.

## The modules

| | |
|---|---|
| `geometry` | Page sizes, and the similarity transform calibration fits |
| `render`   | LibreOffice for Word documents, pdfium for pixels, page frames |
| `diff`     | What ink is new, what ink is gone |
| `delta`    | Writing the delta PDF, raster or vector |
| `safety`   | The checks that run before paper is committed |
| `calibrate`| The two-pass target, the fit, and the stored profiles |
| `pipeline` | The whole job, end to end |
| `scan`     | Finding the sheet in a scan and measuring how it sits |
| `letters`  | Reading the ink off a registered scan |
| `document` | A document made from nothing and edited |
| `font`     | Embedding a font so the printer needs nothing installed |
| `pdf`      | Writing text into a PDF |
| `acquire`  | Driving a scanner through SANE |
| `web`      | A local HTTP server with no dependency and no external asset |

## Making a document, and adding to it after it is printed

This is Onionskin's own idea applied to its own documents, and it is the one
place where the delta is *exact*. Start from a blank sheet:

```bash
onionskin new order.onionskin --page a4
onionskin write order.onionskin --at '25,35:PURCHASE ORDER 4471' --size 16 --font bold
onionskin write order.onionskin --at '25,50:Wickham & Sons Ltd\n14 Mill Lane\nBristol'
onionskin write order.onionskin --at '25,90:Two hundred widgets, 40 mm, black.' --width 90
onionskin show order.onionskin
```

Coordinates are millimetres from the top-left of the paper, the way you would
measure with a ruler; the second number is the baseline, where the letters sit.
`show` lists everything with a number, and those numbers are what you edit by:

```bash
onionskin edit order.onionskin 3 --by '0,-2'          # nudge it up 2 mm
onionskin edit order.onionskin 3 --text 'Two hundred widgets, black anodised.'
onionskin erase order.onionskin 2
```

Print it, and say that you have:

```bash
onionskin print order.onionskin -o order.pdf --printed
```

Now add the approval and print **only that**, onto the same sheet:

```bash
onionskin write order.onionskin --at '25,150:Approved — J. Bezzina, 26 July'
onionskin print order.onionskin -o delta.pdf --delta
```

No rendering, no diffing, no comparing of pixels: the document recorded exactly
which words went on the paper, so the delta is the words that did not. Nothing
can drift, because nothing is measured.

It also cannot reflow — every piece of text sits at a millimetre you chose, so
inserting one does not push another down the page. What it *can* do is catch you
moving something that is already printed:

```
BLOCKER [page 1]: item 1 has been moved, and it is already on the sheet.
    "PURCHASE ORDER 4471"
    Toner does not come off paper, so an overlay cannot undo it. Print this
    page fresh.
```

`print --delta` exits `2` when it finds one and writes nothing.

The document is JSON, so it diffs, it goes in version control, and you can edit
it by hand. It is written through a temporary file and renamed into place, so a
failure halfway leaves the old one intact rather than an empty file where your
work was.

## Adding words to a scanned page

This is the workflow Rust does end to end today. You have a sheet in your hand
and an image of it; point at a spot on the scan and Onionskin writes a delta
that puts those words on the paper.

```bash
onionskin scanners                       # what this machine can see
onionskin acquire -o scan.png            # scan the sheet
onionskin inspect scan.png --page a4     # how does it sit?
onionskin add scan.png -o delta.pdf --at '527,916:J. Bezzina' --preview proof.png
```

Or work from an image you already have:

```bash
onionskin add form.jpg -o delta.pdf --page letter --at-mm '60,150:Approved'
```

`--at` takes coordinates read straight off an image viewer, in scan pixels.
`--at-mm` takes millimetres measured on the paper with a ruler. Either way the
delta comes out at the sheet's true size, to be printed onto the sheet itself.

### Scanning it here

`onionskin acquire` drives the scanner through SANE, and that is worth more
than the step it saves. The settings a scanning program turns on by default are
the ones that ruin this: **auto-crop** throws away the backing around the sheet,
and with it the outline the page is measured from; **auto-deskew** straightens
the image, which sounds helpful and quietly rewrites the geometry the delta has
to match; **auto-rotate** can turn the page a quarter turn without saying so.
Asking for the scan ourselves means asking for a plain one.

It also checks the scan before you take the sheet off the glass, since it is
quicker to lay it down again than to work around a bad scan later.

Where SANE is not available — Windows, or a machine without it — every command
says so and points at the path that still works: scan with whatever software you
like and pass the file.

### Writing in other alphabets

The fonts built into every PDF reader cover Western European text and nothing
else. Onionskin will not let a reader substitute for the rest, because that
prints a row of solid blocks onto a sheet that may be the only copy. Supply a
font instead and it is carried inside the delta, so the printer needs nothing
installed:

```bash
onionskin add scan.png -o delta.pdf --font-file /path/to/NotoSans.ttf \
  --at-mm '40,100:承認済み 2026年7月25日'
```

**Any font file works** — `.ttf`, `.ttc`, `.otf`, `.otc`. The two outline
formats are held differently inside a PDF, and swapping them gives a file that
opens fine and prints a blank page, so Onionskin looks at what the font actually
carries rather than at its extension:

| outlines | written as | typical fonts |
|---|---|---|
| TrueType (`glyf`) | `CIDFontType2` + `FontFile2` | Arial, Times New Roman, Noto, DejaVu |
| PostScript (`CFF`) | `CIDFontType0` + `FontFile3` `/OpenType` | Calibri, Cambria, most `.otf` |

That second row is the one that matters for Word: its default faces are
PostScript-flavoured. A font carrying neither — a colour-emoji bitmap font, say
— is refused by name rather than embedded as an empty page.

The whole font is embedded rather than subset, so a CJK delta runs to a few
megabytes: a large file that prints correctly beats a small one that does not.

### Reading the letters off the page

Registration says where the *sheet* is. `onionskin read` says where the *words*
are — every mark of ink, grouped into letters, words and lines, reported in the
same millimetres everything else uses:

```bash
onionskin read scan.png --page a4
onionskin read scan.png --font-file /usr/share/fonts/truetype/dejavu/DejaVuSans.ttf
onionskin read scan.png --json
```

Without a font that is all you get, and it is already the useful half: you can
point at a gap and know it really is a gap, rather than finding out at the
printer that your new line landed on top of a footnote.

With a font it reads them. A page is set in *some* font, and comparing ink
against the glyphs of the font it was set in is both far more accurate and far
less code than guessing from scratch. Each letter comes back with how well it
actually matched, and a poor match is reported as unread rather than as a
confident wrong answer — a plausible guess is worse than a blank when the sheet
may be someone's only copy.

**Any language.** The alphabet is not a list written down in English; it is
everything the font can draw. Point it at a Greek font and it looks for Greek, a
Devanagari font and it looks for Devanagari. `--letters` narrows it if you want.

Three problems come with that, and all three are handled:

* **Homoglyphs.** Latin `o`, Greek `ο` and Lao `໐` are three characters drawn as
  one circle, and no amount of looking at ink will separate them. So the page is
  read twice: once with no opinion, then again knowing which script it turned
  out to be in. Most letters in any script have no lookalike, so the first pass
  settles it comfortably.
* **Second alphabets.** Unicode holds several complete extra copies of Latin —
  small capitals, subscripts, four mathematical variants, roman numerals — all
  drawn as ordinary letters and none of them used in ordinary text. They are
  left out of the default alphabet, or `Paid` comes back as `ᴘaid`. Ask for them
  with `--letters` and you get them.
* **Right-to-left.** A line of Hebrew or Arabic found left-to-right comes back
  with its words reversed, which is invisible to anyone who does not read the
  script. The characters know which way they go, so the line is put in reading
  order from what it says.

Two honest limits, both about scripts rather than size. **Cursive** scripts —
Arabic, and handwriting — join their letters, so a joined run is one mark and
comes back as one unread letter; finding *where* the ink is still works, which
is what placing a delta needs. **Combining marks** are folded into the letter
they sit on, so Devanagari and Thai read as their base letters.

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
| `font` — embedding a font to write any alphabet | yes | new; no Python counterpart |
| `acquire` — driving the scanner | yes | new; no Python counterpart |
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
