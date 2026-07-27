# Onionskin

Add words to a page that is already printed.

You have a printed sheet. You want to add something to it — a signature line, an
approval date, a paragraph in a gap — without reprinting the whole page.
Onionskin works out which ink is new and writes a **delta PDF**: the same page
size, blank except for the additions. Put the sheet back in the tray, print the
delta at 100%, and the new words land in the gaps.

The name is the point: the delta is a transparent sheet laid over what is
already there.

It runs entirely on your own machine, nothing about your documents ever leaves
it, and it works on Linux, Windows and macOS.

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

**`sudo apt install onionskin` needs an archive to install from.** Being in
Debian's own archive takes a sponsor and months, so Onionskin publishes its
own: every release builds one and puts it at
`https://driedbrocoli.github.io/OnionSkin`. Point apt at it once —

```bash
echo 'deb [trusted=yes] https://driedbrocoli.github.io/OnionSkin stable main' \
  | sudo tee /etc/apt/sources.list.d/onionskin.list
sudo apt update && sudo apt install onionskin
```

— and `apt upgrade` keeps it current with everything else on the machine. The
page at that address prints the current lines, including the `signed-by=` form
once the archive is signed.

**You can host the same archive yourself**, and for anything beyond one person's
machines you probably should. Take `onionskin-apt-repo.tar.gz` from the release,
unpack it where a web server can see it, and use your own address in the line
above; there is nothing to run there, no database, and no software to install.
Or build one from scratch: `onionskin apt-repo` writes the whole thing — pool,
`Packages`, `Release`, the hashes — from your `.deb` files, and prints the `gpg`
commands to sign it and the two lines your users type:

```bash
onionskin apt-repo --deb onionskin_0.1.0_amd64.deb --out apt \
  --url https://packages.example.com/apt
```

Signing is left to `gpg` on purpose: everything else is Rust down to the
SHA-256, but key custody is not a thing to hand-roll, and the private key
belongs wherever you already keep your keys. The instructions it prints use
`signed-by=`, which trusts that one key for that one repository — not
`apt-key add`, which trusted a key for the whole machine and has been
deprecated since apt 2.2.

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

Call the document whatever you like. Onionskin knows its own documents by what
is in them, not by what they are named, so `order.pdf` works exactly as well —
`new` will just mention that nothing *else* will open a file by that name, and
tell you the one command that turns it into a real PDF.

**In the window, click the page and type.** The **Make a document** screen draws
the sheet at the size it will print; click a spot on it and a caret appears
there, and what you type goes on the paper at that spot. Shift-Enter starts
another line, Enter or Escape finishes. Double-click something already written
to change it, and drag it to move it — the millimetre positions in the form
below follow, so the two never disagree.

A click you thought better of leaves nothing behind: an empty caret is taken off
the page rather than left as an invisible item in the document and in every
delta after it. The file is written once when the caret leaves, not once per
letter, so Undo goes back a sentence at a time rather than a keystroke.

### Draw on it

Lines, boxes, circles and paths, anywhere on the page, in any colour. Drawings
go into the delta on the same terms as words: what is new since the sheet was
printed is what gets printed.

It works on any document, not only Onionskin's own — a Word file, an
OpenDocument, a PDF, a scan. The source is opened, measured, and left exactly
as it was; what comes out is a delta with the shapes on it.

```bash
onionskin draw statement.pdf --box '20,140:100x14' --colour red -o delta.pdf
onionskin write invoice.docx --at '25,120:Paid — 27 July' --size 13 -o delta.pdf
```

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

#### You do not have to know the millimetres

Nobody holding a form knows the gap after `Received:` starts 44.9 mm across and
40.1 mm down. They know it is the gap after `Received:`. So say that:

```bash
onionskin add form.png --after 'Received:Approved 27 July'
onionskin add form.png --below 'Signature:J. Bezzina'
```

```
Found "Received" on the line: Received:
  putting the words at 44.9, 40.1 mm
```

The page is read, the anchor is found, and the words go where a person would
have put them. It works on a scan, a PDF, a Word file or an OpenDocument — for
a document the page is drawn and then read, which is what a person does when
they look at one.

On Onionskin's own documents the same two flags are on `write`, and there they
are *exact*: the document already knows where each of its words sits, to the
millimetre it will print at, so nothing is drawn and nothing is read.

