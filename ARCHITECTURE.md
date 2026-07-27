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
| `render`   | Which engine opens a document, pdfium for pixels, page frames |
| `diff`     | What ink is new, what ink is gone |
| `delta`    | Writing the delta PDF, raster or vector |
| `safety`   | The checks that run before paper is committed |
| `calibrate`| The two-pass target, the fit, and the stored profiles |
| `pipeline` | The whole job, end to end |
| `scan`     | Finding the sheet in a scan and measuring how it sits |
| `letters`  | Reading the ink off a registered scan |
| `document` | A document made from nothing and edited: words and drawings |
| `font`     | Embedding a font so the printer needs nothing installed |
| `pdf`      | Writing text and shapes into a PDF |
| `office`   | Reading and writing `.docx` and `.odt`, without a word processor |
| `printer`  | Printing over IPP and scanning over eSCL, both spoken directly |
| `acquire`  | Driving a scanner through SANE |
| `web`      | A local HTTP server with no dependency and no external asset |
| `install`  | Putting the program where the operating system can find it |
| `settings` | The few things worth remembering between one run and the next |
| `package`  | Building the archive people download |

And a second binary, `src/desktop/`, which is the window.

## Two programs, one library

`onionskin` is the command line. `onionskin-desktop` is a window. Everything
either of them can do lives in the library; neither has any logic of its own
beyond arranging it.

They are two executables rather than one because **Windows decides at link time
whether a program owns a console**. A window built as a console program flashes
a black box behind itself on every launch; a console program built as a window
prints to nowhere. One file cannot be both.

### Why not a web view

The obvious way to build a desktop application now is to wrap a browser engine.
It is the wrong answer here twice over. It would add something like a hundred
megabytes to a five-megabyte download, and it would put a full network stack
inside a program whose central claim is that nothing of yours leaves the machine — a claim
that would then be impossible to verify by reading the code.

