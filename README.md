# Onionskin

Add words to a page that is already printed.

You have a printed sheet. You want to add something to it — a signature line, an
approval date, a paragraph in a gap — without reprinting the whole page. Give
Onionskin the original document and an edited copy. It works out which ink is
new and writes a **delta PDF**: the same page size, blank except for the
additions. Put the sheet back in the tray, print the delta at 100%, and the new
words land in the gaps.

The name is the point: the delta is a transparent sheet laid over what is
already there.

## Install

```bash
pip install -e ".[web,dev]"
```

Word documents are rendered by **LibreOffice**, which must be installed
([download](https://www.libreoffice.org/download/)). PDFs work without it.
Check the setup with `onionskin doctor`.

## Two ways to work

**Type on the page.** One file in, no editing round trip. Say where the words
go and Onionskin puts them there:

```bash
onionskin add po.docx -o delta.pdf --text '1:45,63:J. Bezzina — approved 25 July'
```

Coordinates are millimetres from the top-left of the sheet, the way you would
measure with a ruler. In the browser (`onionskin serve` → *Type on the page*)
you drop the file in, click where you want the words, and drag to nudge.

Because the text is placed at an absolute position, **nothing on the page can
move** — the reflow problem below simply cannot happen. This is the path to
reach for when you are filling a gap on a form.

**Compare two documents.** Edit in Word as you normally would, and let
Onionskin work out what is new:

```bash
onionskin delta report.docx report-edited.docx -o delta.pdf
onionskin inspect report.docx report-edited.docx      # analyse, write nothing
onionskin delta a.docx b.docx -o delta.pdf --preview ./proof
```

Either way, put the printed sheet back in the tray and print `delta.pdf` **at
100% / "Actual size"**, with "Fit to page" turned off.

## Placing text precisely

```bash
onionskin add form.pdf -o delta.pdf \
  --text '1:45,63:Approved' \
  --text '2:20,240:Continued overleaf' \
  --size 11 --font Times-Roman --width 80 --align right \
  --save-layout layout.json
```

`--width` wraps the text at that many millimetres; without it, text stays on
one line. `onionskin fonts` lists the built-in fonts.

**Writing in any language.** The fonts built into every PDF reader only cover
Western European characters. Ask for Chinese, Arabic, Cyrillic, Greek, Hebrew or
an emoji and Onionskin will refuse rather than print a row of black boxes onto
your sheet. Point it at a font that has the characters and it embeds it:

```bash
onionskin add form.pdf -o delta.pdf --text '1:30,80:承認済み 2026年7月25日' \
  --font-file /usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc
```

`--save-layout` writes the placements out as JSON, and `--layout` reads them
back — so a monthly form can be filled with one command. Layout files number
pages from 1, and take every setting a text box has:

```json
{"boxes": [
  {"page": 1, "x_mm": 45, "y_mm": 63, "text": "Approved", "size_pt": 11,
   "font": "Helvetica", "align": "left", "colour": "#000000", "rotation_deg": 0}
]}
```

## The thing that will bite you: reflow

*(Only when comparing two documents — typing on the page cannot cause it.)*

Insert a word in the middle of a paragraph and every line after it shifts down.
The delta is then not just your new word — it is the whole re-flowed remainder
of the document, at positions that no longer match the sheet in your hand.

Onionskin detects this and refuses. The signal is ink that *disappeared* from
where it used to be: toner does not come off paper, so if anything moved, an
overlay cannot fix it.

```
BLOCKER [page 1]: Existing content moved or was deleted on this page.
    41 mm² of ink is gone from where it was, starting 96 mm down the page.
    The printed sheet no longer matches the document, so an overlay cannot
    fix it — print this page fresh.
```

**To add text without disturbing the layout**, put it in a Word text box set to
*Fixed position on page* with no text wrapping. Floating frames do not push
other content around, so the delta stays small and everything still lines up.

`onionskin delta` exits `2` when it finds a blocker, so it is safe to put in a
script. `--force` overrides.

## Calibration — the part that makes it accurate

Uncalibrated, a second pass through a sheet-fed printer lands within about
**±2 mm**. That is fine for a signature and useless for filling a pre-printed
box. Calibration gets it under **±0.5 mm**, and needs no scanner.

```bash
onionskin calibrate target -o target.pdf                 # A4 by default
onionskin calibrate target -o target.pdf --page legal    # or a5, a3, tabloid…
onionskin calibrate target -o target.pdf --page 100x150  # or any size in mm
```

1. Print `target.pdf` on **blank** paper at 100%.
2. Put that same sheet back in the tray and print the same file **again**.
3. Each crosshair now has two impressions. Read how far the second landed from
   the first — right is `+x`, down is `+y` — against the ruler printed beside
   it.
4. Feed those readings back:

```bash
onionskin calibrate solve --name office \
  --point 'P1:+0.40,-0.15' --point 'P2:+0.35,-0.20' \
  --point 'P3:+0.45,-0.10' --point 'P4:+0.40,-0.15'
```

That fits shift, rotation and scale — the full space of error a paper path can
introduce — and stores it in `~/.onionskin/profiles/`. Every later run applies
the inverse:

```bash
onionskin delta a.docx b.docx -o delta.pdf --profile office
```

Calibrate once per printer, per tray. Profiles are per-printer, not per-job.

Calibrate on the paper you actually print on. A shift carries over to any sheet
size — the paper path pushes every page the same way — but rotation and scale
are applied about the centre of the page, so using an A4 profile on Legal leaves
some error behind. Onionskin says so when it spots the mismatch.

## Raster or vector

| | what it prints | when |
|---|---|---|
| `--mode raster` *(default)* | exactly the pixels that are new | always safe — it cannot re-print ink that is already on the sheet |
| `--mode vector` | the edited PDF clipped to the changed regions | crisper text, but a clip box is a rectangle: a new word hard against an existing one will re-print a sliver of its neighbour, very slightly offset |

Raster recovers anti-aliasing as an alpha channel, so glyph edges stay smooth
rather than printing inside a pale halo.

## Pages that are not the simple case

A page is not always a box starting at (0,0) the right way up. Media boxes have
non-zero origins, crop boxes shrink the visible area, and `/Rotate` turns a page
a quarter turn — all three are ordinary in scans and anything that has been
through a PDF editor, and all three move where ink lands on paper.

Onionskin renders and diffs the page as you see it, then writes the delta back
into the source's own frame, copying its boxes and rotation exactly. A printer
places both impressions identically, so they line up. Without that the delta
would print somewhere other than where the preview showed it.

## Other checks

* **Dead border** — most printers cannot place ink within ~5 mm of an edge.
  Additions that stray in there get a warning (`--margin` to tune).
* **Page count and size** — a page that vanished, or a document that switched
  from A4 to Letter, blocks.
* **Coverage** — a delta covering a large fraction of the page usually means
  something reflowed in a way the ink test did not catch.
* **Empty delta** — the two documents render identically.

## Library

```python
from onionskin import compose, pipeline

# Type on the page
pipeline.compose_run(
    "po.docx",
    [compose.TextBox(page=0, x_mm=45, y_mm=63, text="Approved", size_pt=11)],
    "delta.pdf",
    pipeline.Options(profile="office"),
)

# Or compare two documents
result = pipeline.run(
    "report.docx", "report-edited.docx", "delta.pdf",
    pipeline.Options(dpi=600, mode="vector", profile="office"),
)

if result.blocked:
    for check in result.checks:
        print(check.format())
else:
    for page in result.pages:
        for region in page.added_regions:
            print(f"page {page.index + 1}: {region.width_mm:.1f}mm at {region.x0_mm:.1f}mm")
```

`result.to_dict()` is JSON-serialisable; the CLI's `--json` returns exactly it.

## How it works

1. Both documents are rendered to PDF by the same LibreOffice build, then to
   pixels at the same DPI. Using one engine for both matters — two different
   renderers disagree about kerning, and every glyph would read as changed.
2. `added = ink(new) AND NOT dilate(ink(old))`, and the reverse for `removed`.
   The dilation absorbs sub-pixel jitter so an unmoved glyph does not leave a
   hairline ghost in the delta.
3. Added pixels are grouped into regions on a coarse grid — cheap connectivity —
   with bounding boxes measured back at full resolution.
4. `removed` is never printed. It is the reflow alarm.
5. The delta is written at the exact page size, then re-placed through the
   calibration transform as a single matrix, leaving the media box untouched.

Memory stays flat regardless of document length: each page is rendered,
diffed, written and released before the next one is touched.

## Privacy and networking

Onionskin never uses the network. There is no telemetry, no update check, and
no external asset in the browser UI — verified by running the whole app, and the
test suite, with every socket call blocked.

`onionskin serve` binds to `127.0.0.1`, so the UI is reachable only from your
own machine. It has no password: if you override `--host`, anyone who can reach
that address can upload documents and read every delta, and Onionskin warns you
when you do. Working files are written with owner-only permissions so that other
accounts on a shared machine cannot read documents you have processed.

Onionskin also refuses to write a delta over one of the documents it was made
from, since that would destroy the sheet you were about to print onto.

## Development

```bash
python -m pytest
```

Tests that need LibreOffice skip automatically when it is absent.

## Licence

MIT. The dependencies are deliberately permissive too — PyMuPDF would be the
obvious choice for the PDF work, but it is AGPL and would follow anyone
shipping this.
