"""The core comparison: what ink is on the edited page that isn't on the original.

The whole app rests on one asymmetry. Ink that appears in the edited document
but not the original is *addable* — a printer can lay it down on the sheet you
already have. Ink that appears in the original but not the edited version is
not removable; toner does not come off paper. So the removed mask is never
printed, but it is the single most valuable signal we have: if anything
disappeared from where it used to be, the layout reflowed and the sheet in the
user's hand no longer matches the document.
"""

from __future__ import annotations

from collections import deque
from dataclasses import dataclass, field

import numpy as np
from PIL import Image, ImageFilter

from .geometry import MM_PER_INCH, PageSize, px_to_mm

#: Pixels at or below this grey level count as ink. Anti-aliased glyph edges
#: run light, so this sits well above pure black.
DEFAULT_INK_THRESHOLD = 200

#: How far a mark may move and still count as "the same mark". Absorbs
#: sub-pixel layout jitter between two renders of near-identical content.
DEFAULT_TOLERANCE_MM = 0.12

#: Additions closer together than this are reported as one region, so a word
#: comes back as one box rather than five letters.
DEFAULT_GROUP_MM = 2.0

#: Specks smaller than this are dropped as rendering noise.
DEFAULT_MIN_REGION_MM2 = 0.05


@dataclass
class Region:
    """A rectangle of changed ink, in page-space millimetres."""

    x0_mm: float
    y0_mm: float
    x1_mm: float
    y1_mm: float
    ink_mm2: float
    px_bbox: tuple[int, int, int, int]  # x0, y0, x1, y1 in pixels

    @property
    def width_mm(self) -> float:
        return self.x1_mm - self.x0_mm

    @property
    def height_mm(self) -> float:
        return self.y1_mm - self.y0_mm

    @property
    def area_mm2(self) -> float:
        return self.width_mm * self.height_mm

    def padded(self, pad_mm: float, page: PageSize) -> "Region":
        return Region(
            x0_mm=max(0.0, self.x0_mm - pad_mm),
            y0_mm=max(0.0, self.y0_mm - pad_mm),
            x1_mm=min(page.width_mm, self.x1_mm + pad_mm),
            y1_mm=min(page.height_mm, self.y1_mm + pad_mm),
            ink_mm2=self.ink_mm2,
            px_bbox=self.px_bbox,
        )

    def to_dict(self) -> dict:
        return {
            "x_mm": round(self.x0_mm, 2),
            "y_mm": round(self.y0_mm, 2),
            "width_mm": round(self.width_mm, 2),
            "height_mm": round(self.height_mm, 2),
            "ink_mm2": round(self.ink_mm2, 3),
        }


@dataclass
class PageDiff:
    """Everything learned by comparing one page of both documents."""

    index: int
    size: PageSize
    dpi: float
    added: np.ndarray = field(repr=False)
    removed: np.ndarray = field(repr=False)
    added_px: int = 0
    removed_px: int = 0
    added_regions: list[Region] = field(default_factory=list)
    removed_regions: list[Region] = field(default_factory=list)

    def release(self) -> None:
        """Drop the pixel masks, keeping every derived measurement.

        A page at 400 dpi is a 13-megapixel mask; holding two of them per page
        for a long document is the difference between a few hundred megabytes
        and a few kilobytes. Counts and regions are computed up front so
        nothing downstream needs the pixels back.
        """
        empty = np.zeros((0, 0), dtype=bool)
        self.added = empty
        self.removed = empty

    @property
    def px_area_mm2(self) -> float:
        side = MM_PER_INCH / self.dpi
        return side * side

    @property
    def added_ink_mm2(self) -> float:
        return self.added_px * self.px_area_mm2

    @property
    def removed_ink_mm2(self) -> float:
        return self.removed_px * self.px_area_mm2

    @property
    def has_additions(self) -> bool:
        return bool(self.added_regions)

    @property
    def bounds_mm(self) -> tuple[float, float, float, float] | None:
        """Bounding box of every addition on the page."""
        if not self.added_regions:
            return None
        return (
            min(r.x0_mm for r in self.added_regions),
            min(r.y0_mm for r in self.added_regions),
            max(r.x1_mm for r in self.added_regions),
            max(r.y1_mm for r in self.added_regions),
        )

    def to_dict(self) -> dict:
        return {
            "page": self.index + 1,
            "size_mm": [round(self.size.width_mm, 1), round(self.size.height_mm, 1)],
            "added_ink_mm2": round(self.added_ink_mm2, 2),
            "removed_ink_mm2": round(self.removed_ink_mm2, 2),
            "added_regions": [r.to_dict() for r in self.added_regions],
            "removed_region_count": len(self.removed_regions),
        }


def _ink_mask(gray: np.ndarray, threshold: int) -> np.ndarray:
    return gray <= threshold


def _dilate(mask: np.ndarray, radius_px: int) -> np.ndarray:
    """Grow a boolean mask by ``radius_px`` using Pillow's C max filter."""
    if radius_px <= 0:
        return mask
    size = 2 * radius_px + 1
    img = Image.fromarray((mask * 255).astype(np.uint8), mode="L")
    grown = img.filter(ImageFilter.MaxFilter(size))
    return np.asarray(grown, dtype=np.uint8) > 0


