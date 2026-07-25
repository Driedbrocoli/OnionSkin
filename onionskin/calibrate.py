"""Per-printer registration calibration.

Uncalibrated, a second pass through a sheet-fed printer lands within about
±2 mm — fine for a signature, useless for filling a pre-printed box. The fix
needs no scanner:

1. ``onionskin calibrate target`` writes a page of crosshairs at known
   positions, each with a fine ruler running right and down from it.
2. Print it on blank paper at 100%.
3. Put that same sheet back in the tray and print the *same file again*.
4. Every crosshair now has two impressions. Read the offset of the second from
   the first against the printed ruler — that offset *is* the error the printer
   will apply to your delta.
5. ``onionskin calibrate solve`` fits shift, rotation and scale to those
   readings and stores the profile.

Deltas then get the inverse of that transform, so the ink lands where the
document says it should.
"""

from __future__ import annotations

import json
import os
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Sequence

from reportlab.lib import colors
from reportlab.pdfgen import canvas

from .geometry import (
    PageSize,
    Similarity,
    SimilarityFit,
    mm_to_pt,
    solve_similarity,
)

A4 = PageSize(210.0, 297.0)
LETTER = PageSize(215.9, 279.4)

#: Every paper size Onionskin knows by name. A printer Onionskin has never
#: heard of is still fine — pass a size as ``WIDTHxHEIGHT`` in millimetres.
PAGE_PRESETS = {
    "a3": PageSize(297.0, 420.0),
    "a4": A4,
    "a5": PageSize(148.0, 210.0),
    "a6": PageSize(105.0, 148.0),
    "b5": PageSize(176.0, 250.0),
    "letter": LETTER,
    "legal": PageSize(215.9, 355.6),
    "tabloid": PageSize(279.4, 431.8),
    "executive": PageSize(184.15, 266.7),
    "statement": PageSize(139.7, 215.9),
}

#: The smallest sheet the target still fits on, with room for the fiducials.
MIN_TARGET_MM = 90.0


def parse_page(spec: str) -> PageSize:
    """Resolve a page name, or a custom ``WIDTHxHEIGHT`` in millimetres.

    Anyone whose printer takes a size not in the list — a photo tray, an index
    card, A0 — can still calibrate it, which is the whole point of accepting
    arbitrary dimensions rather than a fixed menu.
    """
    text = (spec or "").strip().lower()
    if text in PAGE_PRESETS:
        return PAGE_PRESETS[text]

    separator = "x" if "x" in text else ("*" if "*" in text else None)
    if separator:
        parts = text.split(separator)
        if len(parts) == 2:
            try:
                width, height = float(parts[0]), float(parts[1])
            except ValueError:
                width = height = 0.0
            if width > 0 and height > 0:
                if width < MIN_TARGET_MM or height < MIN_TARGET_MM:
                    raise ValueError(
                        f"{width:g}×{height:g} mm is too small to calibrate on — "
                        f"the target needs at least {MIN_TARGET_MM:g} mm each way. "
                        "Calibrate on a larger sheet from the same printer instead; "
                        "the correction is a property of the paper path, not the page."
                    )
                if width > 2000 or height > 2000:
                    raise ValueError(f"{width:g}×{height:g} mm is not a paper size")
                return PageSize(width, height)

    raise ValueError(
        f"unknown page size '{spec}'. Use one of "
        f"{', '.join(sorted(PAGE_PRESETS))}, or a custom size like '210x297' (mm)."
    )


#: How far the measuring rulers extend from each crosshair.
RULER_REACH_MM = 4.0
RULER_STEP_MM = 0.25


def home_dir() -> Path:
    env = os.environ.get("ONIONSKIN_HOME")
    return Path(env) if env else Path.home() / ".onionskin"


def profiles_dir() -> Path:
    path = home_dir() / "profiles"
    path.mkdir(parents=True, exist_ok=True, mode=0o700)
    try:
        path.chmod(0o700)
        path.parent.chmod(0o700)
    except (OSError, NotImplementedError):
        pass
    return path