```bash
onionskin write form.onionskin --after 'Received:27 July 2026'
onionskin write form.onionskin --below 'Signature:J. Bezzina'
```

If an anchor cannot be found, nothing is written at all — not even the `--at`
placements in the same run. Half a page of new words followed by a refusal
would be the worst of both.

Matching is forgiving, because a scan is never read perfectly: case, spacing
and punctuation are ignored, and up to a quarter of the letters may be wrong.
`Received:` really does come back as `Peceived:` off a noisy scan. Exact matches
are tried first and always win.

What it will not do is guess. An anchor that is not there is an error listing
what **is** on the page. An anchor that appears twice is an error saying where
both are and asking for more words — putting the date next to the first of five
`Date:` fields is a coin toss, and a coin toss that ruins a sheet of paper is
worse than a question. Under five letters nothing is forgiven at all, because
`Date` and `Rate` are both plausible labels on the same form.

Because the text is placed at an absolute position, **nothing on the page can
move** — the reflow problem below simply cannot happen.

#### It picks the font off the page

Adding words to a scan with no `--font` and no `--size`, Onionskin works out
what the page is already set in and matches it:

```bash
$ onionskin add scan.png --at-mm '60,150:Approved'
Matched the page: Times-Roman at about 11.9 pt, from 8 words
  Say --font or --size to choose for yourself.
```

Somebody filling a gap on a form wants the words to look like the rest of the
form, and the answer is printed on the sheet in their hand — it is not written
down anywhere in their head. Nobody looks at a rent statement and thinks
"Helvetica, eleven point".

It reads the page against a serif, a sans and a monospaced face in turn, and
whichever accounts for most of the ink says which family it is. Then the widths
of the words it read are fitted against Adobe's published metrics, which gives
the size. A typewriter face is caught a third way, from the letter spacing
alone, because the only monospaced fonts on most machines are sans ones that
look nothing like Courier.

A blank page, a poor scan or a machine with no fonts all mean "use the default"
rather than an error. `--font`, `--size` and `--no-match-font` all still win.

### One sheet each, for everybody on a list

Two hundred certificates. Two hundred names. One command.

```bash
onionskin batch certificate.pdf --from people.csv \
    --after 'Awarded to:{name}' \
    --at '60,140:{course} — no. {number}' \
    -o certificates.pdf
```

```
certificates.pdf: 3 sheets, 6 additions in all.
```

The list is an ordinary spreadsheet saved as CSV. Its first line names the
columns, and `{name}` anywhere in a placement means that person's own. There
is also `{number}`, which counts the sheets for you — invoice numbers, ticket
numbers and asset tags need no column of their own.

Out comes one PDF with one page per person: a stack through the printer once,
not two hundred separate jobs.

**Before you commit two hundred sheets of paper:**

```bash
onionskin batch certificate.pdf --from people.csv --after 'Awarded to:{name}' --first 2
```

`--first 2` makes two, so you can hold a real one against a real sheet.

A column name that does not exist is caught **before anything is written**:

```
error: people.csv has no column called 'nmae'.
    It has: {name}, {course}
    {number} also works, and counts the sheets for you.
```

That check matters more than it looks. Two hundred certificates reading
`{nmae}` is a bad day; two hundred reading *nothing at all* is worse, because
the stack looks right until somebody reads one. So a name in braces that is
not a column is left visible rather than quietly blanked.

Commas and line breaks inside a value are fine as long as the value is
quoted, which every spreadsheet does for you — `"Smith, John"` stays one name,
and an address typed across two lines of one cell stays one address.

**A picture each, too.** The file name can name a column, so everybody's sheet
gets their own signature, photograph or seal:

```bash
onionskin batch pass.pdf --from people.csv \
  --at '30,60:{name}' \
  --image 'photos/{name}.jpg:120,40:35'
```

A mistyped column in the file name is caught in the same breath as one in a
line of text — before a single sheet is made, rather than at the two hundredth.

### Envelopes, cards and postcards

`--page` knows them by name, so nobody has to look up that a DL is 110 by 220:

```bash
onionskin write envelope.pdf --page dl --at '20,60:Mrs A Wickham'
onionskin write card.pdf --page business-card --at '5,20:14 Mill Lane'
```

