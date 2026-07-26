"""Turning source documents into PDFs, and PDFs into rasters.

Word documents go through LibreOffice in headless mode. That matters more than
it looks: both the original and the edited file must be laid out by the *same*
engine at the *same* version, or the two renders will disagree about kerning
and line breaks and every glyph on the page will show up as a difference.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import tempfile
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Iterator

import numpy as np
import pypdfium2 as pdfium
from PIL import Image

from .geometry import PageSize

#: Extensions LibreOffice will convert for us. Anything else is rejected up
#: front rather than failing deep inside a subprocess.
CONVERTIBLE = {
    ".doc", ".docx", ".docm", ".dot", ".dotx",
    ".odt", ".ott", ".fodt",
    ".rtf", ".txt", ".html", ".htm",
}
PASSTHROUGH = {".pdf"}
SUPPORTED = CONVERTIBLE | PASSTHROUGH


class ConversionError(RuntimeError):
    """LibreOffice could not produce a PDF from the input."""


class DocumentError(ValueError):
    """The input document is unusable for a delta."""


def find_soffice() -> str | None:
    env = os.environ.get("ONIONSKIN_SOFFICE")
    if env and Path(env).exists():
        return env
    for name in ("soffice", "libreoffice"):
        found = shutil.which(name)
        if found:
            return found
    for candidate in (
        "/Applications/LibreOffice.app/Contents/MacOS/soffice",
        "/usr/lib/libreoffice/program/soffice",
        "C:\\Program Files\\LibreOffice\\program\\soffice.exe",
    ):
        if Path(candidate).exists():
            return candidate
    return None


def to_pdf(source: str | Path, workdir: str | Path, timeout: int = 180) -> Path:
    """Return a PDF for ``source``, converting via LibreOffice if needed."""
    source = Path(source)
    if not source.is_file():
        raise DocumentError(f"no such file: {source}")

    suffix = source.suffix.lower()
    if suffix in PASSTHROUGH:
        return source
    if suffix not in CONVERTIBLE:
        raise DocumentError(
            f"unsupported file type '{suffix}'. "
            f"Supported: {', '.join(sorted(SUPPORTED))}"
        )

    soffice = find_soffice()
    if soffice is None:
        raise ConversionError(
            "LibreOffice was not found, so Word documents cannot be converted.\n"
            "Install it (https://www.libreoffice.org/download/) or set "
            "ONIONSKIN_SOFFICE to the soffice binary.\n"
            "You can also export both documents to PDF yourself and pass those."
        )

    workdir = Path(workdir)
    workdir.mkdir(parents=True, exist_ok=True)
    # A private profile per conversion: LibreOffice refuses to run two headless
    # instances against one profile, which would otherwise break concurrent
    # requests in the web app.
    profile = workdir / f"lo-profile-{uuid.uuid4().hex[:8]}"
    outdir = workdir / f"lo-out-{uuid.uuid4().hex[:8]}"
    outdir.mkdir(parents=True, exist_ok=True)

    cmd = [
        soffice,
        # as_uri() gets this right on Windows (file:///C:/...) where a bare
        # f"file://{path}" does not, and percent-encodes spaces.
        f"-env:UserInstallation={profile.absolute().as_uri()}",
        "--headless",
        "--norestore",
        "--invisible",
        "--nolockcheck",
        "--convert-to", "pdf:writer_pdf_Export",
        "--outdir", str(outdir),
        str(source.resolve()),
    ]
    try:
        proc = subprocess.run(
            cmd, capture_output=True, text=True, timeout=timeout, check=False
        )
    except subprocess.TimeoutExpired as exc:
        raise ConversionError(
            f"LibreOffice timed out after {timeout}s converting {source.name}"
        ) from exc
    finally:
        shutil.rmtree(profile, ignore_errors=True)

    produced = sorted(outdir.glob("*.pdf"))
    if not produced:
        detail = (proc.stderr or proc.stdout or "").strip()[:500]
        raise ConversionError(
            f"LibreOffice produced no PDF for {source.name}"
            + (f"\n{detail}" if detail else "")
        )
    return produced[0]


@dataclass(frozen=True)
class PageFrame:
    """Where a page's content actually sits, in PDF user space.

    A page is not always the simple case of "a box starting at (0,0), the right
    way up". It can have a media box with a non-zero origin, a crop box smaller
    than the media box, and a ``/Rotate`` that turns it a quarter turn for
    display. All three are common in the wild — phone scans and anything that
    has been through a PDF editor — and all three move where ink lands on the
    physical sheet.

    Onionskin renders and diffs in *display space*: the page as you see it,
    origin at the top-left, already cropped and rotated. The delta must then be
    written back into the source's own frame, or it will not line up with the
    sheet in the tray.
    """

    media: tuple[float, float, float, float]
    crop: tuple[float, float, float, float]
    rotate: int

    @property
    def crop_size_pt(self) -> tuple[float, float]:
        return (self.crop[2] - self.crop[0], self.crop[3] - self.crop[1])

    @property
    def display_size(self) -> PageSize:
        """The page as rendered: cropped, and turned if ``/Rotate`` says so."""
        width, height = self.crop_size_pt
        if self.rotate in (90, 270):
            width, height = height, width
        return PageSize.from_pt(width, height)

    @property
    def is_simple(self) -> bool:
        """True when display space and user space are the same thing."""
        return (
            self.rotate == 0
            and abs(self.crop[0]) < 1e-6
            and abs(self.crop[1]) < 1e-6
            and all(abs(c - m) < 1e-6 for c, m in zip(self.crop, self.media))
        )

    def describe(self) -> str:
        bits = []
        if self.rotate:
            bits.append(f"rotated {self.rotate}°")
        if abs(self.crop[0]) > 1e-6 or abs(self.crop[1]) > 1e-6:
            bits.append(f"origin at ({self.crop[0]:.1f}, {self.crop[1]:.1f}) pt")
        if any(abs(c - m) > 1e-6 for c, m in zip(self.crop, self.media)):
            bits.append("cropped")
        return ", ".join(bits) or "standard"


def _unreadable(path: Path, exc: Exception) -> str:
    """Turn a library's complaint into something a person can act on."""
    detail = str(exc).lower()
    if "password" in detail or "encrypt" in detail:
        return (
            f"{path.name} is password-protected. Open it in a PDF reader, save an "
            "unprotected copy, and use that."
        )
    if path.exists() and path.stat().st_size == 0:
        return f"{path.name} is empty (0 bytes)."
    return (
        f"{path.name} could not be opened as a PDF. It may be damaged, incomplete, "
        f"or not really a PDF.\n    ({exc})"
    )


