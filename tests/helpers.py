"""Test fixtures.

Documents are built at absolute positions wherever possible: a test that says
"the addition is at 40 mm, 100 mm" should fail if the geometry drifts, not if a
font metric changed.
"""

from __future__ import annotations

import zipfile
from pathlib import Path

import numpy as np
import pytest
from reportlab.pdfgen import canvas

from onionskin.geometry import PageSize, mm_to_pt
from onionskin.render import find_soffice

A4 = PageSize(210.0, 297.0)

requires_soffice = pytest.mark.skipif(
    find_soffice() is None, reason="LibreOffice is not installed"
)


def make_pdf(
    path: Path,
    items: list[tuple[float, float, str]],
    page: PageSize = A4,
    pages: int = 1,
    font_size: float = 12.0,
) -> Path:
    """A PDF with text drawn at exact page-space (mm from top-left) positions."""
    pdf = canvas.Canvas(str(path), pagesize=(page.width_pt, page.height_pt))
    for page_index in range(pages):
        pdf.setFont("Helvetica", font_size)
        for x_mm, y_mm, text in items:
            pdf.drawString(
                mm_to_pt(x_mm), page.height_pt - mm_to_pt(y_mm), text
            )
        pdf.showPage()
    pdf.save()
    return path


DOCX_CONTENT_TYPES = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"""

DOCX_RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"""


def make_docx(path: Path, paragraphs: list[str]) -> Path:
    """A minimal but genuinely valid .docx, so the LibreOffice path is real."""
    body = "".join(
        "<w:p><w:r><w:t xml:space='preserve'>"
        + text.replace("&", "&amp;").replace("<", "&lt;")
        + "</w:t></w:r></w:p>"
        for text in paragraphs
    )
    document = (
        "<?xml version='1.0' encoding='UTF-8' standalone='yes'?>"
        "<w:document xmlns:w='http://schemas.openxmlformats.org/wordprocessingml/2006/main'>"
        f"<w:body>{body}"
        "<w:sectPr><w:pgSz w:w='11906' w:h='16838'/>"
        "<w:pgMar w:top='1134' w:right='1134' w:bottom='1134' w:left='1134'/>"
        "</w:sectPr></w:body></w:document>"
    )
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as zf:
        zf.writestr("[Content_Types].xml", DOCX_CONTENT_TYPES)
        zf.writestr("_rels/.rels", DOCX_RELS)
        zf.writestr("word/document.xml", document)
    return path


def ink_bbox_mm(pdf_path: Path, dpi: float = 300.0, page: int = 0):
    """Bounding box of everything darker than mid-grey, in page-space mm."""
    from onionskin.render import Document

    with Document(pdf_path) as doc:
        rendered = doc.render(page, dpi)
    mask = rendered.gray < 128
    if not mask.any():
        return None
    rows = np.flatnonzero(mask.any(axis=1))
    cols = np.flatnonzero(mask.any(axis=0))
    scale = 25.4 / dpi
    return (
        float(cols[0] * scale),
        float(rows[0] * scale),
        float((cols[-1] + 1) * scale),
        float((rows[-1] + 1) * scale),
    )
