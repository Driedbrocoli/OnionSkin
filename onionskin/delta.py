"""Writing the delta PDF — the transparent sheet you print onto the original.

Two ways to build it, with a real trade-off between them:

``raster``
    Print exactly the pixels that are new, and nothing else. Anti-aliasing is
    recovered as an alpha channel so glyph edges stay smooth. This can never
    re-print ink that is already on the sheet, which makes it the safe default.

``vector``
    Clip the edited PDF to the changed regions and keep the original vector
    text. Sharper at any print resolution, but a clip rectangle is a rectangle:
    if a new word sits hard against an existing one, a sliver of the existing
    word falls inside the box and gets printed a second time, very slightly
    offset. On close inspection that reads as a bolded or blurred character.
"""

from __future__ import annotations

import io
from pathlib import Path
from typing import Sequence

import numpy as np
import pikepdf
from PIL import Image
from reportlab.lib.utils import ImageReader
from reportlab.pdfgen import canvas

from .diff import PageDiff, Region
from .geometry import PageSize, Similarity, mm_to_pt

PRODUCER = "Onionskin"


def _unmatte(rgb: np.ndarray, mask: np.ndarray) -> Image.Image:
    """Recover straight RGBA ink from a render composited over white paper.

    A renderer gives us ``C = a*K + (1-a)*255`` — ink colour ``K`` at coverage
    ``a`` over white. Coverage comes from the darkest channel, and the ink
    colour follows. Without this step every anti-aliased edge pixel would be
    printed at full opacity and new text would sit inside a pale halo.
    """
    h, w = mask.shape
    out = np.zeros((h, w, 4), dtype=np.uint8)
    ys, xs = np.nonzero(mask)
    if ys.size == 0:
        return Image.fromarray(out, mode="RGBA")

    px = rgb[ys, xs].astype(np.float32)
    coverage = (255.0 - px.min(axis=1)) / 255.0
    coverage = np.clip(coverage, 0.0, 1.0)

    safe = np.maximum(coverage, 1e-3)[:, None]
    ink = (px - (1.0 - safe) * 255.0) / safe
    ink = np.clip(ink, 0.0, 255.0)

    out[ys, xs, :3] = ink.astype(np.uint8)
    out[ys, xs, 3] = np.clip(coverage * 255.0, 0, 255).astype(np.uint8)
    return Image.fromarray(out, mode="RGBA")


def _pdf_rects(regions: Sequence[Region], page: PageSize) -> bytes:
    """Region boxes as PDF-space rectangle operators (y flips to point up)."""
    ops = []
    for r in regions:
        x = mm_to_pt(r.x0_mm)
        y = page.height_pt - mm_to_pt(r.y1_mm)
        ops.append(
            f"{x:.4f} {y:.4f} {mm_to_pt(r.width_mm):.4f} "
            f"{mm_to_pt(r.height_mm):.4f} re"
        )
    return (" ".join(ops)).encode("ascii")


class RasterDeltaWriter:
    """Streams a raster delta a page at a time.

    Pages are written and released as they arrive so a long document never
    holds more than one page of pixels in memory.
    """

    def __init__(self, out_path: str | Path, title: str = "Onionskin delta"):
        self.out_path = Path(out_path)
        self.out_path.parent.mkdir(parents=True, exist_ok=True)
        self._pdf = canvas.Canvas(
            str(self.out_path), pagesize=(595.276, 841.89), pageCompression=1
        )
        self._pdf.setTitle(title)
        self._pdf.setProducer(PRODUCER)
        self._pdf.setSubject("Additions only — print onto the already-printed sheet")

    def add_page(self, diff: PageDiff, rgb: np.ndarray | None = None) -> None:
        self._pdf.setPageSize((diff.size.width_pt, diff.size.height_pt))
        if rgb is not None and diff.added_px:
            h, w = diff.added.shape
            image = _unmatte(rgb[:h, :w], diff.added)
            buf = io.BytesIO()
            image.save(buf, format="PNG", optimize=True)
            buf.seek(0)
            self._pdf.drawImage(
                ImageReader(buf),
                0,
                0,
                width=diff.size.width_pt,
                height=diff.size.height_pt,
                mask="auto",
            )
        self._pdf.showPage()

    def close(self) -> Path:
        self._pdf.save()
        return self.out_path


class VectorDeltaWriter:
    """Streams a vector delta by clipping the edited PDF to changed regions."""

    def __init__(
        self,
        out_path: str | Path,
        edited_pdf: str | Path,
        pad_mm: float = 0.3,
        title: str = "Onionskin delta",
    ):
        self.out_path = Path(out_path)
        self.out_path.parent.mkdir(parents=True, exist_ok=True)
        self.pad_mm = pad_mm
        self.title = title
        self._src = pikepdf.open(str(edited_pdf))
        self._out = pikepdf.new()

    def add_page(self, diff: PageDiff, rgb: np.ndarray | None = None) -> None:
        page = self._out.add_blank_page(
            page_size=(diff.size.width_pt, diff.size.height_pt)
        )
        if not diff.added_regions or diff.index >= len(self._src.pages):
            return
        regions = [r.padded(self.pad_mm, diff.size) for r in diff.added_regions]
        page.add_overlay(self._src.pages[diff.index])
        page.contents_add(
            b"q " + _pdf_rects(regions, diff.size) + b" W n ", prepend=True
        )
        page.contents_add(b" Q")

    def close(self) -> Path:
        try:
            with self._out.open_metadata() as meta:
                meta["dc:title"] = self.title
                meta["pdf:Producer"] = PRODUCER
            self._out.save(str(self.out_path))
        finally:
            self._out.close()
            self._src.close()
        return self.out_path