So the window is [egui](https://github.com/emilk/egui), which draws every widget
itself onto an OpenGL surface. No web view, no system toolkit, no C++ library.
The binary links against nothing but libc; X11, xkbcommon and OpenGL are opened
when it starts.

That last detail is worth knowing, because it decides the failure mode. On a
machine without those libraries — a server, a container, a minimal virtual
machine — the window quits with a line about a file nobody has heard of.
`install::desktop_needs` looks for them and `onionskin doctor` prints the one
command that installs them for whichever package manager is actually present.
Nothing on the command line needs any of it.

### Slow work

Making a delta takes seconds and reading a page of letters takes about one.
Either between two frames freezes the window: the title bar greys out and the
operating system offers to kill the program. So every slow thing runs on a
thread of its own and reports back through `desktop::job`, and the window keeps
drawing, says what it is doing, and counts the seconds — which is what tells
somebody it is working rather than stuck.

A hundred-page delta takes minutes, which is long enough that "working" is not
enough to say. `pipeline::run_watched` takes a callback and reports a
`pipeline::Step` — what it is doing and which page of how many — so the window
shows a bar that moves and the command line rewrites one line as it goes.
`pipeline::run` is the same thing with the callback thrown away, because most
callers do not want one. The command line only draws that line when there is a
terminal to draw on: piped into a file, a carriage return every page turns the
output into one unreadable line.

There is exactly one worker, on purpose. pdfium serialises individual calls but
not the *sequence* of calls that makes up one document, so two renders at once
will eventually read one another's state and crash.

### How it is tested

By running it. There is no display in the build environment, so it runs under
Xvfb against Mesa's software rasteriser and is screenshotted. Compiling proves
nothing about a window: the first run found the sidebar text running off the
edge of the panel and losing its last word, which was the word that
distinguished one screen from the next.

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

#### Small ink is not the same thing as dust

A tittle at 11 pt is under half a millimetre and a full stop is smaller still —
both well below the size at which a mark is worth taking seriously as a letter.
Judging them by size alone threw away the dot of every `i` and `j`, the accent of
every é and ü, and the punctuation from every page of body text ever scanned. A
page of prose came back reading `Lıne` and `ȷumps` with no full stops, and
nothing anywhere said so.

What separates dust from a full stop is not size but **company**. A speck on the
glass sits alone; a full stop sits on a line of writing next to letters of a
proper size. So small marks are carried all the way through — offered to a letter
as an accent, offered to another small mark as the other half of a colon — and
only what is still standing alone at the end, on a line with no proper letters on
it, is thrown away as dust.

#### Measuring the type rather than assuming it

To tell an `l` from a dotless `ı` you have to know how big the type is, and the
obvious estimate is wrong in a way that hides. A line of prose is mostly
lowercase, so its tallest quarter are the ascenders of `b d f h k l` — which
stand taller than a capital in nearly every typeface. Read that height as a cap
height and every letter on the page measures about a sixth short; then an `l` is
exactly as tall as a dotless `ı`, and a page of English comes back sprinkled with
Turkish.

So each line is read twice. The first pass guesses the scale from the height of
its tall letters. Every letter it reads then answers the question "how many
millimetres is one cap height on this line?" — the mark's own height divided by
the glyph's — and the middle of those answers is the line's real scale. The
second pass reads it again knowing that.

Lines do not affect one another, so they are read in parallel. Even with the
second pass, a page takes a quarter of the time it used to.

#### What is left

Measured on ordinary prose at 8–18 pt, read against the font it was set in:
**98–100% of characters**, and 100% at several sizes. Two things account for
nearly all of the remainder:

* A capital `I` and a lowercase `l` are one rectangle in most sans-serif faces,
  differing by about three hundredths of an em — a pixel and a half at 11 pt on a
  300 dpi scan. Nothing in the ink can settle it, so when two candidates come
  within 2% of each other the commoner letter wins, on the grounds that in
  running text `l` outnumbers `I` by a hundred to one. That is a better answer,
  not a right one.
* Reading a page against a **different** typeface from the one it was printed in
  is markedly worse. The shape score falls for every letter at once, and some
  lookalike from another alphabet fits the ink better than the true letter about
  as often as not.

### Turning a scan into something editable

`--to` writes the page out as a Word document, an OpenDocument text, or an
Onionskin document:

```bash
onionskin read scan.png --font-file font.ttf --to invoice.docx
onionskin read scan.png --font-file font.ttf --to notes.odt --flow
```

Both formats are a zip of XML, and Onionskin writes them itself — the zip writer
already exists for the packaging, and requiring a word processor in order to
*produce* a document would be a strange thing for a program that only needs one
to read somebody else's.

Each line goes into a frame anchored to the page at the millimetre it was found,
rather than flowing into paragraphs. Flowing throws away everything Onionskin
knows about where the ink was, and a scanned form comes out as a column of
disconnected phrases. `--flow` gives ordinary paragraphs to anyone who wants
them.

Two things had to be discovered by handing the files to LibreOffice rather than
by reading the specifications. `w:framePr`, the obvious tool for a placed
paragraph in a `.docx`, merges a run of framed paragraphs into **one** frame at
the first one's position — a page of twelve placed lines opened showing one. Text
boxes cannot be merged, so those are used instead. And LibreOffice's plain-text
export silently drops everything inside a frame, so a file that opens perfectly
comes back empty: the tests convert to flat ODF instead, and check the positions
as well as the words.

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

## Opening a Word document without a word processor

Onionskin used to need LibreOffice to open a `.docx`. That is a fair thing to
ask of somebody who already has it and an unreasonable thing to ask of everybody
else — three hundred megabytes, on every machine, so that a program can read a
file that is a zip of XML. So it reads them itself.

| | |
|---|---|
| `office::unzip` | The zip reader: central directory, stored and deflated entries, zip64 sizes |
| `office::xml`   | A scanner that hands back tags and text, one at a time |
| `office::read::docx` | `word/document.xml`, `styles.xml`, `numbering.xml` → paragraphs |
| `office::read::odt`  | `content.xml` and `styles.xml`, zipped or flat |
| `office::read::plain`| `.txt`, and Markdown read as text with its headings honoured |
| `office::read::layout` | Paragraphs → lines on paper → the same PDF writer everything else uses |

`office::read::Sheet` is the middle: some paper, some margins, and a list of
blocks — paragraphs with runs of styled text, and tables of cells holding blocks
of their own. Each reader fills one in and the layout module empties it, so
neither knows anything about the other's format.

### What it reads, and what it says it cannot

Text, headings, bold, italic, underline, strikethrough, colour, type size and
font family; alignment including justification; indents, hanging indents,
spacing before and after, and line spacing; bulleted, numbered, lettered and
Roman lists with their counters; tables with column widths, spanned cells and
ruled borders; page and line breaks; the paper size and its margins.

Not images, footnotes, columns, or headers and footers. Every one of those comes
back as a sentence in `Sheet::notes`, which `render::to_pdf_noting` passes to the
pipeline, which turns it into a `safety::Check` at `Severity::Note` — so the
command line, the window, the browser page and the JSON all say the same thing
without any of them knowing where it came from.

The browser page is the one that had to change shape for it. It used to hand
back the delta as a download and nothing else, which silently threw away every
warning the run produced — the margins, the coverage, the missing calibration,
and now which engine opened the document. A browser can be told one thing per
response, so a run with anything to say now answers with a page that says it and
offers the file underneath; the delta waits in memory for the second request and
is handed over once. A run with nothing to say still downloads straight away —
and so does one whose caller said `Accept: application/pdf`, which is how
something automated distinguishes itself from a person: a browser says it will
take anything, and only a script asks for a PDF by name.

### Three traps worth writing down

Word writes some shapes **twice**: as DrawingML inside `mc:Choice`, and again as
the old VML inside `mc:Fallback`, so that older readers see something. A reader
that takes both gets every text box twice. The fallback is skipped whole.

Word also writes text that is not words. `w:instrText` is the machinery of a
field — `PAGE \* MERGEFORMAT` and the like — and `w:delText` is text somebody
deleted with track changes on. Both would otherwise be printed.

And a `draw:frame` in an OpenDocument file is **not necessarily a picture**. It
is also how a text box is written — including by the `.odt` writer at the other
end of this same module, which puts each line of a scanned page in one so that
it opens where it was found on the paper. Skipping frames as images reads those
documents as blank pages, which is how Onionskin briefly could not read its own
output. There is a test that writes a placed document and reads it back.

### Which engine, and why it matters

LibreOffice is used wherever it is installed, because it lays a document out the
way the program that wrote it would. Onionskin's own reader is what a machine
without it gets. `ONIONSKIN_OFFICE=onionskin` forces the built-in one.

For the delta itself the choice is safe either way: both documents go through
the same engine in the same run, so their renderings agree by construction. What
the choice changes is the sheet **already in the tray**. If that came out of
Word, Word's line breaks are on it, and Onionskin's may fall elsewhere — which
is why the opener says which one it was, in the checks printed before anything
reaches a printer.

### Reading is not trusting

A document arriving from somewhere else is not a friendly input. The zip reader
checks every entry against its CRC, refuses an entry that inflates to far more
than it claims (a few kilobytes that fill memory is an old trick), bounds the
walk by the bytes rather than by the count the file gives, and names the
compression methods it will not read rather than reading them wrongly. The XML
scanner resolves no external entities and follows no DTD, so there is nothing to
point at `/etc/passwd` and no entity expansion to run away with.

## Reading the font off the page

`src/typeface.rs`, and the trial in `match_the_page` in `src/main.rs`.

The problem is circular and the way out of it is worth writing down. Reading
letters off a scan means comparing ink to some font's glyph shapes, so what
comes back depends on which font is doing the comparing — and the font is the
thing being looked for. Times text read against a sans face comes back mostly
unread.

That is not a defeat, it is the measurement. The page is read against a serif, a
sans and a monospaced face in turn, and whichever accounts for most of the ink
is the family. Two details make it work in practice. The alphabet is restricted
to the common Latin shapes rather than everything the font can draw — DejaVu
carries some six thousand glyphs, and asking which of six thousand a smudged
`o` is gets a worse answer than asking which of eighty, thirty times slower.
And the reference faces are the metric-compatible clones that ship with nearly
every system, so the comparison is against something that looks like what is on
the paper.

Then the size, from the widths. `pdf::builtin_width_mm` does not estimate the
width of a word in one of the eight built-in faces; it knows it, from the table
Adobe published. A page's worth of words is a page's worth of exact predictions,
so the size falls out of a least-squares fit — **two** parameters, not one,
because a scan measures the *ink* box and the metrics give the *advance*, which
is wider by a side bearing at each end. Fitting only the size absorbs that
bearing into the size and, worse, leaves every face wrong by the same shape of
error, so the comparison stops depending on the page at all.

Courier needed a third measurement. The only monospaced faces on most machines
are sans ones, which look nothing like Courier's slab serifs, so a Courier page
comes back barely read and is then discarded *for* being barely read — having
been perfectly recognisable the whole time by how evenly it was spaced. So the
letter pitch is measured directly, from the gaps between consecutive letters
within words, before the gate that would have thrown it away.

## Finding a printer without being told where it is

`src/discover.rs`. DNS-SD over multicast UDP, written here rather than pulled
in: the DNS message format, including name compression pointers, which
responders rely on heavily. A hostile or broken packet must not hang or panic,
so there is a jump budget on pointers, a cap on name length, and every field is
bounds-checked against what actually arrived.

Queries go out from an ephemeral port, which makes them one-shot queries that a
responder answers unicast — so no share of port 5353 is needed and it works
alongside Avahi and Bonjour. Two rounds: PTR questions first, then direct
SRV/TXT/A questions for anything that answered with a name and nothing behind
it, so a device that omits the additional records is usable rather than merely
visible.

Everything degrades to an empty list. No multicast permission, no network, a
firewall: none of those is an error worth stopping a program over, and none of
them is the user's fault.

This is the one place Onionskin speaks to anything it was not told about by
name, and the promise on the sidebar was rewritten to match rather than left to
drift. A promise that is nearly true is worth less than a smaller one that is
exactly true.

## Where the time goes

Measured before anything was changed, on a twenty-one page comparison at four
hundred dots an inch. Of twenty-three seconds: ten drawing pages in pdfium,
eight comparing them, three and a half converting the documents to PDF, and
under one writing the delta.

pdfium is the largest single cost and cannot be threaded. `src/render.rs` holds
it behind a mutex because the `thread_safe` feature makes each individual call
safe and that is not enough: rendering a page is a *sequence* of calls, pdfium
keeps state across them, and two interleaved sequences segfault inside the
library. That was found the hard way, and the comment above the mutex says so.

So the work went where the work was, and the guiding rule was that the pages
must come out the same. They do: the delta is byte-for-byte identical on every
path tested but one, and that one differs only in the creation timestamp
LibreOffice embeds in its own PDF, which the old build differs from itself on.

Three of the four wins were about doing less rather than doing it cleverer.

**A branch per pixel.** The dilation was already separable — growing by *r*
horizontally then vertically, rather than one (2r+1)² window — but it asked `if
this pixel is set` fifteen million times a page. That question is what stops a
compiler using the vector instructions every machine has had for twenty years.
Written as one slice against another with `|=`, eight or sixteen pixels are
grown per instruction: 120 ms a page became 39. The combine loop had the same
fault, `&&` being a branch where `&` is not.

**Copies that arrived at bytes already in hand.** The crop took the top-left
*w* × *h* of each render, and when the two renders already agreed on their size
— the ordinary case, being the same page at the same resolution — it copied
fifteen megabytes to produce them unchanged. It borrows now.

**Colour nobody read.** Every render built RGB *and* greyscale. The sheet being
compared against is only ever wanted in grey, and when no delta is being built
neither page needs colour: forty-six megabytes a page not allocated, not filled
in and not handed back.

**A delta built and deleted.** `onionskin compare` reports and writes nothing,
and it did that by running the whole pipeline into a temporary folder and
removing the file unread — cropping every changed region, giving it a soft
mask, compressing it, for nothing. `pipeline::examine` stops before that.

The remaining shape is: rendering dominates, and it belongs to pdfium. The next
thing worth trying, when it is worth trying, is overlapping the comparison with
the rendering — the render must stay on one thread, but nothing says the
comparison of page *n* cannot run while page *n+1* is being drawn.

## Placing words by what is already on the page

`src/anchor.rs`. Everything else in Onionskin wants millimetres from the
top-left corner. That is the honest unit and a miserable thing to have to
supply, so the page is read and the new words are put after something already
on it.

The matching is deliberately forgiving and deliberately bounded, and the two
halves of that are the whole design. Forgiving, because a scan is never read
perfectly — `Received:` genuinely comes back as `Peceived:` off a noisy one, and
refusing over that sends somebody back to the ruler this exists to replace. So
case, spacing and punctuation are discarded and up to a quarter of the letters
may be wrong, by a bounded edit distance that abandons a row as soon as every
reachable cell in it is over budget.

Bounded, because the failure mode is a sheet of paper. Exact matches are tried
across the whole page first and only then near ones, so a page carrying both
`Dispatched` and `Dispatcher` resolves to the right one instead of reporting a
tie. Under five letters nothing is forgiven, because `Date` and `Rate` are both
plausible labels on the same form and one wrong letter between them is a coin
toss. An anchor found twice is refused with both positions rather than resolved
by taking the first.

The matcher works on a list of rows rather than on a `PageText`, for the same
reason `typeface::detect_measured` takes widths: a page of letters carries the
straightened bitmap each was matched from and cannot be built by hand, which
would leave the logic tested only through the thing that produces it.

## Measuring a printer instead of asking somebody to

`src/calibrate.rs`. Calibration was always the part that made Onionskin
accurate and always the part nobody did, because it ended in reading eight
offsets off paper with a ruler, in tenths of a millimetre, and typing them into
a command with an awkward syntax. Those numbers are what every later delta is
placed by, so the least reliable step in the whole program was a person
squinting at a printed scale.

The target is two pages now, and that change is what makes the rest possible.
Both passes used to print the same file, so on the sheet the two impressions
were identical crosshairs and nothing could tell which was which — an automatic
measurement would have got the offset's sign ambiguous. Page one prints crosses
and page two prints diamonds, so the vector from cross to diamond is
unambiguous and is exactly the registration error.

Then `measure_from_scan` takes a window around each expected crosshair, finds
the connected components in it, classifies each as cross or diamond by shape,
and takes the difference of their centroids. A window with the wrong number of
marks in it lowers that reading's confidence; a crosshair that cannot be read at
all is left out rather than guessed at, and `solve_from_offsets` was always
happy with a subset. Fewer than three readable ones is refused, because two
points and a similarity is four numbers through four numbers — it passes exactly
through both readings and says nothing about whether either was any good.

On a synthetic sheet with a known 0.76 mm offset, the five crosshairs come back
within 0.05 mm on average. A ruler and a good eye do perhaps ten times worse.

## Publishing an apt repository

`src/apt.rs`. `sudo apt install onionskin` cannot be made to work by hosting a
file: apt installs from an archive with an index and a signature, and being in
Debian's own archive means a sponsor and months of waiting. Hosting the same
thing yourself takes a directory and two lines typed once by whoever wants it,
and that directory is what this builds — `pool/`, `Packages`, `Packages.gz`,
`Release`, with SHA-256 throughout and no MD5 or SHA-1, both of which have been
broken for years and unnecessary since 2016.

SHA-256 is written out here by hand, like the CRC-32, the tar, the zip and the
`ar` archive next door. That is the house style and it is not merely stylistic:
it is one fewer dependency in the trust path of the thing that says these
packages came from you.

Signing is the one part deliberately left to `gpg`. Everything else is Rust down
to the hash, but key custody is not a thing to hand-roll, and the private key
belongs wherever its owner already keeps their keys. What is generated instead
is the exact commands, including both `InRelease` and the detached
`Release.gpg`, and the `signed-by=` form of the sources line, which trusts one
key for one repository — `apt-key add` trusted a key for the whole machine,
including the operating system, and has been deprecated since apt 2.2.

Two details worth writing down because both produce a repository that looks
right and does not work. Packages of architecture `all` must be copied into
*every* `binary-<arch>/Packages`, not only `binary-all`, or apt never sees them
and says there is no such package. And an epoch in a version (`1:0.1.0`) must be
stripped from the pool filename and kept in the `Version:` field, because a
colon is reserved in URLs and illegal in a filename on Windows.

## Getting it onto somebody's machine

The `package` module writes the archives and `install` unpacks itself into a
home directory; between them sits the part that was missing for a long time,
which is anywhere to download from. Two workflows fill it.

| | |
|---|---|
| `.github/workflows/ci.yml` | The tests, on every push, with pdfium and LibreOffice installed |
| `.github/workflows/release.yml` | Four runners, ten files, one GitHub release |

Neither uses an action from the marketplace. Both are `cargo` and `gh`, which
the runners already have. An action is somebody else's code running with a
token that can write to this repository, and that is a lot to accept for a step
that is four lines of shell — the same reasoning that keeps the dependency list
short everywhere else here.

### Why each archive is uploaded twice

GitHub serves the newest release at
`/releases/latest/download/<the exact file name>`. A name with a version in it
therefore cannot be written down: `onionskin-0.1.0-linux.tar.gz` is a URL that
stops working the day there is a 0.1.1, and it is exactly the URL a README, an
`install.sh` or an answer to "how do I install this" would contain.

So every archive goes up under both names. The versioned one is the nicer thing
to find in a Downloads folder six months later; the plain one —
`onionskin-linux-x64.tar.gz` — is what a script can point at forever. It costs
a `cp` and eleven megabytes of storage.

### The one-line install

`install.sh` at the root is what `curl … | sh` runs. It is fifty lines of
POSIX shell: work out the machine from `uname`, refuse politely if there is no
ready-made archive for it and say how to build from source instead, fetch,
unpack into a directory it cleans up on any exit, and hand over to
`onionskin install` — which is the same code path the person who unpacked a
`.tar.gz` by hand runs. There is no second installer to keep in step.

No `sudo`, and none needed: everything goes into the user's own account. A
program that asks for an administrator password to put a file on somebody's own
computer is teaching them to give passwords to programs.

The README also writes out the two commands the script runs. Piping a script
from the internet into a shell is a reasonable thing to refuse, and somebody
who refuses it should not be left without instructions.

### What cannot be done here

`sudo apt install onionskin` needs the package to be in Debian's own archive,
which needs a Debian developer to sponsor it and a good deal of back-and-forth
over packaging policy. The `.deb` this builds installs from a file —
`sudo apt install ./onionskin-linux-x64.deb` — and that is as close as it gets
without that process. A self-hosted apt repository would close the gap, at the
cost of a GPG signing key that has to live somewhere and be looked after.

macOS is unsigned, and says so in the archive's README: Gatekeeper stops a
downloaded program the first time it runs and the message it gives reads like
the file is broken. Saying so up front is cheaper than the alternative, which
is paying Apple for a certificate.

## Where it came from

Onionskin began as Python and was ported module by module. The Python is gone,
and so is the harness that checked the two agreed while both existed — each
module printed its results as JSON and a script recomputed the same numbers the
other way round. It earned its keep: the subtle parts here are exactly where a
rewrite encodes the same misunderstanding twice, and a clockwise page rotation
becoming a counter-clockwise PDF rotation is not the kind of thing a unit test
written by the same hand catches.

The printing half of it is still here — `examples/dump_geometry.rs` and
`examples/dump_pdf.rs` — because dumping a module's numbers is the quickest way
to see what a change did to them, with or without anything to compare against.

What it settled, and what the tests now hold on their own:

| | |
|---|---|
| `geometry` — units, page sizes, the calibration transform | 582 values, identical to 5e-10 |
| `pdf` — writing the delta | ink measured in place, within 0.2 mm |
| `scan`, `font`, `acquire`, `letters`, `office`, `printer` | new here; there was never a Python counterpart |

## Dependencies

`pdfium-render` binds the same pdfium engine the Python version uses through
pypdfium2, so rasters match rather than merely resembling each other — which is
what makes a pixel diff of two documents comparable across the two
implementations. `lopdf` replaces pikepdf for reading page boxes and rewriting
content streams. Everything stays permissively licensed.

The zip and XML readers are written here rather than pulled in, and the reason
is the same one that applies to the writers in `package`: the formats are small,
the alternative is two more dependencies for something a page of code does, and
`flate2` — the one real algorithm involved — is already in the tree because PNG
needs it. What it buys is that the checksum used to verify a document is the
same function used to write one.

## Building

```bash
cargo test
cargo build --release
```

pdfium is loaded at run time, not linked. `Engine::bind` looks beside the
binary, then in the places a package manager puts it, then at whatever the
system library path offers. To point it somewhere else, set `ONIONSKIN_PDFIUM`
to the library **file** — `libpdfium.so`, or `.dylib` / `.dll` on macOS and
Windows. The copy inside pypdfium2 works, and so does the one from
[pdfium-binaries](https://github.com/bblanchon/pdfium-binaries/releases),
which is what the release workflow fetches.

Without it everything works except comparing two documents, and `onionskin
doctor` says so along with the exact download to get.
