"""Units, page geometry, and the similarity transform used for printer calibration.

Two coordinate systems appear throughout Onionskin:

*page space*
    Millimetres from the top-left corner of the sheet, x right, y **down**.
    This is how a person measures a printed page with a ruler, so every
    user-facing number (region positions, calibration offsets, margins) is in
    page space.

*PDF space*
    Points from the bottom-left corner, y **up**. Only the PDF writers touch it.

``Similarity`` is defined in page space; :meth:`Similarity.to_pdf_matrix`
handles the flip.
"""

from __future__ import annotations

import math
from dataclasses import dataclass
from typing import Iterable, Sequence

MM_PER_INCH = 25.4
PT_PER_INCH = 72.0
PT_PER_MM = PT_PER_INCH / MM_PER_INCH


def mm_to_pt(mm: float) -> float:
    return mm * PT_PER_MM


def pt_to_mm(pt: float) -> float:
    return pt / PT_PER_MM


def mm_to_px(mm: float, dpi: float) -> float:
    return mm * dpi / MM_PER_INCH


def px_to_mm(px: float, dpi: float) -> float:
    return px * MM_PER_INCH / dpi


@dataclass(frozen=True)
class PageSize:
    """A page's physical size in millimetres."""

    width_mm: float
    height_mm: float

    @classmethod
    def from_pt(cls, width_pt: float, height_pt: float) -> "PageSize":
        return cls(pt_to_mm(width_pt), pt_to_mm(height_pt))

    @property
    def width_pt(self) -> float:
        return mm_to_pt(self.width_mm)

    @property
    def height_pt(self) -> float:
        return mm_to_pt(self.height_mm)

    @property
    def center_mm(self) -> tuple[float, float]:
        return (self.width_mm / 2.0, self.height_mm / 2.0)

    def px_size(self, dpi: float) -> tuple[int, int]:
        """Raster dimensions at ``dpi``, rounded consistently for old and new."""
        return (
            max(1, int(round(mm_to_px(self.width_mm, dpi)))),
            max(1, int(round(mm_to_px(self.height_mm, dpi)))),
        )

    def matches(self, other: "PageSize", tol_mm: float = 0.5) -> bool:
        return (
            abs(self.width_mm - other.width_mm) <= tol_mm
            and abs(self.height_mm - other.height_mm) <= tol_mm
        )

    def describe(self) -> str:
        name = _NAMED_SIZES.get(
            (round(self.width_mm), round(self.height_mm))
        )
        base = f"{self.width_mm:.1f}×{self.height_mm:.1f} mm"
        return f"{name} ({base})" if name else base


_NAMED_SIZES = {
    (210, 297): "A4",
    (297, 210): "A4 landscape",
    (216, 279): "Letter",
    (279, 216): "Letter landscape",
    (216, 356): "Legal",
    (148, 210): "A5",
    (297, 420): "A3",
}


