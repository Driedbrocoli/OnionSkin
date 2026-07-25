"""End to end: two documents in, one printable delta PDF out."""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Sequence

import numpy as np

from . import calibrate as calib
from . import safety
from .compose import Composition, TextBox, compose
from .delta import (
    RasterDeltaWriter,
    VectorDeltaWriter,
    apply_correction,
    conform_to_source,
    preview_page,
)
from .diff import (
    DEFAULT_GROUP_MM,
    DEFAULT_INK_THRESHOLD,
    DEFAULT_MIN_REGION_MM2,
    DEFAULT_TOLERANCE_MM,
    PageDiff,
    diff_page,
    label_regions,
)
from .geometry import PageSize, Similarity
from .render import Document, DocumentError, Workspace, to_pdf
from .safety import Check

RASTER = "raster"
VECTOR = "vector"
MODES = (RASTER, VECTOR)

#: Not a diff mode — the label a composed delta reports itself under.
COMPOSE = "compose"

DEFAULT_DPI = 400.0


@dataclass
class Options:
    dpi: float = DEFAULT_DPI
    mode: str = RASTER
    margin_mm: float = safety.DEFAULT_MARGIN_MM
    profile: str | None = None
    ink_threshold: int = DEFAULT_INK_THRESHOLD
    tolerance_mm: float = DEFAULT_TOLERANCE_MM
    group_mm: float = DEFAULT_GROUP_MM
    min_region_mm2: float = DEFAULT_MIN_REGION_MM2
    pad_mm: float = 0.3
    preview_dir: Path | None = None
    preview_width: int = 1000

    def validate(self) -> None:
        if self.mode not in MODES:
            raise ValueError(f"mode must be one of {MODES}, got '{self.mode}'")
        if not 50 <= self.dpi <= 1200:
            raise ValueError("dpi must be between 50 and 1200")
        if not 0 < self.ink_threshold < 255:
            raise ValueError("ink-threshold must be between 1 and 254")


@dataclass
class Result:
    output: Path
    pages: list[PageDiff]
    checks: list[Check] = field(default_factory=list)
    previews: list[Path] = field(default_factory=list)
    mode: str = RASTER
    dpi: float = DEFAULT_DPI
    profile: calib.Profile | None = None

    @property
    def blocked(self) -> bool:
        return safety.has_blockers(self.checks)

    @property
    def total_regions(self) -> int:
        return sum(len(p.added_regions) for p in self.pages)

    @property
    def total_added_mm2(self) -> float:
        return sum(p.added_ink_mm2 for p in self.pages)

    @property
    def pages_with_additions(self) -> list[int]:
        return [p.index + 1 for p in self.pages if p.has_additions]

    def to_dict(self) -> dict:
        return {
            "output": str(self.output),
            "mode": self.mode,
            "dpi": self.dpi,
            "profile": self.profile.name if self.profile else None,
            "correction": (
                self.profile.correction.describe() if self.profile else None
            ),
            "blocked": self.blocked,
            "total_regions": self.total_regions,
            "total_added_mm2": round(self.total_added_mm2, 2),
            "pages_with_additions": self.pages_with_additions,
            "pages": [p.to_dict() for p in self.pages],
            "checks": [c.to_dict() for c in self.checks],
            "previews": [str(p) for p in self.previews],
        }


def _blank_gray(size: PageSize, dpi: float) -> np.ndarray:
    w, h = size.px_size(dpi)
    return np.full((h, w), 255, dtype=np.uint8)


def guard_output(output: str | Path, *inputs: str | Path) -> Path:
    """Refuse to write the delta over one of the documents it was made from.

    ``onionskin delta report.pdf report-v2.pdf -o report.pdf`` is an easy thing
    to type, and without this it destroys the original — the very sheet the
    delta is meant to be printed onto, and quite possibly the only copy. Paths
    are resolved first so that a symlink or a roundabout relative path cannot
    slip past.
    """
    output = Path(output)
    try:
        resolved = output.resolve()
    except OSError:
        return output

    for source in inputs:
        candidate = Path(source)
        try:
            if candidate.exists() and candidate.resolve() == resolved:
                raise DocumentError(
                    f"refusing to write the delta over '{candidate}' — that is one "
                    "of the documents it is made from, and overwriting it would "
                    "destroy the original. Choose a different --output."
                )
        except OSError:
            continue
    return output