`dl`, `c4`, `c5`, `c6`, `monarch` and `comm10` for envelopes; `postcard`,
`index-card`, `business-card` and `a7` for cards. Alongside the paper sizes that
were already there — `onionskin write --help` lists them all, and any size at
all still works as `--page 100x150`.

### A signature, a stamp, or a logo

The commonest thing anybody adds to a document that is already printed:

```bash
onionskin write contract.pdf --image 'signature.png:120,240:40'
```

The file, where its top-left corner goes in millimetres, and how wide it is.
The height follows the picture's own shape, so a signature is never squashed
into a box it was not drawn for. Give `40x15` if you want both exactly.

**Transparency is carried through.** A signature saved on a see-through
background prints as a signature — not as a signature inside a white
rectangle covering the ruled line it is meant to be sitting on. PNG, JPEG,
TIFF and BMP all work.

A JPEG is passed through exactly as it arrived, because PDF reads JPEG
itself; a photographed letterhead stays the size it was instead of becoming
megabytes of raw pixels. A logo used on all two hundred sheets of a batch is
stored once.

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

### Ask the form where there is room

Filling in a printed form means finding the coordinates first, and that has
always meant a ruler against the paper, or opening the scan in an image editor
and reading pixels off it and converting — for every box on the page. The page
can be asked instead:

```bash
onionskin blanks form.pdf
```

```
7 place(s) to write on form.pdf, roomiest first:

   50,65 mm    145 mm wide, about 52 characters at 11 pt
   50,82 mm    145 mm wide, about 52 characters at 11 pt
   20,150 mm   170 mm wide, about 61 characters at 24 pt   (open area)
   ...

Use one by pasting its millimetres in:
  onionskin write form.pdf --at '50,65:Your words' --size 11
```

It reports two kinds of place: gaps beside words already printed — `Name:`
followed by six centimetres of nothing — and open areas with no ink in them at
all. Gaps on a line come back with that line's own baseline, so what you write
sits level with the label beside it. Nothing inside the border a printer cannot
reach is offered, and the spaces between ordinary words are not offered either,
or a page of prose would come back with two hundred entries.

It works on a scan as well as a PDF: the sheet is straightened onto the paper's
own grid first, so a millimetre in the answer is a millimetre on the paper
however crookedly it went on the glass.

```bash
onionskin blanks scan-of-form.png --page a4
onionskin blanks form.pdf --min-width 10      # narrower boxes too
onionskin blanks form.pdf --json              # for a script
```

### Look at it before you print it

The delta on its own is a nearly blank page. Whether *Approved* lands in the box
or across the line under it is not visible in it, and it is not visible in the
sheet either — only in the two together. Until now the only way to see that was
to print it.

```bash
onionskin proof invoice.pdf --delta delta.pdf -o proof.pdf
```

The sheet comes out in grey and what would be added on top of it in red, at the
real size of the paper, in a PDF anybody can open. Nothing goes near a printer.

```bash
onionskin proof invoice.pdf --delta delta.pdf --tracing      # the sheet as a faint hint
onionskin proof invoice.pdf --delta delta.pdf --colour blue  # or any colour
```

`--tracing` fades the existing page almost away, which is the same thing as
holding the delta against a window with the original behind it — the way this
was checked before there was a program to do it.

### How much room the words actually have

An uncalibrated printer is out by about ±2 mm on a second pass. Whether that
matters depends entirely on the job, and the page knows which:

```
note: The tightest addition has more than 8 mm of clear paper around it.
    More than the ±2 mm an uncalibrated printer is out by, so this sheet will
    come out right whether or not this printer has ever been measured.
```

```
WARNING: One addition has only 0.8 mm of clear paper around it.
    It is at 50,112 mm. An uncalibrated printer is out by about ±2 mm on a
    second pass, which is more than the gap — so this one can land on top of
    what is already there.
    Calibrate this printer first and it comes down to under half a millimetre:
      onionskin calibrate learn scan.png --delta <the delta>
```

A signature going into a wide empty box does not care what the printer does; the
same printer filling a ruled column cares very much. That is the whole question
behind "should I calibrate before this one", and it is answered per job rather
than left as a standing warning.

### Check the first sheet before doing sixty

