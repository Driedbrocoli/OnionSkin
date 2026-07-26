# Onionskin

Add words to a page that is already printed.

You have a printed sheet. You want to add something to it — a signature line, an
approval date, a paragraph in a gap — without reprinting the whole page.
Onionskin works out which ink is new and writes a **delta PDF**: the same page
size, blank except for the additions. Put the sheet back in the tray, print the
delta at 100%, and the new words land in the gaps.

The name is the point: the delta is a transparent sheet laid over what is
already there.

It runs entirely on your own machine, never uses the network, and works on
Linux, Windows and macOS.

## Install

### Download it

The [Releases page](https://github.com/driedbrocoli/onionskin/releases) has one
file per machine. Take yours:

| | |
|---|---|
| **Windows** | `onionskin-VERSION-windows.zip` |
| **Linux** — Debian, Ubuntu, Mint | `onionskin_VERSION_amd64.deb`, then `sudo dpkg -i onionskin_*.deb` |
| **Linux** — anything else | `onionskin-VERSION-linux.tar.gz` |
| **macOS** — Apple silicon | `onionskin-VERSION-macos-arm64.tar.gz` |
| **macOS** — Intel | `onionskin-VERSION-macos-x64.tar.gz` |

Unpack it, open a terminal in that folder, and run one line:

```bash
./onionskin install          # Windows: onionskin.exe install
```

#### Or from the terminal, on Linux and macOS

```bash
curl -fsSL https://raw.githubusercontent.com/Driedbrocoli/OnionSkin/main/install.sh | sh
```

That works out which machine this is, fetches the right archive, and runs the
same `onionskin install`. No `sudo` anywhere.

Piping a script from the internet into a shell is a reasonable thing to refuse.
These are the two commands it runs, if you would rather type them yourself:

```bash
curl -fL -O https://github.com/Driedbrocoli/OnionSkin/releases/latest/download/onionskin-linux-x64.tar.gz
tar -xzf onionskin-linux-x64.tar.gz && ./onionskin install
```

Swap `linux-x64` for `macos-arm64` or `macos-x64` on a Mac. Those names have no
version in them on purpose, so the URL keeps working after the next release.

On Debian, Ubuntu or Mint, the `.deb` is the tidier route:

```bash
curl -fL -O https://github.com/Driedbrocoli/OnionSkin/releases/latest/download/onionskin-linux-x64.deb
sudo apt install ./onionskin-linux-x64.deb
```

**`sudo apt install onionskin` does not work**, and will not for a long while:
that means being in Debian's own archive, which takes a sponsor and months. The
`.deb` above is the same package, installed from a file instead of a mirror.

That copies both programs and the PDF renderer into your own account, puts
`onionskin` on your path, and — on Linux and Windows — adds a menu entry that
opens the window. **Nothing asks for an administrator password.** Then open the
applications menu and look for Onionskin, or type `onionskin-desktop`.

The archives hold everything: both programs, the renderer, and the licences.
Nothing else has to be installed.

#### If your computer says it will not run it

Onionskin is not signed with a paid certificate, and both Windows and macOS
say so the first time — in wording that sounds like the file is damaged rather
than merely unsigned. It is not damaged. Nothing was downloaded twice and
nothing needs repairing.

- **Windows** shows a blue box, *"Windows protected your PC"*. Click
  **More info**, then **Run anyway**. The Run button is hidden until you do.
- **macOS** says it *"cannot verify the developer"*. Right-click the program in
  Finder and choose **Open**, or run `xattr -d com.apple.quarantine onionskin`.

The one-line `curl` install above avoids both, because those checks apply to
files a browser downloaded.

> **If the Releases page is empty**, no version has been tagged yet — build it
> from source below. Anyone with write access to the repository makes the
> downloads appear by pushing a tag (`git tag v0.1.0 && git push origin v0.1.0`),
> which builds all five archives and attaches them to a release.

### Or build it from source

It takes about five minutes, most of it waiting. The steps below are the ones
that were actually run on a clean clone, in this order, and they work.

#### 1. Rust

If `cargo --version` says nothing, get it from [rustup.rs](https://rustup.rs) —
one command on every platform, and it installs into your own account.

#### 2. Build it

```bash
git clone https://github.com/driedbrocoli/onionskin
cd onionskin
cargo build --release        # about two minutes
```

The program is now `target/release/onionskin`. It already works:

```bash
./target/release/onionskin doctor
```

#### 3. The PDF renderer

`doctor` will say `PDF rendering MISSING`, and tell you what to do about it.
Onionskin draws PDF pages with **pdfium**, Google's renderer from Chromium. It
is not on crates.io because it is a C++ library, so it is fetched once:

```bash
# Linux — on macOS use pdfium-mac-arm64.tgz, on Windows pdfium-win-x64.tgz
curl -L -o pdfium.tgz \
  https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-linux-x64.tgz
tar -xzf pdfium.tgz lib/libpdfium.so
cp lib/libpdfium.so target/release/          # beside the binary is where it looks
```

It is BSD and Apache licensed, like Onionskin, so there is nothing to agree to.
**Everything works without it except comparing two documents** — writing on a
scan, typing onto a form, drawing, reading letters and printing all need
nothing installed.

#### 4. Install it

```bash
cd target/release
./onionskin install          # Windows: onionskin.exe install
```

That copies both programs and the renderer into `~/.local/bin` (or
`%LOCALAPPDATA%\Onionskin`), adds an applications-menu entry that opens the
window, and puts that folder on your path. **Nothing asks for an administrator
password**: it installs into your own account, and a program that demands a
password to put a file on your own computer teaches people to give passwords to
programs.

Open a new terminal, and:

```bash
onionskin doctor             # what works on this machine, and what is missing
onionskin --help             # everything it can do
onionskin-desktop            # the window
```

`onionskin uninstall` removes exactly what was put there and says what it
removed.

### The window

There are two programs. `onionskin` is the command line; `onionskin-desktop` is
the window, and it is what the applications menu opens. **That is the app** —
you download it, install it, and it is in your menu like anything else.

It is a real window. egui draws every widget itself onto an OpenGL surface:
**no browser, no web view, no localhost, no tab.** That is what keeps the whole
thing a 5 MB download that runs on a machine with nothing installed, and it is
why the program can promise it never touches the network and have you be able
to check.

On **Linux** the window needs the display and graphics libraries every desktop
already has: X11 or Wayland, xkbcommon, and OpenGL. A desktop machine has them.
A server, a container or a minimal virtual machine may not, and then the window
quits with a line about a file nobody has heard of. `onionskin doctor` checks
for them and prints the one command that installs them for your distribution.
Nothing on the command line needs any of it.

On **Windows** and **macOS** there is nothing to install: both ship their own
graphics and keyboard handling.

**Drag a file onto the window** and it goes into the first box that can use it —
drop two documents on the comparing screen and they fill both, in order. The
window opens on the screen you were last using, and the file browser opens in
the folder you last chose something from.

`onionskin uninstall` removes exactly what was put there and says what it
removed. It leaves your calibration profiles alone, and says so.

### Making a release

You should not have to. Pushing a version tag does all of it:

```bash
git tag v0.1.1 && git push origin v0.1.1
```

That runs `.github/workflows/release.yml`, which builds on four machines —
Linux, Windows, and macOS on both chips — fetches the PDF renderer for each,
packages, and attaches ten files to a GitHub release: five archives with the
version in the name, and five with a name that never changes, for URLs that
have to keep working. The same workflow can be run from the Actions tab with a
version typed in, which is how to try it without committing to a number.

`.github/workflows/ci.yml` runs the tests on every push, with the renderer and
LibreOffice installed, because a release nobody tested is a download nobody
should trust.

Neither uses an action from the marketplace. Both are `cargo` and `gh`, which
the runners already have — an action is somebody else's code running with a
token that can write to this repository.

To build the archives on your own machine instead:

```bash
cargo run --release -- package --out dist/
```

Each is about 11 MB and holds both programs, the renderer, both licence files
and a README saying to run `onionskin install`. Built from the same input twice
it produces the same bytes, so a download can be checked against a hash
somebody else published. It refuses to put a Linux binary in a Windows archive,
which is the mistake that produces a download that looks completely normal and
cannot run.

### Word documents, with or without LibreOffice

Nothing else has to be installed. The readers are Rust, like everything else
here — a zip reader, an XML scanner and a page-layout engine, and no new
dependency for any of them. Onionskin opens these by itself:

| | |
|---|---|
| `.pdf`, and any image | directly |
| `.docx`, `.docm`, `.dotx` | Word, in the format it has written since 2007 |
| `.odt`, `.ott`, `.fodt` | OpenDocument text |
| `.txt`, `.md` | plain text |

It reads the words, the headings, the lists, the tables, the alignment, the
indents, the bold and italic and colour, and the paper size — and it says what
it left out. It is not a word processor: it does not lay out images, footnotes,
columns, or headers and footers, and its lines will not break exactly where
Word breaks them.

**LibreOffice** ([download](https://www.libreoffice.org/download/)) is used
whenever it is installed, because it lays a document out the way the program
that wrote it would. It is also the only way to open the older and stranger
formats — `.doc`, `.rtf`, `.xlsx`, `.ods`, `.pptx`, `.odp`, `.vsdx` and the
rest of the list in `render::CONVERTIBLE`.

**Which one matters when.** If you are adding words to a sheet you printed from
Onionskin, either is exact. If the sheet in your tray came out of Word, Word's
line breaks are on it — so use LibreOffice, or export the document to PDF from
Word and use that. Onionskin says which opener it used in the checks it prints
before anything reaches a printer. `ONIONSKIN_OFFICE=onionskin` forces the
built-in reader, which is how to see what somebody without LibreOffice gets.

LibreOffice is deliberately not bundled: it is under the MPL, which would put
obligations on anyone passing the archive on. Onionskin looks for it wherever it
installs — package manager, Snap, Flatpak, `/Applications`, `Program Files` —
and `ONIONSKIN_SOFFICE` points at it if it is somewhere else.

## Four ways to work

### Make a document, print it, and keep adding to it

Start from a blank sheet, and print only what you add afterwards. This is the
one place where the delta is *exact*: the document records precisely which words
went on the paper, so what is new is what is not in that record.

```bash
onionskin new order.onionskin --page a4
onionskin write order.onionskin --at '25,35:PURCHASE ORDER 4471' --size 16 --font bold
onionskin write order.onionskin --at '25,90:Two hundred widgets, black.' --width 90
onionskin show order.onionskin                       # numbered, so you can edit it
onionskin edit order.onionskin 2 --by '0,-2'         # nudge it up 2 mm
onionskin print order.onionskin -o order.pdf --printed

# later — only the approval goes onto the sheet
onionskin write order.onionskin --at '25,150:Approved — J. Bezzina, 26 July'
onionskin print order.onionskin -o delta.pdf --delta
```

### Draw on it

Lines, boxes, circles and paths, anywhere on the page, in any colour. Drawings
go into the delta on the same terms as words: what is new since the sheet was
printed is what gets printed.

```bash
onionskin draw form.onionskin --line '20,100:190,100'          # a rule across
onionskin draw form.onionskin --box '20,40:80x30' --radius 3   # a rounded box
onionskin draw form.onionskin --box '20,40:80x30' --fill lightgrey --no-outline
onionskin draw form.onionskin --circle '105,150:20' --colour red --dash '2,1.5'
onionskin draw form.onionskin --path '20,20 60,50 100,20' --colour '#0033aa'
onionskin draw form.onionskin --path '20,150 60,150 60,175' --close --fill '#ffdd44'
```

Colours are `#rrggbb`, the `#rgb` shorthand, or a name — `black`, `white`,
`grey`, `lightgrey`, `red`, `green`, `blue`, `yellow`, `orange`. Anything that
is a shade of grey is written to the PDF on the greyscale operator rather than
as three equal numbers, so a mono printer is never asked for colour ink it has
not got. Words are always drawn on top of shapes, so a label over a shaded box
stays readable.

### Type onto a document or a form

One file in, no editing round trip. Say where the words go and Onionskin puts
them there:

```bash
onionskin add po.docx -o delta.pdf --at-mm '45,63:J. Bezzina — approved 25 July'
```

Because the text is placed at an absolute position, **nothing on the page can
move** — the reflow problem below simply cannot happen.

### Fill in a sheet you only have as a scan

```bash
onionskin fetch -o scan.png --scanner http://printer.local/eSCL   # from the printer
onionskin inspect scan.png --page a4     # how does it sit on the glass?
onionskin read scan.png --page a4        # where is every letter on it?
onionskin add scan.png -o delta.pdf --at-mm '60,150:Approved' --preview proof.png
onionskin send delta.pdf --printer ipp://printer.local/ipp/print  # back to the paper
```

That is the whole loop on one machine: scan the sheet from the printer, work
out what to add, print it back onto the same paper.

### Turn a scan into something you can edit

A scan read against the font it was set in comes back as a Word document, an
OpenDocument text, or an Onionskin document — each line pinned at the millimetre
it was found, so what opens looks like the paper rather than a column of
disconnected phrases.

```bash
onionskin read scan.png --font-file /path/to/the/font.ttf --to invoice.docx
onionskin read scan.png --font-file font.ttf --to invoice.odt
onionskin read scan.png --font-file font.ttf --to invoice.onionskin
onionskin read scan.png --font-file font.ttf --to notes.docx --flow   # paragraphs
```

A 100-word A4 page takes about a second at 300 dpi — 0.2 s to find the sheet and
its skew, 0.7 s to read every letter. Higher resolution is slower and no more
accurate; 300 dpi is the one to use. Reading is 98–100% right on ordinary prose
at 8–18 pt when the font is the one the page was set in. Two honest limits: a
capital `I` and a lowercase `l` are the same rectangle in most sans-serif faces
and differ by about a pixel, so which you get is a coin toss; and reading a page
against a *different* typeface from the one it was printed in is markedly worse.

For a scanner plugged into this computer rather than one on the network, there
is `onionskin scanners` and `onionskin acquire` instead, which go through SANE.

Onionskin finds the paper's outline in the scan, measures how far it is turned,
and works back to millimetres on the physical sheet. Across 360 combinations of
page size, resolution, skew, margin and position, a point picked on the scan
lands within **0.30 mm** of where it belongs on the paper.

### Compare two documents

Edit in Word as you normally would, and let Onionskin work out what is new:

```bash
onionskin delta report.docx report-edited.docx        # writes report-edited-delta.pdf
onionskin compare report.docx report-edited.docx      # report, write nothing
onionskin delta a.docx b.docx -o delta.pdf --preview ./proof
onionskin delta a.docx b.docx --open                  # and open it when it is done
```

`-o` is optional: without it the result goes beside the edited copy, named after
it. Same for `add` and `print`.

Either way, put the printed sheet back in the tray and print the delta **at
100% / "Actual size"**, with "Fit to page" turned off.

**You never have to touch a browser.** For anyone who would rather use one
anyway — over SSH, or on a machine with no desktop at all — `onionskin serve`
puts the two-document workflow on `http://127.0.0.1:8737/`, reachable only from
that machine. It is an extra command that has to be typed to happen: nothing
opens a browser by itself, ever. Anything automated
that posts to it with `Accept: application/pdf` gets the file straight back
instead, as it always did.

## The thing that will bite you: reflow

*(Only when comparing two documents. The other three ways cannot cause it.)*

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

Calibrate once per printer, per tray. Calibrate on the paper you actually print
on: a shift carries over to any sheet size, but rotation and scale are applied
about the centre of the page, so an A4 profile used on Legal leaves some error
behind. Onionskin says so when it spots the mismatch.

## Writing in any language

The fonts built into every PDF reader cover Western European text and nothing
else. Ask for Chinese, Arabic, Cyrillic, Greek, Hebrew or an emoji and Onionskin
refuses rather than printing a row of black boxes onto your sheet. Point it at a
font that has the characters and it carries that font inside the delta, so the
printer needs nothing installed:

```bash
onionskin add form.pdf -o delta.pdf --at-mm '30,80:承認済み 2026年7月25日' \
  --font-file /usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc
```

Any font file works — `.ttf`, `.ttc`, `.otf`, `.otc`. The two outline formats
live differently inside a PDF and swapping them gives a file that opens fine and
prints a blank page, so Onionskin looks at what the font actually carries rather
than at its extension. That matters for Word in particular: its default faces,
Calibri and Cambria, are PostScript-flavoured.

## Reading a scanned page

`onionskin read` finds every mark of ink on a scan, groups it into letters,
words and lines, and reports each in millimetres from the corner of the paper.
That alone answers the question a delta has to answer before it prints: is this
gap really a gap.

Given the font the page was set in, it also reads them — the alphabet is
whatever that font can draw, so the language is whatever the page is in. See
[ARCHITECTURE.md](ARCHITECTURE.md) for how homoglyphs and right-to-left scripts
are handled, and for the two honest limits (cursive scripts and combining
marks).

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

## Talking to the printer

Onionskin speaks two printer protocols itself, in Rust, with nothing underneath
them but a socket — no CUPS libraries, no scanning software, nothing to install:

* **IPP** to print. `onionskin send` puts the delta on the paper directly, and
  sends `print-scaling: none` with it. That is the point of not going through a
  print dialogue: every dialogue in the world defaults to fitting the page,
  which scales by a percent or two and puts every word in the wrong place.
* **eSCL** to scan — the protocol behind AirScan, which every multifunction
  printer made in the last decade speaks. `onionskin fetch` asks for the whole
  platen with no auto-crop, no auto-deskew and no auto-rotate, because those
  throw away the paper's outline that the page is measured from.

```bash
onionskin printers                                   # what CUPS knows about
onionskin printers --server ipp://printer.local/     # or ask a printer directly
onionskin fetch -o scan.png --scanner http://printer.local/eSCL --capabilities
onionskin send delta.pdf --printer ipp://printer.local/ipp/print --copies 2
```

## Privacy and networking

Onionskin never phones home. There is no telemetry, no update check, and no
external asset in the browser UI — the page it serves contains nothing fetched
from anywhere else. Verified by tracing a full delta run: not one internet
socket is opened.

The one time it opens a socket is when **you name a printer**. Then it talks to
that printer, on your own network, and to nothing else. There is no discovery
beacon and no directory service; if you never name a printer, nothing leaves the
machine.

`onionskin serve` binds to `127.0.0.1`, so the UI is reachable only from your
own machine. It has no password: if you override `--host`, anyone who can reach
that address can upload documents and read every delta, and Onionskin warns you
when you do. Working files are written with owner-only permissions so that other
accounts on a shared machine cannot read documents you have processed.

Onionskin also refuses to write a delta over one of the documents it was made
from, since that would destroy the sheet you were about to print onto.

## Development

```bash
cargo test
cargo clippy --all-targets
```

Tests that need LibreOffice, a scanner or a particular font skip themselves when
it is absent. [ARCHITECTURE.md](ARCHITECTURE.md) explains how the pieces fit
together and why several of them are the shape they are.

## Licence

MIT. The dependencies are deliberately permissive too — PyMuPDF would be the
obvious choice for the PDF work, but it is AGPL and would follow anyone shipping
this.