def _read_frames(pdf_path: str | Path) -> list[PageFrame]:
    """Read every page's box geometry, resolving inherited attributes."""
    import pikepdf

    frames: list[PageFrame] = []
    with pikepdf.open(str(pdf_path)) as pdf:
        for page in pdf.pages:
            handle = pikepdf.Page(page)
            media = tuple(float(v) for v in handle.mediabox)
            try:
                crop = tuple(float(v) for v in handle.cropbox)
            except Exception:
                crop = media
            rotate = int(handle.rotation or 0) % 360
            if rotate % 90:
                raise DocumentError(
                    f"page {len(frames) + 1} is rotated {rotate}°, which the PDF "
                    "specification does not allow (it must be a multiple of 90)"
                )
            # A crop box is only meaningful where it intersects the media box.
            crop = (
                max(crop[0], media[0]),
                max(crop[1], media[1]),
                min(crop[2], media[2]),
                min(crop[3], media[3]),
            )
            if crop[2] - crop[0] <= 0 or crop[3] - crop[1] <= 0:
                crop = media
            frames.append(PageFrame(media=media, crop=crop, rotate=rotate))
    return frames


def _fit(array: np.ndarray, height: int, width: int, fill: int) -> np.ndarray:
    """Crop or pad an image to exactly ``height`` × ``width``, paper-side out."""
    cropped = array[:height, :width]
    if cropped.shape[0] == height and cropped.shape[1] == width:
        return cropped

    shape = (height, width) + array.shape[2:]
    padded = np.full(shape, fill, dtype=array.dtype)
    padded[: cropped.shape[0], : cropped.shape[1]] = cropped
    return padded