Overprinting is the one operation where nothing tells you it went wrong. The
delta can be perfect and still land two millimetres low, or not print at all
because the sheet went in the wrong way up — and the file on disk says nothing
about any of it. Usually the mistake surfaces when somebody opens the envelope.

Scan the first sheet and be told:

```bash
onionskin verify sheet.png --delta delta.pdf
```

```
A4 (210.0×297.0 mm) at 200 dpi, sheet turned -0.00°, top-left at (60, 60) px

4 addition(s) asked for:
  ✓   40.1,38.5   mm   out by 0.29 mm
  ✓  160.9,43.4   mm   out by 0.21 mm
  ✓   37.5,248.7  mm   out by 0.34 mm
  ✓  150.9,253.6  mm   out by 0.27 mm

Everything printed, and nothing is more than 0.34 mm out of place.
```

It exits `2` when something did not print or landed further out than
`--tolerance` (a millimetre by default), so it can go in a script between the
first sheet and the rest of the stack. How close is close enough is yours to
set: a signature can be two millimetres out and a pre-printed box cannot.

Add `--learn office` and the same scan also teaches the printer's profile —
having gone to the trouble of scanning it, you may as well have the
measurement.

### Several deltas, one pass through the printer

A day's work on one document arrives as more than one delta. The paid stamp is a
saved job, the signature is a picture, the reference number came out of a
spreadsheet. Each of those is a delta, each of them prints, and printing three of
them means feeding the same sheet through the printer three times.

Every pass is a chance to lose the sheet. It can go in crooked, it can jam, it
can pick up the one underneath it, and it lands a little differently each time —
which is the entire reason this program has a calibration step. Three passes are
three of those chances on a piece of paper that already has the letterhead on it
and cannot be reprinted.

```bash
onionskin merge stamp.pdf signature.pdf reference.pdf -o all.pdf
```

One file, one pass. Print `all.pdf` **instead of** the deltas it was made from —
printing both it and them puts the ink down twice.

```bash
onionskin merge stamp.pdf signature.pdf -o all.pdf --print-to "Office Laser"
```

The deltas are drawn in the order they are given, so a later one lands on top of
an earlier one where they overlap.

Each delta keeps its own typeface, its own pictures and its own colours: they go
in as self-contained parcels rather than being glued together, so two deltas that
both happen to call their first font `F0` cannot end up sharing one. Deltas
written by some other program merge just as well as Onionskin's own.

Before anything is written it checks the pages are the same size. Merging a
letter's delta with an invoice's would print one of them off the edge of the
paper, so it is refused with both sizes named:

```
error: these are not deltas for the same sheet of paper. Page 1 of stamp.pdf is
A4 (210.0×297.0 mm), and page 1 of usdelta.pdf is Letter (215.9×279.4 mm).
Merging them would print one of them off the edge.
```

A file that runs out early simply stops contributing, so a one-page stamp merges
onto the front of a five-page invoice and the other four pages are left alone.
And the same delta given twice is pointed out rather than refused — the merged
file is fine, but every letter in it would be printed twice in the same place,
which comes out heavier and blurred.

### See what changed

A delta prints only what is new, which is the point — and can make the change
hard to find on the sheet afterwards. `--outline` draws a box round each one:

```bash
onionskin delta before.odt after.odt -o delta.pdf --outline
onionskin delta before.odt after.odt -o delta.pdf --outline --outline-colour blue
```

Boxes that would cross each other are merged into one, and none of them runs off
the paper. It is off by default because the box is printed onto the sheet along
with the change, and is therefore just as permanent.

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
box. Calibration gets it under **±0.5 mm**.

There are two ways to get there. The second one is a chore, so start with the
first.

### Learn from a job you were printing anyway

Every delta ever printed is a set of marks in known places. So the sheet that
comes out of the printer *is* a calibration target — scan it afterwards and
Onionskin reads its own error off it:

```bash
onionskin write invoice.pdf --at '30,40:Paid' -o delta.pdf   # print this
onionskin calibrate learn scan.png --delta delta.pdf --name office
```