@dataclass(frozen=True)
class Similarity:
    """A rigid-plus-uniform-scale transform of the printed page.

    This is the full space of registration error a sheet-fed printer can
    introduce on a second pass: the paper can land shifted, very slightly
    rotated, and the imaging can be marginally over- or under-scaled. Shear is
    not physically reachable, so it is deliberately not modelled.

    The transform is applied *about the centre of the page*::

        p' = centre + scale * R(rotation_deg) * (p - centre) + (dx_mm, dy_mm)

    ``rotation_deg`` is positive **clockwise** as you look at the sheet, which
    is what people mean when they say "it came out slightly rotated right".
    """

    dx_mm: float = 0.0
    dy_mm: float = 0.0
    rotation_deg: float = 0.0
    scale: float = 1.0

    @classmethod
    def identity(cls) -> "Similarity":
        return cls()

    @property
    def is_identity(self) -> bool:
        return (
            abs(self.dx_mm) < 1e-9
            and abs(self.dy_mm) < 1e-9
            and abs(self.rotation_deg) < 1e-9
            and abs(self.scale - 1.0) < 1e-12
        )

    def apply(self, point_mm: Sequence[float], page: PageSize) -> tuple[float, float]:
        cx, cy = page.center_mm
        theta = math.radians(self.rotation_deg)
        cos_t, sin_t = math.cos(theta), math.sin(theta)
        ax, ay = point_mm[0] - cx, point_mm[1] - cy
        # y-down space: positive theta rotates +x toward +y, i.e. clockwise.
        rx = cos_t * ax - sin_t * ay
        ry = sin_t * ax + cos_t * ay
        return (cx + self.scale * rx + self.dx_mm, cy + self.scale * ry + self.dy_mm)

    def inverse(self) -> "Similarity":
        """The transform that undoes this one.

        Derived from ``p = c + sR(q - c) + t``: solving for ``q`` gives scale
        ``1/s``, rotation ``-theta`` and translation ``-(1/s) R^-1 t``.
        """
        inv_scale = 1.0 / self.scale
        theta = math.radians(-self.rotation_deg)
        cos_t, sin_t = math.cos(theta), math.sin(theta)
        tx, ty = self.dx_mm, self.dy_mm
        rx = cos_t * tx - sin_t * ty
        ry = sin_t * tx + cos_t * ty
        return Similarity(
            dx_mm=-inv_scale * rx,
            dy_mm=-inv_scale * ry,
            rotation_deg=-self.rotation_deg,
            scale=inv_scale,
        )

    def to_pdf_matrix(self, page: PageSize):
        """Build the equivalent PDF content-stream matrix.

        PDF space is y-up, so a clockwise page-space rotation becomes a
        counter-clockwise PDF rotation and the y translation flips sign.
        """
        import pikepdf

        cx_pt = mm_to_pt(page.center_mm[0])
        cy_pt = page.height_pt - mm_to_pt(page.center_mm[1])
        dx_pt = mm_to_pt(self.dx_mm)
        dy_pt = -mm_to_pt(self.dy_mm)

        # pikepdf chains right-to-left (standard math order): the rightmost
        # call is applied to the point first.
        return (
            pikepdf.Matrix()
            .translated(cx_pt + dx_pt, cy_pt + dy_pt)
            .rotated(-self.rotation_deg)
            .scaled(self.scale, self.scale)
            .translated(-cx_pt, -cy_pt)
        )

    def describe(self) -> str:
        if self.is_identity:
            return "identity (no correction)"
        parts = []
        if abs(self.dx_mm) >= 5e-4 or abs(self.dy_mm) >= 5e-4:
            parts.append(f"shift {self.dx_mm:+.2f}, {self.dy_mm:+.2f} mm")
        if abs(self.rotation_deg) >= 5e-4:
            parts.append(f"rotate {self.rotation_deg:+.3f}° cw")
        if abs(self.scale - 1.0) >= 5e-7:
            parts.append(f"scale {self.scale:.5f} ({(self.scale - 1) * 100:+.3f}%)")
        return ", ".join(parts) or "identity (no correction)"

    def to_dict(self) -> dict:
        return {
            "dx_mm": self.dx_mm,
            "dy_mm": self.dy_mm,
            "rotation_deg": self.rotation_deg,
            "scale": self.scale,
        }

    @classmethod
    def from_dict(cls, data: dict) -> "Similarity":
        return cls(
            dx_mm=float(data.get("dx_mm", 0.0)),
            dy_mm=float(data.get("dy_mm", 0.0)),
            rotation_deg=float(data.get("rotation_deg", 0.0)),
            scale=float(data.get("scale", 1.0)),
        )


@dataclass
class SimilarityFit:
    transform: Similarity
    rms_residual_mm: float
    max_residual_mm: float
    n_points: int


def solve_similarity(
    nominal: Iterable[Sequence[float]],
    observed: Iterable[Sequence[float]],
    page: PageSize,
) -> SimilarityFit:
    """Least-squares fit of a :class:`Similarity` mapping ``nominal -> observed``.

    Uses the complex-number formulation: in 2D a scale-plus-rotation is just
    multiplication by a complex number, so the least-squares solution has a
    closed form and needs no iteration.

    ``nominal`` are the coordinates Onionskin asked the printer for;
    ``observed`` are where the ink actually landed, both in page-space mm.
    """
    src = [complex(p[0], p[1]) for p in nominal]
    dst = [complex(p[0], p[1]) for p in observed]
    if len(src) != len(dst):
        raise ValueError("nominal and observed must have the same length")
    n = len(src)
    if n < 2:
        raise ValueError(
            "need at least 2 measured points to solve for shift, rotation and scale"
        )

    cx, cy = page.center_mm
    centre = complex(cx, cy)
    a = [z - centre for z in src]
    b = [z - centre for z in dst]

    mean_a = sum(a) / n
    mean_b = sum(b) / n
    num = sum((za - mean_a).conjugate() * (zb - mean_b) for za, zb in zip(a, b))
    den = sum(abs(za - mean_a) ** 2 for za in a)
    if den < 1e-12:
        raise ValueError(
            "measured points are coincident; spread them across the page"
        )
    w = num / den  # complex: |w| is scale, arg(w) is rotation
    t = mean_b - w * mean_a

    residuals = [abs(w * za + t - zb) for za, zb in zip(a, b)]
    rms = math.sqrt(sum(r * r for r in residuals) / n)

    return SimilarityFit(
        transform=Similarity(
            dx_mm=t.real,
            dy_mm=t.imag,
            rotation_deg=math.degrees(math.atan2(w.imag, w.real)),
            scale=abs(w),
        ),
        rms_residual_mm=rms,
        max_residual_mm=max(residuals),
        n_points=n,
    )