@dataclass
class RenderedPage:
    index: int
    size: PageSize
    rgb: np.ndarray  # (h, w, 3) uint8
    gray: np.ndarray  # (h, w) uint8

    @property
    def shape(self) -> tuple[int, int]:
        return self.gray.shape  # type: ignore[return-value]


class Document:
    """A PDF opened for measurement and rasterising."""

    def __init__(self, pdf_path: str | Path):
        self.path = Path(pdf_path)
        # pdfium raises its own exception type for anything it cannot open —
        # encrypted, truncated, empty, or not a PDF at all. Left unwrapped it
        # reaches a non-technical user as a Python traceback, which is exactly
        # what the rest of the app is careful never to do.
        try:
            self._doc = pdfium.PdfDocument(str(self.path))
        except Exception as exc:
            raise DocumentError(_unreadable(self.path, exc)) from exc
        try:
            self.frames: list[PageFrame] = _read_frames(self.path)
        except DocumentError:
            self.close()
            raise
        except Exception as exc:
            self.close()
            raise DocumentError(_unreadable(self.path, exc)) from exc
        self.page_sizes: list[PageSize] = [f.display_size for f in self.frames]
        if not self.page_sizes:
            raise DocumentError(f"{self.path.name} has no pages")
        if len(self._doc) != len(self.frames):
            raise DocumentError(
                f"{self.path.name} is inconsistent: pdfium sees {len(self._doc)} "
                f"page(s), the page tree has {len(self.frames)}"
            )

    def __len__(self) -> int:
        return len(self.page_sizes)

    def render(self, index: int, dpi: float) -> RenderedPage:
        size = self.page_sizes[index]
        page = self._doc[index]
        try:
            bitmap = page.render(scale=dpi / 72.0, draw_annots=True)
            image = bitmap.to_pil().convert("RGB")
        finally:
            page.close()

        rgb = np.asarray(image, dtype=np.uint8)
        gray = np.asarray(image.convert("L"), dtype=np.uint8)

        target_w, target_h = size.px_size(dpi)
        if (rgb.shape[1], rgb.shape[0]) != (target_w, target_h):
            # pdfium rounds each axis independently, so a page can come back a
            # pixel off. Both documents must land on identical rasters for the
            # diff to be a straight array comparison — but the difference is
            # never more than a pixel, so crop or pad rather than resample.
            # Resampling a 13-megapixel page to move it one pixel was costing
            # a fifth of the total run time and blurring every glyph edge.
            rgb = _fit(rgb, target_h, target_w, fill=255)
            gray = _fit(gray, target_h, target_w, fill=255)

        return RenderedPage(index=index, size=size, rgb=rgb, gray=gray)

    def pages(self, dpi: float) -> Iterator[RenderedPage]:
        for i in range(len(self)):
            yield self.render(i, dpi)

    def close(self) -> None:
        try:
            self._doc.close()
        except Exception:  # pragma: no cover - pdfium already torn down
            pass

    def __enter__(self) -> "Document":
        return self

    def __exit__(self, *exc) -> None:
        self.close()


class Workspace:
    """A scratch directory that cleans itself up."""

    def __init__(self, keep: bool = False):
        self.keep = keep
        self.path = Path(tempfile.mkdtemp(prefix="onionskin-"))

    def __enter__(self) -> Path:
        return self.path

    def __exit__(self, *exc) -> None:
        if not self.keep:
            shutil.rmtree(self.path, ignore_errors=True)