```
A4 (210.0×297.0 mm) at 200 dpi, sheet turned -0.00°, top-left at (60, 60) px

Where the additions landed:
    40.1,38.5   mm   out by 1.38 mm
   160.9,43.4   mm   out by 1.30 mm
    37.5,248.7  mm   out by 1.43 mm
   150.9,253.6  mm   out by 1.36 mm

What this printer does:
profile 'office'
  printer error : shift +1.11, +0.80 mm, rotate -0.010° cw, scale 1.00001
  correction    : shift -1.11, -0.80 mm, rotate +0.010° cw, scale 0.99999
  fit           : 4 points, rms 0.047 mm, max 0.058 mm
```

No target sheet, no ruler, no separate errand — one scan of work you were doing
regardless. `--dry-run` shows the numbers without saving. Run it again next
week and it gets better: the correction already in force is measured *through*,
so a printer that is now landing on the mark is understood as a printer being
corrected, not a printer with no error.

It needs at least three additions that can be read. An addition that did not
print is left out, and so is one that landed on top of something already on the
sheet — the middle of two overlapping marks is the middle of neither. If the
additions run down one column, as a form filled in on the left does, you get
the shift and nothing else: marks in a line say nothing about the direction
across them, and a rotation invented from them would be wrong everywhere else
on the page. The profile says so when that happens.

### Or measure a target sheet, once

```bash
onionskin calibrate target -o target.pdf                 # A4 by default
onionskin calibrate target -o target.pdf --page legal    # or a5, a3, tabloid…
onionskin calibrate target -o target.pdf --page 100x150  # or any size in mm
```

The target is two pages. Print **page 1** on blank paper at 100%, put that same
sheet back in the tray, and print **page 2** onto it. Each crosshair now carries
a cross from the first pass and a diamond from the second, and the gap between
them is the printer's error.

Then scan the sheet and let Onionskin read it:

```bash
onionskin calibrate measure sheet.png --name office
```

```
Measured off the sheet:
  P1  +0.75, -0.40 mm
  P2  +0.74, -0.47 mm
  P3  +0.77, -0.43 mm
  P4  +0.80, -0.37 mm
  P5  +0.67, -0.42 mm

shift -0.74, +0.42 mm, rotate +0.008° cw, scale 0.99989 (-0.011%)
  the fit misses each crosshair by 0.05 mm on average
```

That is the whole of it. `--dry-run` shows the numbers without saving anything.
A crosshair the scan cannot resolve is left out rather than guessed at, and
fewer than three readable ones is refused — two points and a similarity is not a
fit, it is four numbers through four numbers.

**No scanner?** The rulers are still printed beside every crosshair. Read the
offsets off them by eye — right is `+x`, down is `+y` — and type them in:

```bash
onionskin calibrate solve --name office \
  --point 'P1:+0.40,-0.15' --point 'P2:+0.35,-0.20' \
  --point 'P3:+0.45,-0.10' --point 'P4:+0.40,-0.15'
```

Either route fits shift, rotation and scale — the full space of error a paper path can
introduce — and stores it in `~/.onionskin/profiles/`. Every later run applies
the inverse:

```bash
onionskin delta a.docx b.docx -o delta.pdf --profile office
```

Calibrate once per printer, per tray. Calibrate on the paper you actually print
on: a shift carries over to any sheet size, but rotation and scale are applied
about the centre of the page, so an A4 profile used on Legal leaves some error
behind. Onionskin says so when it spots the mismatch.

## Fonts, and where they live

`onionskin fonts` lists the eight faces built into every PDF reader, which need
nothing installed anywhere and are what the delta uses unless told otherwise.

LibreOffice ships its own fonts *inside its installation* rather than putting
them where the system keeps fonts, which is the whole reason a face that looks
right in Writer can be one Onionskin has never heard of. It now looks in
LibreOffice's folders on all three systems, in the per-user places a font
installed by double-clicking lands, and in any folder you name:

```bash
onionskin fonts                        # the built-in eight, and a count of the rest
onionskin fonts --all                  # every font file found, and where it is
onionskin fonts --folders              # where it looked
onionskin fonts --add-folder ~/Fonts   # look here too, from now on
onionskin fonts --forget-folder ~/Fonts
```

The folder is remembered between runs. One on a drive that is not plugged in
today is skipped rather than complained about, and stays remembered for when it
comes back.

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

It reads them too, and works out for itself what the page is set in — nobody
looking at a scan of a letter knows which of three faces it was typed in, and
being asked is what stops people at the first step:

```bash
onionskin read letter.png
```