@dataclass
class Profile:
    """A stored measurement of one printer's second-pass registration error."""

    name: str
    #: The error the printer introduces. Deltas get its inverse.
    error: Similarity
    page: PageSize = field(default_factory=lambda: A4)
    rms_residual_mm: float | None = None
    max_residual_mm: float | None = None
    n_points: int = 0
    created: float = field(default_factory=time.time)
    notes: str = ""

    @property
    def correction(self) -> Similarity:
        return self.error.inverse()

    def to_dict(self) -> dict:
        return {
            "name": self.name,
            "error": self.error.to_dict(),
            "page_mm": [self.page.width_mm, self.page.height_mm],
            "rms_residual_mm": self.rms_residual_mm,
            "max_residual_mm": self.max_residual_mm,
            "n_points": self.n_points,
            "created": self.created,
            "notes": self.notes,
        }

    @classmethod
    def from_dict(cls, data: dict) -> "Profile":
        page = data.get("page_mm") or [A4.width_mm, A4.height_mm]
        return cls(
            name=data["name"],
            error=Similarity.from_dict(data.get("error", {})),
            page=PageSize(float(page[0]), float(page[1])),
            rms_residual_mm=data.get("rms_residual_mm"),
            max_residual_mm=data.get("max_residual_mm"),
            n_points=int(data.get("n_points", 0)),
            created=float(data.get("created", time.time())),
            notes=data.get("notes", ""),
        )

    def describe(self) -> str:
        lines = [
            f"profile '{self.name}'",
            f"  printer error : {self.error.describe()}",
            f"  correction    : {self.correction.describe()}",
            f"  page          : {self.page.describe()}",
        ]
        if self.rms_residual_mm is not None:
            lines.append(
                f"  fit           : {self.n_points} points, "
                f"rms {self.rms_residual_mm:.3f} mm, "
                f"max {self.max_residual_mm:.3f} mm"
            )
        if self.notes:
            lines.append(f"  notes         : {self.notes}")
        return "\n".join(lines)


def profile_path(name: str) -> Path:
    safe = "".join(c for c in name if c.isalnum() or c in "-_") or "default"
    return profiles_dir() / f"{safe}.json"


def save_profile(profile: Profile) -> Path:
    path = profile_path(profile.name)
    path.write_text(json.dumps(profile.to_dict(), indent=2), encoding="utf-8")
    try:
        path.chmod(0o600)
    except (OSError, NotImplementedError):
        pass
    return path


def load_profile(name: str) -> Profile:
    path = profile_path(name)
    if not path.is_file():
        available = ", ".join(p.name for p in list_profiles()) or "none"
        raise FileNotFoundError(
            f"no calibration profile '{name}' (available: {available})"
        )
    return Profile.from_dict(json.loads(path.read_text(encoding="utf-8")))


def list_profiles() -> list[Profile]:
    out = []
    for path in sorted(profiles_dir().glob("*.json")):
        try:
            out.append(Profile.from_dict(json.loads(path.read_text(encoding="utf-8"))))
        except (json.JSONDecodeError, KeyError):
            continue
    return out


def delete_profile(name: str) -> bool:
    path = profile_path(name)
    if path.is_file():
        path.unlink()
        return True
    return False


def fiducials(page: PageSize, inset_mm: float = 25.0) -> list[tuple[float, float]]:
    """Crosshair positions: four corners plus centre.

    Spread matters. Rotation and scale are only observable from points far
    apart, so clustering them would leave those terms unconstrained.
    """
    w, h = page.width_mm, page.height_mm
    return [
        (inset_mm, inset_mm),
        (w - inset_mm, inset_mm),
        (inset_mm, h - inset_mm),
        (w - inset_mm, h - inset_mm),
        (w / 2.0, h / 2.0),
    ]


#: How far each scale sits from the crosshair centre. Far enough that the two
#: scales cannot overlap each other, close enough that the second impression's
#: arms still reach across them.
SCALE_OFFSET_MM = 7.0

#: Crosshair arms must be longer than SCALE_OFFSET_MM, or the second
#: impression's arms would never reach the scale they are read against.
ARM_MM = 12.0


