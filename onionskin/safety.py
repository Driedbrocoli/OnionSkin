"""Checks that run before a sheet goes back in the tray.

Every one of these exists to stop a specific way of wasting paper, and the
first is the important one. Adding a word mid-paragraph pushes everything after
it down the page. The delta then contains not just the new word but the
re-flowed remainder, which cannot be printed onto a sheet whose text is still
in the old position. Detecting that is worth more than any amount of
sub-millimetre accuracy.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Sequence

from .diff import PageDiff
from .geometry import PageSize

BLOCKER = "blocker"
WARNING = "warning"
NOTE = "note"

_RANK = {BLOCKER: 0, WARNING: 1, NOTE: 2}

#: Default non-printable border. Most inkjets cannot place ink within about
#: 5 mm of any edge; many lasers need more at the trailing edge.
DEFAULT_MARGIN_MM = 5.0

#: Enough displaced ink to mean a line moved, rather than a stray anti-aliased
#: pixel. Roughly the footprint of two characters.
REFLOW_INK_MM2 = 1.5


@dataclass
class Check:
    severity: str
    code: str
    message: str
    detail: str = ""
    page: int | None = None

    def to_dict(self) -> dict:
        return {
            "severity": self.severity,
            "code": self.code,
            "message": self.message,
            "detail": self.detail,
            "page": self.page,
        }

    def format(self) -> str:
        label = {BLOCKER: "BLOCKER", WARNING: "WARNING", NOTE: "note"}[self.severity]
        where = f" [page {self.page}]" if self.page else ""
        line = f"{label}{where}: {self.message}"
        return f"{line}\n    {self.detail}" if self.detail else line


def sort_checks(checks: Sequence[Check]) -> list[Check]:
    return sorted(checks, key=lambda c: (_RANK[c.severity], c.page or 0, c.code))


def check_documents(
    original_sizes: Sequence[PageSize], edited_sizes: Sequence[PageSize]
) -> list[Check]:
    checks: list[Check] = []

    if len(edited_sizes) > len(original_sizes):
        extra = len(edited_sizes) - len(original_sizes)
        checks.append(
            Check(
                severity=WARNING,
                code="pages_added",
                message=f"The edit added {extra} page(s).",
                detail=(
                    f"The original is {len(original_sizes)} page(s), the edit is "
                    f"{len(edited_sizes)}. The extra page(s) have no printed sheet "
                    "to go onto — print those on blank paper."
                ),
            )
        )
    elif len(edited_sizes) < len(original_sizes):
        checks.append(
            Check(
                severity=BLOCKER,
                code="pages_removed",
                message="The edit has fewer pages than the original.",
                detail=(
                    f"{len(original_sizes)} → {len(edited_sizes)} pages. Content was "
                    "removed or pulled onto earlier pages, so the printed sheets no "
                    "longer match the document. Print a fresh copy."
                ),
            )
        )

    for i, (old, new) in enumerate(zip(original_sizes, edited_sizes), start=1):
        if not old.matches(new):
            checks.append(
                Check(
                    severity=BLOCKER,
                    code="page_size_mismatch",
                    page=i,
                    message="Page size changed between the two documents.",
                    detail=(
                        f"original {old.describe()} vs edited {new.describe()}. "
                        "Nothing will line up. Check the page setup in both files."
                    ),
                )
            )
    return checks


def check_reflow(diff: PageDiff) -> list[Check]:
    removed = diff.removed_ink_mm2
    if removed < REFLOW_INK_MM2:
        return []

    top = min((r.y0_mm for r in diff.removed_regions), default=0.0)
    return [
        Check(
            severity=BLOCKER,
            code="reflow",
            page=diff.index + 1,
            message="Existing content moved or was deleted on this page.",
            detail=(
                f"{removed:.0f} mm² of ink is gone from where it was, starting "
                f"{top:.0f} mm down the page. The printed sheet no longer matches the "
                "document, so an overlay cannot fix it — print this page fresh.\n"
                "    To add text without disturbing the layout, put it in a Word text "
                "box set to 'Fixed position on page' with no text wrapping."
            ),
        )
    ]


def check_margins(
    diff: PageDiff, margin_mm: float = DEFAULT_MARGIN_MM
) -> list[Check]:
    if margin_mm <= 0 or not diff.added_regions:
        return []

    size = diff.size
    offenders = []
    for region in diff.added_regions:
        if (
            region.x0_mm < margin_mm
            or region.y0_mm < margin_mm
            or region.x1_mm > size.width_mm - margin_mm
            or region.y1_mm > size.height_mm - margin_mm
        ):
            offenders.append(region)

    if not offenders:
        return []

    worst = min(
        min(
            r.x0_mm,
            r.y0_mm,
            size.width_mm - r.x1_mm,
            size.height_mm - r.y1_mm,
        )
        for r in offenders
    )
    return [
        Check(
            severity=WARNING,
            code="margin",
            page=diff.index + 1,
            message=(
                f"{len(offenders)} addition(s) sit inside the {margin_mm:g} mm "
                "non-printable border."
            ),
            detail=(
                f"The closest comes within {max(worst, 0):.1f} mm of an edge. Most "
                "printers will clip or refuse to print it. Move it inward, or lower "
                "--margin if you know this printer goes closer."
            ),
        )
    ]


def check_coverage(diff: PageDiff) -> list[Check]:
    page_mm2 = diff.size.width_mm * diff.size.height_mm
    added = diff.added_ink_mm2
    if page_mm2 <= 0 or added <= 0:
        return []
    fraction = added / page_mm2
    if fraction < 0.06:
        return []
    return [
        Check(
            severity=WARNING,
            code="large_delta",
            page=diff.index + 1,
            message=f"The delta covers {fraction * 100:.0f}% of this page.",
            detail=(
                "That is a lot of new ink for an overlay. If this is not what you "
                "expected, the layout probably shifted — compare the preview against "
                "the sheet before printing."
            ),
        )
    ]


def check_empty(diffs: Sequence[PageDiff]) -> list[Check]:
    if any(d.has_additions for d in diffs):
        return []
    return [
        Check(
            severity=BLOCKER,
            code="empty_delta",
            message="No additions found — the delta would print a blank page.",
            detail=(
                "The two documents render identically. Check you passed the edited "
                "file second, and that the edit was saved."
            ),
        )
    ]


def check_calibration(calibrated: bool, profile_name: str | None) -> list[Check]:
    if calibrated:
        return [
            Check(
                severity=NOTE,
                code="calibrated",
                message=f"Calibration profile '{profile_name}' applied.",
            )
        ]
    return [
        Check(
            severity=NOTE,
            code="uncalibrated",
            message="No calibration profile — expect roughly ±2 mm of registration error.",
            detail=(
                "Run 'onionskin calibrate target' once per printer to bring that "
                "under ±0.5 mm."
            ),
        )
    ]


def check_all(
    diffs: Sequence[PageDiff],
    original_sizes: Sequence[PageSize],
    edited_sizes: Sequence[PageSize],
    margin_mm: float = DEFAULT_MARGIN_MM,
    calibrated: bool = False,
    profile_name: str | None = None,
) -> list[Check]:
    checks = list(check_documents(original_sizes, edited_sizes))
    for diff in diffs:
        checks += check_reflow(diff)
        checks += check_margins(diff, margin_mm)
        checks += check_coverage(diff)
    checks += check_empty(diffs)
    checks += check_calibration(calibrated, profile_name)
    return sort_checks(checks)


def has_blockers(checks: Sequence[Check]) -> bool:
    return any(c.severity == BLOCKER for c in checks)