def build_raster_delta(
    diffs: Sequence[PageDiff],
    new_rgb: Sequence[np.ndarray],
    out_path: str | Path,
    title: str = "Onionskin delta",
) -> Path:
    writer = RasterDeltaWriter(out_path, title)
    for diff, rgb in zip(diffs, new_rgb):
        writer.add_page(diff, rgb)
    return writer.close()


def build_vector_delta(
    diffs: Sequence[PageDiff],
    edited_pdf: str | Path,
    out_path: str | Path,
    pad_mm: float = 0.3,
    title: str = "Onionskin delta",
) -> Path:
    writer = VectorDeltaWriter(out_path, edited_pdf, pad_mm, title)
    for diff in diffs:
        writer.add_page(diff)
    return writer.close()


def apply_correction(
    pdf_path: str | Path,
    out_path: str | Path,
    correction: Similarity,
    sizes: Sequence[PageSize],
) -> Path:
    """Re-place every page's content through a calibration transform.

    The transform is prepended to each page as a single ``cm`` matrix and
    balanced with a trailing ``Q``, which keeps vector text vector, keeps every
    resource attached to the page it came from, and leaves the media box — the
    physical sheet — untouched. Only the ink moves.
    """
    pdf_path = Path(pdf_path)
    out_path = Path(out_path)
    if correction.is_identity:
        if pdf_path != out_path:
            out_path.write_bytes(pdf_path.read_bytes())
        return out_path

    with pikepdf.open(str(pdf_path)) as pdf:
        for i, page in enumerate(pdf.pages):
            size = sizes[i] if i < len(sizes) else sizes[-1]
            m = correction.to_pdf_matrix(size)
            page.contents_add(
                f"q {m.a:.9f} {m.b:.9f} {m.c:.9f} "
                f"{m.d:.9f} {m.e:.6f} {m.f:.6f} cm ".encode("ascii"),
                prepend=True,
            )
            page.contents_add(b" Q")
        pdf.save(str(out_path))
    return out_path


def _display_to_user_matrix(frame, display: PageSize) -> tuple[float, ...]:
    """Map delta (display-space) coordinates into the source page's user space.

    The delta is written as a plain page: origin at (0,0), the right way up,
    the size you see on screen. The source page may be none of those things.
    This returns the PDF matrix that puts the delta's ink exactly where the
    same spot appears on the source page, so both land together on the sheet.
    """
    x0, y0 = frame.crop[0], frame.crop[1]
    width, height = frame.crop_size_pt
    rotate = frame.rotate

    if rotate == 90:
        #  a  b  c  d   e         f
        return (0.0, 1.0, -1.0, 0.0, x0 + width, y0)
    if rotate == 180:
        return (-1.0, 0.0, 0.0, -1.0, x0 + width, y0 + height)
    if rotate == 270:
        return (0.0, -1.0, 1.0, 0.0, x0, y0 + height)
    return (1.0, 0.0, 0.0, 1.0, x0, y0)


def conform_to_source(
    pdf_path: str | Path,
    out_path: str | Path,
    frames: Sequence,
    sizes: Sequence[PageSize],
) -> Path:
    """Give the delta the same page geometry as the document it overlays.

    Printers place a page on paper using its boxes and ``/Rotate``. If the delta
    disagrees with the source about any of those, the two impressions cannot
    line up no matter how good the calibration is — so the delta copies them
    exactly, and its content is transformed to match.
    """
    pdf_path = Path(pdf_path)
    out_path = Path(out_path)

    if all(getattr(f, "is_simple", True) for f in frames):
        if pdf_path != out_path:
            out_path.write_bytes(pdf_path.read_bytes())
        return out_path

    with pikepdf.open(str(pdf_path)) as pdf:
        for i, page in enumerate(pdf.pages):
            if i >= len(frames):
                break
            frame = frames[i]
            if frame.is_simple:
                continue
            display = sizes[i] if i < len(sizes) else frame.display_size
            a, b, c, d, e, f = _display_to_user_matrix(frame, display)
            page.contents_add(
                f"q {a:.6f} {b:.6f} {c:.6f} {d:.6f} {e:.6f} {f:.6f} cm ".encode("ascii"),
                prepend=True,
            )
            page.contents_add(b" Q")
            page.MediaBox = list(frame.media)
            page.CropBox = list(frame.crop)
            if frame.rotate:
                page.Rotate = frame.rotate
            elif "/Rotate" in page:
                del page.Rotate
        pdf.save(str(out_path))
    return out_path


def preview_page(
    diff: PageDiff,
    old_gray: np.ndarray,
    max_width: int = 1000,
    highlight: tuple[int, int, int] = (214, 51, 51),
) -> Image.Image:
    """A proof image: the existing sheet faded back, new ink in red.

    This is the thing that actually stops wasted paper — you see where the new
    ink will land relative to what is already printed before committing a sheet
    to the tray.
    """
    h, w = diff.added.shape
    base = old_gray[:h, :w].astype(np.float32)
    faded = 255.0 - (255.0 - base) * 0.28  # keep the original as a ghost
    canvas_rgb = np.repeat(faded[:, :, None], 3, axis=2)

    ys, xs = np.nonzero(diff.added)
    if ys.size:
        canvas_rgb[ys, xs] = np.array(highlight, dtype=np.float32)

    ys, xs = np.nonzero(diff.removed)
    if ys.size:
        canvas_rgb[ys, xs] = np.array((120, 160, 255), dtype=np.float32)

    image = Image.fromarray(canvas_rgb.astype(np.uint8), mode="RGB")
    if image.width > max_width:
        ratio = max_width / image.width
        image = image.resize(
            (max_width, max(1, int(image.height * ratio))), Image.LANCZOS
        )
    return image