def _draw_scales(pdf: canvas.Canvas, x_mm: float, y_mm: float, page: PageSize) -> None:
    """Draw the two measuring scales for one crosshair.

    The x scale sits *below* the fiducial and the y scale to its *left*, in
    strips that cannot overlap. You read an offset where the second
    impression's arm crosses a scale: its vertical arm cuts the x scale, its
    horizontal arm cuts the y scale.
    """

    def to_pt(mx: float, my: float) -> tuple[float, float]:
        return mm_to_pt(mx), page.height_pt - mm_to_pt(my)

    steps = int(RULER_REACH_MM / RULER_STEP_MM)

    def tick_length(offset: float) -> float:
        if abs(offset - round(offset)) < 1e-9:
            return 1.7  # whole millimetre
        if abs(offset * 2 - round(offset * 2)) < 1e-9:
            return 1.1  # half
        return 0.6

    # --- x scale: a baseline below the fiducial, ticks hanging down from it
    pdf.setLineWidth(0.25)
    bx0, by = to_pt(x_mm - RULER_REACH_MM, y_mm + SCALE_OFFSET_MM)
    bx1, _ = to_pt(x_mm + RULER_REACH_MM, y_mm + SCALE_OFFSET_MM)
    pdf.line(bx0, by, bx1, by)

    pdf.setLineWidth(0.2)
    for i in range(-steps, steps + 1):
        offset = i * RULER_STEP_MM
        sx, sy = to_pt(x_mm + offset, y_mm + SCALE_OFFSET_MM)
        pdf.line(sx, sy, sx, sy - mm_to_pt(tick_length(offset)))

    pdf.setFont("Helvetica", 3.6)
    for mark in (-4, -2, 2, 4):
        sx, sy = to_pt(x_mm + mark, y_mm + SCALE_OFFSET_MM)
        pdf.drawCentredString(sx, sy - mm_to_pt(3.6), f"{mark:+d}")
    sx, sy = to_pt(x_mm, y_mm + SCALE_OFFSET_MM)
    pdf.drawCentredString(sx, sy - mm_to_pt(3.6), "x")

    # --- y scale: a baseline left of the fiducial, ticks running left from it
    pdf.setLineWidth(0.25)
    bx, by0 = to_pt(x_mm - SCALE_OFFSET_MM, y_mm - RULER_REACH_MM)
    _, by1 = to_pt(x_mm - SCALE_OFFSET_MM, y_mm + RULER_REACH_MM)
    pdf.line(bx, by0, bx, by1)

    pdf.setLineWidth(0.2)
    for i in range(-steps, steps + 1):
        offset = i * RULER_STEP_MM
        sx, sy = to_pt(x_mm - SCALE_OFFSET_MM, y_mm + offset)
        pdf.line(sx, sy, sx - mm_to_pt(tick_length(offset)), sy)

    for mark in (-4, -2, 2, 4):
        sx, sy = to_pt(x_mm - SCALE_OFFSET_MM, y_mm + mark)
        pdf.drawRightString(sx - mm_to_pt(2.4), sy - mm_to_pt(0.5), f"{mark:+d}")
    sx, sy = to_pt(x_mm - SCALE_OFFSET_MM, y_mm)
    pdf.drawRightString(sx - mm_to_pt(2.4), sy - mm_to_pt(0.5), "y")


def default_inset(page: PageSize) -> float:
    """How far in to place the corner crosshairs on a given sheet.

    25 mm suits office paper, but a small sheet needs the fiducials pulled in
    proportionally or their scales would hang off the edge. Spread still has to
    be as wide as the sheet allows, since rotation and scale are only
    observable from points far apart.
    """
    shortest = min(page.width_mm, page.height_mm)
    return max(15.0, min(25.0, shortest / 4.0))