```
42 letters in 9 words on 3 lines.
Read automatically: Helvetica at about 14.1 pt, from 9 words
```

It reads the page against each face it has and keeps whichever accounted for the
most ink — which is also how it can say what it decided, and how sure it is.

Naming a font is still there, and is how an alphabet the three built-in faces do
not cover gets read:

```bash
onionskin read letter.png --font-file ~/fonts/NotoSansGreek.ttf
```

The alphabet is then whatever that font can draw, so the language is whatever
the page is in. In the window it lives under **Read it in a particular font**,
folded away, because most of the time there is nothing to answer. See
[ARCHITECTURE.md](ARCHITECTURE.md) for how homoglyphs and right-to-left scripts
are handled, and for the two honest limits (cursive scripts and combining
marks).

## The expert numbers

Four numbers decide what Onionskin considers to be a change: how dark a pixel
must be to count as ink, how far apart two marks can be and still be one change,
how small a speck to ignore, and how far ink may move and still count as
unchanged. The defaults are right for paper. They are reachable anyway, because
a program that hides its own workings is asking to be trusted rather than
checked:

```bash
onionskin delta a.pdf b.pdf -o delta.pdf   --ink-threshold 200 --group 2 --min-region 0.05 --tolerance 0.12 --pad 0.3
```

In the window they are under **Settings ▸ Expert**, two doors deep on purpose,
with a button that puts them all back.

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

### Finding them

You should not have to know your printer's address. `onionskin printers` asks
this machine what is set up on it — which is where a printer plugged in by USB
appears — and then listens for anything announcing itself on the local network,
and prints an address you can paste straight back in:

```bash
$ onionskin printers
Set up on this machine, including anything plugged in by USB:

  Brother_HL_L2350DW
    Brother HL-L2350DW series
    --printer ipp://127.0.0.1:631/printers/Brother_HL_L2350DW

Announcing themselves on this network:

  Reception
    HP LaserJet 400
    --printer ipp://192.168.1.5/ipp/print
```

`onionskin scanners` does the same: anything on a USB cable, through SANE, and
any eSCL scanner on the network. In the window it is one button — **Find my
printers and scanners** — and each result has a button that fills the address
in for you.

The discovery is DNS-SD, written in Rust here rather than pulled in: multicast
UDP, the DNS message format with its compression pointers, guarded against
malformed packets. It asks this network and no other.

```bash
onionskin printers --no-network                      # only what is set up here
onionskin printers --server ipp://printer.local/     # or ask a printer directly
onionskin fetch -o scan.png --scanner http://printer.local/eSCL --capabilities
onionskin send delta.pdf --printer ipp://printer.local/ipp/print --copies 2
```

## Sheets of labels

Address labels, file labels, shelf labels. The stock comes pre-cut in a grid and
the job is always the same: take a column of names and put one in each label.

```bash
onionskin labels --from addresses.csv --grid 3x8 --label 63.5x33.9 \
  --text '{name}\n{address}\n{town} {postcode}'
```

This is the one thing here that is *not* an overlay on something already
printed — label stock is blank, so nothing is rendered and nothing is diffed.
It is in this program because the hard part is the same hard part: getting ink
onto a particular rectangle of a particular sheet, in millimetres, and being
right about it.

**The half-used sheet.** Nobody ever uses a whole one. There is always a sheet
in the drawer with the first five peeled off:

```bash
onionskin labels --from addresses.csv --grid 3x8 --label 63.5x33.9 \
  --text '{name}' --start 6
```

**A grid that runs off the paper is refused before anything is written**, with
the numbers, because it would not fail — it would print onto the backing paper
and cost you a sheet:

```
error: the labels run 44.0 mm off the right-hand edge. 4 columns of 63.5 mm,
    2.5 mm apart, starting 7.0 mm in, needs 254.0 mm and the paper is 210.0 mm.
```

A label with more lines than fit is warned about for the same reason. `--first 1`
makes one sheet, to try on plain paper against the real stock before committing
the box.

**Why the grid is given rather than looked up.** Label stock is sold by a code —
Avery 5160, L7160, a hundred others — and those codes mean different sizes in
different countries and change between years. A built-in table would be wrong
for somebody, silently, on paper. The measurements are on the box, they are four
numbers, and being asked once beats a boxful printed half a millimetre out.

