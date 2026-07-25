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

## Use

```bash
# The common case
onionskin delta report.docx report-edited.docx -o delta.pdf

# See what changed without producing anything printable
onionskin inspect report.docx report-edited.docx

# Proof images showing where the new ink lands on the existing page
onionskin delta a.docx b.docx -o delta.pdf --preview ./proof

# In the browser: drag both files in, see the proof, download the delta
onionskin serve
```

Then put the printed sheet back in the tray and print `delta.pdf` **at 100% /
"Actual size"**, with "Fit to page" turned off.

## The thing that will bite you: reflow

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
onionskin calibrate target -o target.pdf
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

## Raster or vector

| | what it prints | when |
|---|---|---|
| `--mode raster` *(default)* | exactly the pixels that are new | always safe — it cannot re-print ink that is already on the sheet |
| `--mode vector` | the edited PDF clipped to the changed regions | crisper text, but a clip box is a rectangle: a new word hard against an existing one will re-print a sliver of its neighbour, very slightly offset |

Raster recovers anti-aliasing as an alpha channel, so glyph edges stay smooth
rather than printing inside a pale halo.

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
from onionskin import pipeline

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

## Development

```bash
python -m pytest
```

Tests that need LibreOffice skip automatically when it is absent.

## Licence

MIT. The dependencies are deliberately permissive too — PyMuPDF would be the
obvious choice for the PDF work, but it is AGPL and would follow anyone
shipping this.
