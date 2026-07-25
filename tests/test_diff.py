import numpy as np
import pytest

from onionskin.diff import diff_page, label_regions
from onionskin.geometry import PageSize
from onionskin.render import Document

from helpers import A4, make_pdf

DPI = 300.0


def render_gray(path):
    with Document(path) as doc:
        return doc.render(0, DPI).gray, doc.page_sizes[0]


def test_identical_pages_produce_nothing(tmp_path):
    items = [(20.0, 40.0, "The quarterly figures are attached.")]
    a = make_pdf(tmp_path / "a.pdf", items)
    b = make_pdf(tmp_path / "b.pdf", items)

    gray_a, size = render_gray(a)
    gray_b, _ = render_gray(b)
    diff = diff_page(gray_a, gray_b, size, DPI)

    assert diff.added_regions == []
    assert diff.removed_regions == []
    assert diff.added_ink_mm2 == 0.0


def test_added_text_is_found_where_it_was_put(tmp_path):
    base = [(20.0, 40.0, "Original line")]
    a = make_pdf(tmp_path / "a.pdf", base)
    b = make_pdf(tmp_path / "b.pdf", base + [(60.0, 120.0, "Approved")])

    gray_a, size = render_gray(a)
    gray_b, _ = render_gray(b)
    diff = diff_page(gray_a, gray_b, size, DPI)

    assert len(diff.added_regions) == 1
    region = diff.added_regions[0]
    assert region.x0_mm == pytest.approx(60.0, abs=1.0)
    # drawString places the text baseline at y, so ink sits just above it.
    assert region.y1_mm == pytest.approx(120.0, abs=1.5)
    assert diff.removed_regions == []


def test_deleted_text_shows_up_as_removed_not_added(tmp_path):
    """Ink that vanished is the reflow signal, and is never printable."""
    a = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "keep"), (20.0, 80.0, "delete me")])
    b = make_pdf(tmp_path / "b.pdf", [(20.0, 40.0, "keep")])

    gray_a, size = render_gray(a)
    gray_b, _ = render_gray(b)
    diff = diff_page(gray_a, gray_b, size, DPI)

    assert diff.added_regions == []
    assert len(diff.removed_regions) == 1
    assert diff.removed_regions[0].y1_mm == pytest.approx(80.0, abs=1.5)


def test_shifted_text_reads_as_both_added_and_removed(tmp_path):
    """A reflowed line leaves ink behind at the old position."""
    a = make_pdf(tmp_path / "a.pdf", [(20.0, 100.0, "This paragraph moved")])
    b = make_pdf(tmp_path / "b.pdf", [(20.0, 106.0, "This paragraph moved")])

    gray_a, size = render_gray(a)
    gray_b, _ = render_gray(b)
    diff = diff_page(gray_a, gray_b, size, DPI)

    assert diff.added_ink_mm2 > 1.0
    assert diff.removed_ink_mm2 > 1.0


def test_tolerance_absorbs_sub_pixel_jitter():
    """A mark that moved a hair must not be re-printed as new ink."""
    size = PageSize(50.0, 50.0)
    w, h = size.px_size(DPI)
    old = np.full((h, w), 255, dtype=np.uint8)
    old[100:140, 100:300] = 0
    new = np.full((h, w), 255, dtype=np.uint8)
    new[101:141, 101:301] = 0  # one pixel down and right

    strict = diff_page(old, new, size, DPI, tolerance_mm=0.0)
    tolerant = diff_page(old, new, size, DPI, tolerance_mm=0.12)

    assert strict.added_ink_mm2 > 0
    assert tolerant.added_ink_mm2 == 0


def test_letters_of_a_word_group_into_one_region(tmp_path):
    a = make_pdf(tmp_path / "a.pdf", [])
    b = make_pdf(tmp_path / "b.pdf", [(30.0, 60.0, "Onionskin")])

    gray_a, size = render_gray(a)
    gray_b, _ = render_gray(b)
    diff = diff_page(gray_a, gray_b, size, DPI)

    assert len(diff.added_regions) == 1
    assert diff.added_regions[0].width_mm > 10.0


def test_far_apart_additions_stay_separate(tmp_path):
    a = make_pdf(tmp_path / "a.pdf", [])
    b = make_pdf(
        tmp_path / "b.pdf", [(20.0, 40.0, "first"), (20.0, 200.0, "second")]
    )

    gray_a, size = render_gray(a)
    gray_b, _ = render_gray(b)
    diff = diff_page(gray_a, gray_b, size, DPI)

    assert len(diff.added_regions) == 2
    assert diff.added_regions[0].y0_mm < diff.added_regions[1].y0_mm


def test_label_regions_measures_exact_bounds():
    mask = np.zeros((400, 400), dtype=bool)
    mask[100:150, 200:260] = True
    dpi = 254.0  # exactly 10 px per mm, so the arithmetic is checkable by hand

    regions = label_regions(mask, dpi, group_mm=2.0)

    assert len(regions) == 1
    region = regions[0]
    assert region.x0_mm == pytest.approx(20.0)
    assert region.y0_mm == pytest.approx(10.0)
    assert region.width_mm == pytest.approx(6.0)
    assert region.height_mm == pytest.approx(5.0)
    assert region.ink_mm2 == pytest.approx(30.0)


def test_label_regions_bounds_are_not_rounded_to_the_grid():
    """Boxes come from full-resolution pixels, not the coarse grouping grid."""
    mask = np.zeros((400, 400), dtype=bool)
    mask[103:107, 203:207] = True  # deliberately off any cell boundary
    dpi = 254.0

    region = label_regions(mask, dpi, group_mm=2.0, min_area_mm2=0.0)[0]

    assert region.x0_mm == pytest.approx(20.3)
    assert region.y0_mm == pytest.approx(10.3)
    assert region.width_mm == pytest.approx(0.4)


def test_diagonally_touching_marks_are_one_region():
    mask = np.zeros((200, 200), dtype=bool)
    mask[50:55, 50:55] = True
    mask[56:61, 56:61] = True
    regions = label_regions(mask, 254.0, group_mm=2.0, min_area_mm2=0.0)
    assert len(regions) == 1


def test_specks_below_the_floor_are_dropped():
    mask = np.zeros((200, 200), dtype=bool)
    mask[10, 10] = True  # a single pixel of rendering noise
    assert label_regions(mask, 254.0, min_area_mm2=0.05) == []


def test_empty_mask_is_cheap_and_empty():
    assert label_regions(np.zeros((3000, 2000), dtype=bool), 300.0) == []


def test_release_frees_masks_but_keeps_measurements(tmp_path):
    a = make_pdf(tmp_path / "a.pdf", [])
    b = make_pdf(tmp_path / "b.pdf", [(30.0, 60.0, "kept")])

    gray_a, size = render_gray(a)
    gray_b, _ = render_gray(b)
    diff = diff_page(gray_a, gray_b, size, DPI)
    before = diff.added_ink_mm2

    diff.release()

    assert diff.added.size == 0
    assert diff.added_ink_mm2 == before
    assert len(diff.added_regions) == 1