## The same job, again next week

An office does the same thing to the same form every day. The paid stamp goes at
150,40 in nine point; the received date goes under the third line. Working that
out once is fine — working it out again every Monday, out of a note in somebody's
head or a shell history that has scrolled away, is how a box of letterhead gets
reprinted.

Save it once:

```bash
onionskin write invoice.pdf --at '150,40:PAID {today}' --size 9 --save-as paid
```

```
Saved as job 'paid' in ~/.onionskin/jobs/paid.json.
  onionskin job run paid <another document>
```

Run it on tomorrow's:

```bash
onionskin job run paid invoice-4472.pdf
onionskin job list
onionskin job show paid
onionskin job delete paid
```

**`{today}` fills itself in.** It is the commonest thing anybody stamps onto a
piece of paper, it is different every day, and somebody typing it by hand
eventually stamps yesterday's. `{year}`, `{month}` and `{day}` too. Say
`--set today=2026-07-26` and you are believed, for the post that should have
gone out yesterday.

**Anything else in braces is a blank the job will ask for**, and it asks
*before* writing anything:

```bash
onionskin write letter.pdf --after 'Our ref:{ref}' --save-as our-ref
onionskin job run our-ref letter.pdf
```

```
error: job 'our-ref' needs {ref} filled in.
    --set ref=…
    onionskin job show our-ref   says what it wants and why
```

That is the same reasoning as the CSV batch: a hundred letters reading `{ref}`
is a bad afternoon, and a hundred reading nothing at all is worse, because they
look finished.

`--dry-run` shows exactly what it would place, filled in, without writing a
file. Running a job never re-saves it — otherwise "the job" quietly becomes
whatever it was last used for, which is the one thing a saved job must not be.

## What was added to which sheet, and when

Overprinting is the one thing Onionskin does that cannot be undone. Toner does
not come off paper, so a delta printed twice onto the same sheet puts every
letter down twice — and it is an easy mistake, because the delta is a file like
any other and the second time looks exactly like the first.

So every delta is remembered by a fingerprint of the file itself, and writing
the same one again says so:

```
NOTE: this is the same delta you wrote a moment ago (2026-07-27 15:03), as invoice-delta.pdf.
  Printing it onto a sheet that already has it puts the ink down twice, and that
  cannot be undone. Onto a fresh sheet it is exactly right.
  Everything written so far:  onionskin history
```

It does not refuse — printing one delta onto a hundred *different* sheets is
what a hundred certificates are. It just asks the question.

The record is worth having on its own account. "What did we add to that invoice,
and when" is a question somebody asks months later about a sheet in a filing
cabinet:

```bash
onionskin history              # most recent first
onionskin history --limit 100
onionskin history --json
onionskin history --forget
```

**The words themselves are never kept** — only which files were involved, how
much went on, and the fingerprint. A log of everything anybody ever wrote onto
anything would be a far more sensitive file than any document it described,
sitting in a home directory being backed up. The fingerprint recognises a delta
without describing it.

## Privacy and networking

Onionskin never phones home. There is no telemetry, no update check, and no
external asset in the browser UI — the page it serves contains nothing fetched
from anywhere else. Verified by tracing a full delta run: not one internet
socket is opened.

It opens a socket in exactly two situations, both of which you asked for:

* When **you name a printer or a scanner**. Then it talks to that machine, on
  your own network, and to nothing else.
* When **you ask it to find them** — `onionskin printers`, `onionskin scanners`,
  or the Find button in the window. Then it sends a DNS-SD question to the
  local multicast address and listens for a couple of seconds. That question
  carries nothing about you and nothing about your documents; it reaches the
  local network and no further, because multicast DNS is not routed off it.
  `--no-network` skips it entirely.

There is no beacon, no directory service and no check for updates. If you never
name a printer and never ask it to look for one, nothing leaves the machine.

`onionskin serve` binds to `127.0.0.1`, so the UI is reachable only from your
own machine. It has no password: if you override `--host`, anyone who can reach
that address can upload documents and read every delta, and Onionskin warns you
when you do. Working files are written with owner-only permissions so that other
accounts on a shared machine cannot read documents you have processed.

Onionskin also refuses to write a delta over one of the documents it was made
from, since that would destroy the sheet you were about to print onto.

