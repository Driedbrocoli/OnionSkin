"""Typing directly onto a page that is already printed.

The diff workflow asks you to edit in Word and works out what changed. This one
skips the round trip: you say *put these words at this spot on page 2*, and
Onionskin writes them onto a delta PDF at exactly that spot.

The important consequence is structural. Text placed at an absolute position
cannot push anything else around, so the reflow that blocks the diff
workflow — insert a word, everything after it shifts, the sheet in your hand no
longer matches — simply cannot happen here. Nothing else on the page moves,
because nothing else on the page is being laid out.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field, fields
from pathlib import Path
from typing import Sequence

from reportlab.lib.colors import Color, HexColor
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfgen import canvas

from .geometry import PageSize, mm_to_pt, pt_to_mm

#: The 14 fonts every PDF reader has built in. Using these keeps the delta
#: small and avoids embedding, which matters when the file is going straight to
#: a printer driver.
STANDARD_FONTS = (
    "Helvetica", "Helvetica-Bold", "Helvetica-Oblique", "Helvetica-BoldOblique",
    "Times-Roman", "Times-Bold", "Times-Italic", "Times-BoldItalic",
    "Courier", "Courier-Bold", "Courier-Oblique", "Courier-BoldOblique",
    "Symbol", "ZapfDingbats",
)

ALIGNMENTS = ("left", "center", "right")


class LayoutError(ValueError):
    """A text box cannot be placed as described."""


def available_fonts() -> tuple[str, ...]:
    return STANDARD_FONTS


def register_font(path: str | Path, name: str | None = None) -> str:
    """Make a TrueType font available for composing, returning its name."""
    from reportlab.pdfbase.ttfonts import TTFont

    path = Path(path)
    if not path.is_file():
        raise LayoutError(f"no font file at {path}")
    family = name or path.stem
    pdfmetrics.registerFont(TTFont(family, str(path)))
    return family


def _known_font(name: str) -> bool:
    if name in STANDARD_FONTS:
        return True
    try:
        pdfmetrics.getFont(name)
        return True
    except Exception:
        return False


def _parse_colour(value: str | Color) -> Color:
    if isinstance(value, Color):
        return value
    text = str(value).strip()
    try:
        return HexColor(text if text.startswith("#") else f"#{text}")
    except Exception as exc:
        raise LayoutError(f"'{value}' is not a colour like #1a1a1a") from exc


@dataclass
class TextBox:
    """Words to place at a fixed spot on a page.

    ``x_mm`` and ``y_mm`` are the top-left corner of the text block, measured
    from the top-left of the sheet — the same frame the rest of Onionskin uses,
    and the one you would use with a ruler.
    """

    page: int = 0  # zero-based
    x_mm: float = 20.0
    y_mm: float = 20.0
    text: str = ""
    size_pt: float = 11.0
    font: str = "Helvetica"
    width_mm: float | None = None  # wrap width; None means never wrap
    line_spacing: float = 1.15
    align: str = "left"
    colour: str = "#000000"
    rotation_deg: float = 0.0

    def validate(self, page_count: int) -> None:
        if not self.text.strip():
            raise LayoutError("a text box with no text would print nothing")
        if not 0 <= self.page < page_count:
            raise LayoutError(
                f"page {self.page + 1} is not in the document "
                f"(it has {page_count} page(s))"
            )
        if self.size_pt <= 0 or self.size_pt > 400:
            raise LayoutError(f"font size {self.size_pt} pt is out of range")
        if not _known_font(self.font):
            raise LayoutError(
                f"unknown font '{self.font}'. Built in: {', '.join(STANDARD_FONTS)}"
            )
        if self.align not in ALIGNMENTS:
            raise LayoutError(f"align must be one of {ALIGNMENTS}")
        if self.width_mm is not None and self.width_mm <= 0:
            raise LayoutError("wrap width must be positive")
        if self.line_spacing <= 0:
            raise LayoutError("line spacing must be positive")
        _parse_colour(self.colour)

    def lines(self) -> list[str]:
        """Split into printed lines, wrapping at ``width_mm`` if set."""
        paragraphs = self.text.replace("\r\n", "\n").split("\n")
        if self.width_mm is None:
            return paragraphs

        limit = mm_to_pt(self.width_mm)
        out: list[str] = []
        for paragraph in paragraphs:
            words = paragraph.split()
            if not words:
                out.append("")
                continue
            line = words[0]
            for word in words[1:]:
                candidate = f"{line} {word}"
                if pdfmetrics.stringWidth(candidate, self.font, self.size_pt) <= limit:
                    line = candidate
                else:
                    out.append(line)
                    line = word
            out.append(line)
        return out

    @property
    def line_height_mm(self) -> float:
        return pt_to_mm(self.size_pt * self.line_spacing)

    def block_size_mm(self) -> tuple[float, float]:
        """Width and height of the text block as it will be drawn."""
        lines = self.lines()
        widest = max(
            (pdfmetrics.stringWidth(line, self.font, self.size_pt) for line in lines),
            default=0.0,
        )
        width = self.width_mm if self.width_mm is not None else pt_to_mm(widest)
        return width, self.line_height_mm * len(lines)

    def to_dict(self) -> dict:
        """Serialise for a layout file or the web API.

        ``page`` is written **1-based**: everywhere a person types a page
        number — the CLI, a layout file, the browser — pages start at 1, the
        way they are counted on the sheet. Only the attribute is 0-based.
        """
        return {
            "page": self.page + 1,
            "x_mm": self.x_mm,
            "y_mm": self.y_mm,
            "text": self.text,
            "size_pt": self.size_pt,
            "font": self.font,
            "width_mm": self.width_mm,
            "line_spacing": self.line_spacing,
            "align": self.align,
            "colour": self.colour,
            "rotation_deg": self.rotation_deg,
        }

    @classmethod
    def from_dict(cls, data: dict) -> "TextBox":
        if not isinstance(data, dict):
            raise LayoutError(f"a text box must be an object, got {type(data).__name__}")
        known = {f.name for f in fields(cls)}
        unknown = set(data) - known
        if unknown:
            raise LayoutError(
                f"unexpected text box setting(s): {', '.join(sorted(unknown))}. "
                f"Known: {', '.join(sorted(known))}"
            )
        values = {k: v for k, v in data.items() if k in known}
        if "page" in values:
            try:
                page = int(values["page"])
            except (TypeError, ValueError) as exc:
                raise LayoutError(f"'{values['page']}' is not a page number") from exc
            if page < 1:
                raise LayoutError("pages are numbered from 1")
            values["page"] = page - 1
        return cls(**values)


def load_layout(path: str | Path) -> list[TextBox]:
    """Read a saved layout: either ``{"boxes": [...]}`` or a bare list."""
    path = Path(path)
    if not path.is_file():
        raise LayoutError(f"no layout file at {path}")
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise LayoutError(f"{path.name} is not valid JSON: {exc}") from exc

    raw = data.get("boxes", data) if isinstance(data, dict) else data
    if not isinstance(raw, list):
        raise LayoutError("a layout must be a list of text boxes")
    return [TextBox.from_dict(item) for item in raw]


def save_layout(boxes: Sequence[TextBox], path: str | Path) -> Path:
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps({"boxes": [b.to_dict() for b in boxes]}, indent=2),
        encoding="utf-8",
    )
    return path


def parse_text_spec(spec: str) -> TextBox:
    """Parse ``2:60,150:Approved 25 July`` — page, position, then the words.

    The page number is 1-based here because that is what is written on the
    sheet in front of you.
    """
    parts = spec.split(":", 2)
    if len(parts) != 3:
        raise LayoutError(
            f"bad --text '{spec}'. Expected 'PAGE:X,Y:the words', "
            "e.g. '1:60,150:Approved 25 July'"
        )
    page_raw, position, text = parts
    try:
        page = int(page_raw.strip())
    except ValueError as exc:
        raise LayoutError(f"'{page_raw}' is not a page number in '{spec}'") from exc
    if page < 1:
        raise LayoutError("pages are numbered from 1")

    coords = position.replace(" ", "").split(",")
    if len(coords) != 2:
        raise LayoutError(f"bad position '{position}' in '{spec}'. Expected 'X,Y' in mm")
    try:
        x_mm, y_mm = float(coords[0]), float(coords[1])
    except ValueError as exc:
        raise LayoutError(f"position in '{spec}' is not a pair of numbers") from exc

    return TextBox(page=page - 1, x_mm=x_mm, y_mm=y_mm, text=text)


@dataclass
class Composition:
    """A set of text boxes bound to a document's page geometry."""

    page_sizes: list[PageSize]
    boxes: list[TextBox] = field(default_factory=list)

    def validate(self) -> None:
        if not self.boxes:
            raise LayoutError("nothing to place — add at least one text box")
        for box in self.boxes:
            box.validate(len(self.page_sizes))

    def boxes_on(self, page: int) -> list[TextBox]:
        return [b for b in self.boxes if b.page == page]