def compose_run(
    source: str | Path,
    boxes: Sequence[TextBox],
    output: str | Path,
    options: Options | None = None,
) -> Result:
    """Place text at fixed positions on a document's pages.

    Everything downstream of the delta is shared with :func:`run` — the same
    margin and coverage checks, the same proof previews, the same calibration.
    What is absent is the reflow check, and not by omission: absolutely
    positioned text cannot displace anything, so no ink can move. That is the
    whole reason this path exists.
    """
    options = options or Options()
    options.validate()
    output = guard_output(output, source)

    profile = calib.load_profile(options.profile) if options.profile else None
    if options.preview_dir:
        Path(options.preview_dir).mkdir(parents=True, exist_ok=True)

    with Workspace() as work:
        source_pdf = to_pdf(source, work)

        with Document(source_pdf) as doc:
            composition = Composition(page_sizes=list(doc.page_sizes), boxes=list(boxes))
            staged = work / "delta-raw.pdf"
            compose(composition, staged)

            diffs: list[PageDiff] = []
            previews: list[Path] = []

            with Document(staged) as delta_doc:
                for index in range(len(doc)):
                    size = doc.page_sizes[index]
                    rendered = delta_doc.render(index, options.dpi)
                    added = rendered.gray <= options.ink_threshold
                    empty = np.zeros_like(added)

                    diff = PageDiff(
                        index=index,
                        size=size,
                        dpi=options.dpi,
                        added=added,
                        removed=empty,
                        added_px=int(added.sum()),
                        removed_px=0,
                        added_regions=label_regions(
                            added, options.dpi, options.group_mm, options.min_region_mm2
                        ),
                        removed_regions=[],
                    )

                    if options.preview_dir:
                        page = doc.render(index, options.dpi)
                        image = preview_page(diff, page.gray, options.preview_width)
                        path = Path(options.preview_dir) / f"page-{index + 1:03d}.png"
                        image.save(path, format="PNG", optimize=True)
                        previews.append(path)
                        del page

                    diff.release()
                    diffs.append(diff)

            checks: list[safety.Check] = []
            for diff in diffs:
                checks += safety.check_margins(diff, options.margin_mm)
                checks += safety.check_coverage(diff)
            checks += safety.check_empty(diffs)
            checks += safety.check_calibration(
                profile is not None, profile.name if profile else None
            )
            checks += safety.check_profile_page(profile, doc.page_sizes[0])

            correction = profile.correction if profile else Similarity.identity()
            output.parent.mkdir(parents=True, exist_ok=True)
            corrected = apply_correction(
                staged, work / "delta-corrected.pdf", correction, list(doc.page_sizes)
            )
            conform_to_source(corrected, output, doc.frames, list(doc.page_sizes))

    return Result(
        output=output,
        pages=diffs,
        checks=safety.sort_checks(checks),
        previews=previews,
        mode=COMPOSE,
        dpi=options.dpi,
        profile=profile,
    )


def run(
    original: str | Path,
    edited: str | Path,
    output: str | Path,
    options: Options | None = None,
) -> Result:
    """Compare two documents and write the delta PDF.

    Pages are handled one at a time — render, diff, emit, release — so memory
    stays flat regardless of document length.
    """
    options = options or Options()
    options.validate()
    output = guard_output(output, original, edited)

    profile = calib.load_profile(options.profile) if options.profile else None

    if options.preview_dir:
        Path(options.preview_dir).mkdir(parents=True, exist_ok=True)

    with Workspace() as work:
        original_pdf = to_pdf(original, work)
        edited_pdf = to_pdf(edited, work)

        with Document(original_pdf) as old_doc, Document(edited_pdf) as new_doc:
            checks = safety.check_documents(old_doc.page_sizes, new_doc.page_sizes)

            staged = work / "delta-raw.pdf"
            writer = (
                RasterDeltaWriter(staged)
                if options.mode == RASTER
                else VectorDeltaWriter(staged, edited_pdf, options.pad_mm)
            )

            diffs: list[PageDiff] = []
            previews: list[Path] = []
            sizes: list[PageSize] = []

            for index in range(len(new_doc)):
                new_page = new_doc.render(index, options.dpi)
                sizes.append(new_page.size)

                if index < len(old_doc):
                    old_page = old_doc.render(index, options.dpi)
                    old_gray = old_page.gray
                    if not old_page.size.matches(new_page.size):
                        # Sizes already flagged as a blocker; skip the page
                        # rather than diff two different geometries.
                        old_gray = _blank_gray(new_page.size, options.dpi)
                else:
                    # A page the edit added: there is no printed sheet behind
                    # it, so everything on it is new.
                    old_gray = _blank_gray(new_page.size, options.dpi)

                diff = diff_page(
                    old_gray,
                    new_page.gray,
                    size=new_page.size,
                    dpi=options.dpi,
                    index=index,
                    ink_threshold=options.ink_threshold,
                    tolerance_mm=options.tolerance_mm,
                    group_mm=options.group_mm,
                    min_region_mm2=options.min_region_mm2,
                )

                writer.add_page(diff, new_page.rgb)

                if options.preview_dir:
                    image = preview_page(diff, old_gray, options.preview_width)
                    path = Path(options.preview_dir) / f"page-{index + 1:03d}.png"
                    image.save(path, format="PNG", optimize=True)
                    previews.append(path)

                diff.release()
                diffs.append(diff)
                del new_page, old_gray

            writer.close()

            checks += safety.sort_checks(
                [
                    c
                    for diff in diffs
                    for c in (
                        safety.check_reflow(diff)
                        + safety.check_margins(diff, options.margin_mm)
                        + safety.check_coverage(diff)
                    )
                ]
            )
            checks += safety.check_empty(diffs)
            checks += safety.check_calibration(
                profile is not None, profile.name if profile else None
            )
            checks += safety.check_profile_page(profile, sizes[0])

            correction = profile.correction if profile else Similarity.identity()
            output.parent.mkdir(parents=True, exist_ok=True)
            corrected = apply_correction(
                staged, work / "delta-corrected.pdf", correction, sizes
            )
            # Conform to the ORIGINAL: that is the sheet going back in the tray.
            frames = list(old_doc.frames[: len(sizes)])
            frames += [new_doc.frames[i] for i in range(len(frames), len(sizes))]
            conform_to_source(corrected, output, frames, sizes)

    return Result(
        output=output,
        pages=diffs,
        checks=safety.sort_checks(checks),
        previews=previews,
        mode=options.mode,
        dpi=options.dpi,
        profile=profile,
    )