## Your defaults, not Onionskin's

Onionskin has to choose something when it is not told — four hundred dots an
inch, a five millimetre margin, A4, no box round the changes. Those are right
for most people most of the time and wrong for somebody every day. Say it once
instead of in every command you ever type:

```bash
onionskin config                       # what is set, and what could be
onionskin config set dpi 300
onionskin config set outline yes
onionskin config set page letter
onionskin config unset dpi             # back to Onionskin's own answer
onionskin config reset                 # all of them
```

**A flag always beats a setting.** Stating a preference does not cost you the
ability to depart from it for one run — `--dpi 600` still wins, and
`--no-outline` turns the boxes off for one delta without changing anything.

A value that is out of range is refused the moment you type it, naming what is
wrong, rather than being stored and met as an error on some later run you have
forgotten this by.

## Undo

`erase` takes a piece of text off a page, and there used to be no way back from
it — nor from an `edit` that replaced the wrong item, nor a `write` at the wrong
millimetre.

```bash
onionskin undo letter.onionskin
```

```bash
onionskin redo letter.onionskin
```

Every command that changes a document sets the previous version aside first,
in a numbered file beside it. **Ten steps are kept**, so three mistakes in a
row can all be undone — and `redo` brings them back.

Undo used to swap the two versions, so running it again went forward. That
was neat and it was wrong: it meant you could not undo twice, because the
second undo put the first one back. Going back is going back now, however
many times you ask, and coming forward is a different word.

Ten, not a hundred. The mistake somebody wants undone is nearly always the
last one or the one before it, and a folder holding fifty copies of a letter
is its own kind of mess.

Making a new change after undoing forgets what could have been redone — those
versions are no longer anywhere the document can get back to, and offering
them would hand you a document that never existed.

In the window there is an Undo button beside the page controls, on both the
Make a document and Draw on a page screens — the two places where removing
something is a single click. It is greyed out until there is something to go
back to.

## Your files are not Onionskin's to destroy

`-o report.pdf` used to write over a `report.pdf` that had nothing to do with
Onionskin, in silence, and report success. It stops now:

```
error: 'report.pdf' is already there, and Onionskin did not write it — so it
has been left alone.
    Write over it:  add --overwrite
    Keep it:        choose another name for the PDF
```

Onionskin stamps everything it writes, so the ordinary loop — make a delta,
look at it, edit, make it again — asks nothing at all. Only a name it cannot
claim stops it, and `--overwrite` is always there when you mean it.

`--overwrite` is deliberately not the same flag as the `--force` that `print`
and `delta` have. That one means "print it anyway, I have read the warning
about reflow". This one means "yes, that file of mine can go". Two different
things to be sure about.

One place this costs you something: scans are written by SANE's `scanimage`,
not by Onionskin, so it cannot stamp them and cannot recognise them later.
Scanning twice to the same name asks for `--overwrite` the second time. That
is the price of never scanning over a photograph, and it seemed the right way
round.

## What Onionskin keeps, and getting rid of it

A program that stores things in a hidden folder should be willing to say so
without being asked twice. `onionskin doctor` ends by listing everything it
holds and where:

```
What Onionskin keeps, all under /home/you/.onionskin:
  settings   your defaults — onionskin config show
  profiles   2 — office-laser, the-old-inkjet
  deltas     3 kept back (1.2 MB), from runs that asked to keep them
             remove them:  onionskin tidy
```

```bash
onionskin tidy      # delete the deltas it is holding, and say what went
```

Deltas are already deleted as new ones are made — this is for anyone who
wants them gone now rather than at the next run, or who would simply rather
decide themselves. Nothing outside that folder is ever touched; your own
files are never in it. The whole folder can be deleted by hand with no harm
beyond losing exactly what the list says it holds.

## Tab completion

```bash
onionskin completions bash > ~/.local/share/bash-completion/completions/onionskin
onionskin completions zsh  > ~/.zsh/completions/_onionskin
onionskin completions fish > ~/.config/fish/completions/onionskin.fish
onionskin completions powershell >> $PROFILE
```

With no shell named it guesses from `$SHELL`. The script is generated from the
same command definitions `--help` comes from, so it cannot drift: a flag added
tomorrow is one Tab knows about tomorrow.

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
