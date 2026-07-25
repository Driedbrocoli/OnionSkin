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
        f"-env:UserInstallation=file://{profile}",
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
        self._doc = pdfium.PdfDocument(str(self.path))
        self.page_sizes: list[PageSize] = []
        for i in range(len(self._doc)):
            page = self._doc[i]
            width_pt, height_pt = page.get_size()
            self.page_sizes.append(PageSize.from_pt(width_pt, height_pt))
            page.close()
        if not self.page_sizes:
            raise DocumentError(f"{self.path.name} has no pages")

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

        target = size.px_size(dpi)
        if image.size != target:
            # pdfium rounds independently per axis; force both documents onto
            # identical rasters so the diff is a straight array comparison.
            image = image.resize(target, Image.LANCZOS)

        rgb = np.asarray(image, dtype=np.uint8)
        gray = np.asarray(image.convert("L"), dtype=np.uint8)
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