def compose(
    composition: Composition,
    out_path: str | Path,
    title: str = "Onionskin delta",
) -> Path:
    """Write the delta PDF: blank pages carrying only the placed text."""
    composition.validate()
    out_path = Path(out_path)
    out_path.parent.mkdir(parents=True, exist_ok=True)

    first = composition.page_sizes[0]
    pdf = canvas.Canvas(
        str(out_path), pagesize=(first.width_pt, first.height_pt), pageCompression=1
    )
    pdf.setTitle(title)
    pdf.setProducer("Onionskin")
    pdf.setSubject("Additions only — print onto the already-printed sheet")

    for index, size in enumerate(composition.page_sizes):
        pdf.setPageSize((size.width_pt, size.height_pt))
        for box in composition.boxes_on(index):
            _draw_box(pdf, box, size)
        pdf.showPage()

    pdf.save()
    return out_path


def _draw_box(pdf: canvas.Canvas, box: TextBox, page: PageSize) -> None:
    lines = box.lines()
    pdf.saveState()
    pdf.setFillColor(_parse_colour(box.colour))
    pdf.setFont(box.font, box.size_pt)

    # Work in a frame whose origin is the block's top-left corner, so rotation
    # pivots there rather than around the page.
    pdf.translate(mm_to_pt(box.x_mm), page.height_pt - mm_to_pt(box.y_mm))
    if box.rotation_deg:
        pdf.rotate(-box.rotation_deg)  # page-space clockwise is PDF anticlockwise

    ascent = pdfmetrics.getAscent(box.font, box.size_pt)
    step = box.size_pt * box.line_spacing
    wrap_pt = mm_to_pt(box.width_mm) if box.width_mm is not None else None

    for i, line in enumerate(lines):
        y = -ascent - i * step
        if not line:
            continue
        if box.align == "left":
            pdf.drawString(0, y, line)
            continue
        # Centre and right need a column to align within: the wrap width when
        # there is one, otherwise the widest line.
        width = wrap_pt if wrap_pt is not None else _widest(lines, box)
        if box.align == "center":
            pdf.drawCentredString(width / 2.0, y, line)
        else:
            pdf.drawRightString(width, y, line)

    pdf.restoreState()


def _widest(lines: Sequence[str], box: TextBox) -> float:
    return max(
        (pdfmetrics.stringWidth(line, box.font, box.size_pt) for line in lines),
        default=0.0,
    )