def label_regions(
    mask: np.ndarray,
    dpi: float,
    group_mm: float = DEFAULT_GROUP_MM,
    min_area_mm2: float = DEFAULT_MIN_REGION_MM2,
) -> list[Region]:
    """Group set pixels into regions with exact bounding boxes.

    Connectivity is resolved on a coarse grid of ``group_mm`` cells rather than
    per pixel: at 400 dpi an A4 page is 13 megapixels, but only ~62k cells, so
    the flood fill stays cheap while still merging the letters of a word into
    one box. Bounding boxes are then measured back at full resolution, cell by
    cell, so nothing is rounded up to the grid.
    """
    if not mask.any():
        return []

    h, w = mask.shape
    cell = max(1, int(round(group_mm * dpi / MM_PER_INCH)))
    gh = (h + cell - 1) // cell
    gw = (w + cell - 1) // cell

    padded = np.zeros((gh * cell, gw * cell), dtype=bool)
    padded[:h, :w] = mask
    grid = padded.reshape(gh, cell, gw, cell).any(axis=(1, 3))

    px_mm2 = (MM_PER_INCH / dpi) ** 2
    seen = np.zeros_like(grid)
    regions: list[Region] = []

    for gy in range(gh):
        for gx in range(gw):
            if not grid[gy, gx] or seen[gy, gx]:
                continue
            seen[gy, gx] = True
            queue = deque([(gy, gx)])
            cells: list[tuple[int, int]] = []
            while queue:
                cy, cx = queue.popleft()
                cells.append((cy, cx))
                for dy in (-1, 0, 1):
                    for dx in (-1, 0, 1):
                        ny, nx = cy + dy, cx + dx
                        if 0 <= ny < gh and 0 <= nx < gw:
                            if grid[ny, nx] and not seen[ny, nx]:
                                seen[ny, nx] = True
                                queue.append((ny, nx))

            x0 = y0 = 1 << 30
            x1 = y1 = -1
            ink_px = 0
            for cy, cx in cells:
                r0, c0 = cy * cell, cx * cell
                sub = mask[r0 : min(r0 + cell, h), c0 : min(c0 + cell, w)]
                if not sub.any():
                    continue
                rows = np.flatnonzero(sub.any(axis=1))
                cols = np.flatnonzero(sub.any(axis=0))
                y0 = min(y0, r0 + int(rows[0]))
                y1 = max(y1, r0 + int(rows[-1]) + 1)
                x0 = min(x0, c0 + int(cols[0]))
                x1 = max(x1, c0 + int(cols[-1]) + 1)
                ink_px += int(sub.sum())

            if x1 < 0:
                continue
            ink_mm2 = ink_px * px_mm2
            if ink_mm2 < min_area_mm2:
                continue
            regions.append(
                Region(
                    x0_mm=px_to_mm(x0, dpi),
                    y0_mm=px_to_mm(y0, dpi),
                    x1_mm=px_to_mm(x1, dpi),
                    y1_mm=px_to_mm(y1, dpi),
                    ink_mm2=ink_mm2,
                    px_bbox=(x0, y0, x1, y1),
                )
            )

    regions.sort(key=lambda r: (round(r.y0_mm, 1), r.x0_mm))
    return regions


def diff_page(
    old_gray: np.ndarray,
    new_gray: np.ndarray,
    size: PageSize,
    dpi: float,
    index: int = 0,
    ink_threshold: int = DEFAULT_INK_THRESHOLD,
    tolerance_mm: float = DEFAULT_TOLERANCE_MM,
    group_mm: float = DEFAULT_GROUP_MM,
    min_region_mm2: float = DEFAULT_MIN_REGION_MM2,
) -> PageDiff:
    """Compare one rendered page against another.

    Each mask is taken against the *dilated* opposite: a glyph that shifted by
    a fraction of a pixel between renders would otherwise leave a hairline
    outline in the delta, and we would print a ghost of text that is already on
    the sheet.
    """
    h = min(old_gray.shape[0], new_gray.shape[0])
    w = min(old_gray.shape[1], new_gray.shape[1])
    old_gray = old_gray[:h, :w]
    new_gray = new_gray[:h, :w]

    old_ink = _ink_mask(old_gray, ink_threshold)
    new_ink = _ink_mask(new_gray, ink_threshold)

    radius = int(round(tolerance_mm * dpi / MM_PER_INCH))
    added = new_ink & ~_dilate(old_ink, radius)
    removed = old_ink & ~_dilate(new_ink, radius)

    return PageDiff(
        index=index,
        size=size,
        dpi=dpi,
        added=added,
        removed=removed,
        added_px=int(added.sum()),
        removed_px=int(removed.sum()),
        added_regions=label_regions(added, dpi, group_mm, min_region_mm2),
        removed_regions=label_regions(removed, dpi, group_mm, min_region_mm2),
    )


def blank_diff(size: PageSize, dpi: float, index: int) -> PageDiff:
    """A diff for a page that exists in only one of the two documents."""
    w, h = size.px_size(dpi)
    empty = np.zeros((h, w), dtype=bool)
    return PageDiff(index=index, size=size, dpi=dpi, added=empty, removed=empty)
