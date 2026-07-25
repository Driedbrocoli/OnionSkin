import numpy as np
import pytest

from onionskin import safety
from onionskin.diff import PageDiff, Region
from onionskin.geometry import PageSize

A4 = PageSize(210.0, 297.0)
DPI = 300.0


def make_page(index=0, added=None, removed=None, added_px=0, removed_px=0):
    empty = np.zeros((0, 0), dtype=bool)
    return PageDiff(
        index=index,
        size=A4,
        dpi=DPI,
        added=empty,
        removed=empty,
        added_px=added_px,
        removed_px=removed_px,
        added_regions=added or [],
        removed_regions=removed or [],
    )


def region(x, y, w=20.0, h=4.0, ink=8.0):
    return Region(x, y, x + w, y + h, ink_mm2=ink, px_bbox=(0, 0, 1, 1))


def px_for_mm2(mm2):
    return int(mm2 / ((25.4 / DPI) ** 2))


def codes(checks):
    return {c.code for c in checks}


# --- reflow, the check that matters most -----------------------------------


def test_displaced_ink_blocks_printing():
    page = make_page(
        removed=[region(20.0, 100.0)], removed_px=px_for_mm2(40.0)
    )
    checks = safety.check_reflow(page)
    assert codes(checks) == {"reflow"}
    assert checks[0].severity == safety.BLOCKER
    assert "text box" in checks[0].detail


def test_a_speck_of_displaced_ink_is_not_reflow():
    page = make_page(removed=[region(20.0, 100.0)], removed_px=px_for_mm2(0.4))
    assert safety.check_reflow(page) == []


def test_pure_addition_is_never_reflow():
    page = make_page(added=[region(60.0, 120.0)], added_px=px_for_mm2(30.0))
    assert safety.check_reflow(page) == []


# --- page structure --------------------------------------------------------


def test_extra_pages_warn_but_do_not_block():
    checks = safety.check_documents([A4], [A4, A4])
    assert codes(checks) == {"pages_added"}
    assert checks[0].severity == safety.WARNING
    assert "blank paper" in checks[0].detail


def test_lost_pages_block():
    checks = safety.check_documents([A4, A4], [A4])
    assert checks[0].code == "pages_removed"
    assert checks[0].severity == safety.BLOCKER


def test_page_size_change_blocks():
    checks = safety.check_documents([A4], [PageSize(215.9, 279.4)])
    assert checks[0].code == "page_size_mismatch"
    assert checks[0].severity == safety.BLOCKER
    assert "A4" in checks[0].detail and "Letter" in checks[0].detail


def test_matching_documents_raise_nothing():
    assert safety.check_documents([A4, A4], [A4, A4]) == []


# --- margins ---------------------------------------------------------------


@pytest.mark.parametrize(
    "box",
    [
        region(1.0, 100.0),          # off the left edge
        region(100.0, 1.0),          # off the top
        region(199.0, 100.0),        # off the right
        region(100.0, 294.0),        # off the bottom
    ],
)
def test_additions_in_the_dead_border_warn(box):
    checks = safety.check_margins(make_page(added=[box]), margin_mm=5.0)
    assert codes(checks) == {"margin"}
    assert checks[0].severity == safety.WARNING


def test_additions_well_inside_do_not_warn():
    page = make_page(added=[region(50.0, 150.0)])
    assert safety.check_margins(page, margin_mm=5.0) == []


def test_margin_check_can_be_switched_off():
    page = make_page(added=[region(0.5, 0.5)])
    assert safety.check_margins(page, margin_mm=0.0) == []


def test_margin_message_counts_every_offender():
    page = make_page(added=[region(1.0, 100.0), region(1.0, 150.0)])
    assert "2 addition(s)" in safety.check_margins(page)[0].message


# --- coverage and emptiness ------------------------------------------------


def test_a_delta_covering_most_of_the_page_warns():
    page = make_page(added=[region(10.0, 10.0)], added_px=px_for_mm2(20000.0))
    checks = safety.check_coverage(page)
    assert codes(checks) == {"large_delta"}


def test_a_few_words_do_not_trip_the_coverage_warning():
    page = make_page(added=[region(60.0, 120.0)], added_px=px_for_mm2(120.0))
    assert safety.check_coverage(page) == []


def test_no_additions_at_all_blocks():
    checks = safety.check_empty([make_page(), make_page(index=1)])
    assert checks[0].code == "empty_delta"
    assert checks[0].severity == safety.BLOCKER


def test_any_addition_clears_the_empty_check():
    pages = [make_page(), make_page(index=1, added=[region(60.0, 120.0)])]
    assert safety.check_empty(pages) == []


# --- calibration nudge -----------------------------------------------------


def test_uncalibrated_is_a_note_not_a_warning():
    checks = safety.check_calibration(False, None)
    assert checks[0].code == "uncalibrated"
    assert checks[0].severity == safety.NOTE
    assert "±2 mm" in checks[0].message


def test_calibrated_reports_the_profile():
    checks = safety.check_calibration(True, "office-laser")
    assert "office-laser" in checks[0].message


# --- aggregation -----------------------------------------------------------


def test_blockers_sort_ahead_of_warnings_and_notes():
    pages = [
        make_page(
            added=[region(1.0, 100.0)],
            removed=[region(20.0, 200.0)],
            added_px=px_for_mm2(30.0),
            removed_px=px_for_mm2(50.0),
        )
    ]
    checks = safety.check_all(pages, [A4], [A4], calibrated=False)
    severities = [c.severity for c in checks]
    assert severities == sorted(severities, key=lambda s: safety._RANK[s])
    assert checks[0].severity == safety.BLOCKER
    assert safety.has_blockers(checks)


def test_a_clean_run_has_no_blockers():
    pages = [make_page(added=[region(60.0, 120.0)], added_px=px_for_mm2(40.0))]
    checks = safety.check_all(pages, [A4], [A4], calibrated=True, profile_name="p")
    assert not safety.has_blockers(checks)


def test_check_formats_with_page_and_detail():
    check = safety.Check(safety.BLOCKER, "reflow", "moved", "because", page=3)
    text = check.format()
    assert "BLOCKER" in text and "[page 3]" in text and "because" in text