def make_target(
    out_path: str | Path,
    page: PageSize = A4,
    inset_mm: float | None = None,
) -> Path:
    """Write the two-pass calibration target."""
    if inset_mm is None:
        inset_mm = default_inset(page)
    out_path = Path(out_path)
    out_path.parent.mkdir(parents=True, exist_ok=True)

    pdf = canvas.Canvas(str(out_path), pagesize=(page.width_pt, page.height_pt))
    pdf.setTitle("Onionskin calibration target")
    pdf.setProducer("Onionskin")

    def to_pt(mx: float, my: float) -> tuple[float, float]:
        return mm_to_pt(mx), page.height_pt - mm_to_pt(my)

    points = fiducials(page, inset_mm)
    pdf.setStrokeColor(colors.black)

    for idx, (x_mm, y_mm) in enumerate(points, start=1):
        cx, cy = to_pt(x_mm, y_mm)
        pdf.setLineWidth(0.35)
        arm = mm_to_pt(ARM_MM)
        gap = mm_to_pt(0.8)
        pdf.line(cx - arm, cy, cx - gap, cy)
        pdf.line(cx + gap, cy, cx + arm, cy)
        pdf.line(cx, cy - arm, cx, cy - gap)
        pdf.line(cx, cy + gap, cx, cy + arm)
        pdf.circle(cx, cy, mm_to_pt(1.6), stroke=1, fill=0)

        _draw_scales(pdf, x_mm, y_mm, page)

        # Above the crosshair: the x scale is below it and the y scale left,
        # so this is the one side left free.
        pdf.setFont("Helvetica-Bold", 6)
        pdf.drawCentredString(
            cx, cy + mm_to_pt(ARM_MM + 2.5), f"P{idx}   {x_mm:g}, {y_mm:g}"
        )

    # Instructions, kept clear of the fiducials and their rulers. A small sheet
    # has no room for them, and a target you can read beats a target with
    # printed prose over the crosshairs.
    text_x, text_y = to_pt(page.width_mm / 2, page.height_mm / 2 + 22)
    if page.width_mm < 170 or page.height_mm < 230:
        pdf.setFont("Helvetica", 5.5)
        pdf.drawCentredString(
            text_x,
            text_y - mm_to_pt(2),
            "Onionskin target — print at 100%, re-feed the sheet, print again.",
        )
        pdf.showPage()
        pdf.save()
        return out_path

    pdf.setFont("Helvetica-Bold", 9)
    pdf.drawCentredString(text_x, text_y, "Onionskin — printer calibration target")

    body = [
        "1.  Print this page on blank paper at 100% / Actual size. Turn OFF 'Fit to page'.",
        "2.  Put that same sheet back in the tray, same way up, and print this file AGAIN.",
        "3.  Each crosshair now has two impressions. For each one, read where the second",
        "     impression's arms cross the scales: its vertical arm on the x scale below,",
        "     its horizontal arm on the y scale to the left. Right and down are positive.",
        "4.  onionskin calibrate solve --point 'P1:+0.4,-0.2' --point 'P2:...' ...",
    ]
    pdf.setFont("Helvetica", 7)
    for i, line in enumerate(body):
        pdf.drawCentredString(text_x, text_y - mm_to_pt(4.0 + i * 3.4), line)

    pdf.setFont("Helvetica-Oblique", 6)
    pdf.drawCentredString(
        text_x,
        text_y - mm_to_pt(4.0 + len(body) * 3.4 + 2.5),
        f"{page.describe()} — ruler ticks are {RULER_STEP_MM} mm",
    )

    pdf.showPage()
    pdf.save()
    return out_path


def solve_from_offsets(
    offsets: Sequence[tuple[int, float, float]],
    page: PageSize = A4,
    inset_mm: float | None = None,
) -> SimilarityFit:
    """Fit the printer's error from per-fiducial ``(index, dx_mm, dy_mm)`` readings.

    The inset must match the target that was printed, or the fitted rotation and
    scale will be wrong — hence the shared default.
    """
    if inset_mm is None:
        inset_mm = default_inset(page)
    points = fiducials(page, inset_mm)
    nominal, observed = [], []
    for index, dx, dy in offsets:
        if not 1 <= index <= len(points):
            raise ValueError(
                f"P{index} is not on the target (it has {len(points)} points)"
            )
        px, py = points[index - 1]
        nominal.append((px, py))
        observed.append((px + dx, py + dy))
    return solve_similarity(nominal, observed, page)


def parse_point(spec: str) -> tuple[int, float, float]:
    """Parse ``P1:+0.40,-0.15`` into ``(1, 0.40, -0.15)``."""
    raw = spec.strip()
    if ":" not in raw:
        raise ValueError(
            f"bad point '{spec}'. Expected 'P1:dx,dy', e.g. 'P1:+0.40,-0.15'"
        )
    label, values = raw.split(":", 1)
    label = label.strip().lstrip("Pp")
    try:
        index = int(label)
    except ValueError as exc:
        raise ValueError(f"bad point label in '{spec}'") from exc
    parts = values.replace(" ", "").split(",")
    if len(parts) != 2:
        raise ValueError(f"bad offsets in '{spec}'. Expected 'dx,dy' in mm")
    try:
        return index, float(parts[0]), float(parts[1])
    except ValueError as exc:
        raise ValueError(f"offsets in '{spec}' are not numbers") from exc
