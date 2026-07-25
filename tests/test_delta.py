import numpy as np
import pikepdf
import pytest

from onionskin.delta import (
    _unmatte,
    apply_correction,
    build_raster_delta,
    build_vector_delta,
    preview_page,
)
from onionskin.diff import diff_page
from onionskin.geometry import PageSize, Similarity
from onionskin.render import Document

from helpers import A4, ink_bbox_mm, make_pdf

DPI = 300.0


def make_diff(tmp_path, base, edited):
    a = make_pdf(tmp_path / "a.pdf", base)
    b = make_pdf(tmp_path / "b.pdf", edited)
    with Document(a) as old, Document(b) as new:
        old_page = old.render(0, DPI)
        new_page = new.render(0, DPI)
    diff = diff_page(old_page.gray, new_page.gray, new_page.size, DPI)
    return diff, new_page.rgb, b


def test_raster_delta_keeps_the_page_size(tmp_path):
    diff, rgb, _ = make_diff(
        tmp_path, [(20.0, 40.0, "original")], [(20.0, 40.0, "original"), (60.0, 120.0, "new")]
    )
    out = build_raster_delta([diff], [rgb], tmp_path / "delta.pdf")

    with pikepdf.open(out) as pdf:
        assert len(pdf.pages) == 1
        box = [float(v) for v in pdf.pages[0].MediaBox]
    assert box[2] == pytest.approx(A4.width_pt, abs=0.5)
    assert box[3] == pytest.approx(A4.height_pt, abs=0.5)


def test_raster_delta_contains_only_the_addition(tmp_path):
    """The whole point: the original's ink must not be on the delta."""
    diff, rgb, _ = make_diff(
        tmp_path,
        [(20.0, 40.0, "already printed")],
        [(20.0, 40.0, "already printed"), (60.0, 120.0, "Approved")],
    )
    out = build_raster_delta([diff], [rgb], tmp_path / "delta.pdf")

    bbox = ink_bbox_mm(out, DPI)
    assert bbox is not None
    x0, y0, x1, y1 = bbox
    assert x0 == pytest.approx(60.0, abs=1.0)
    assert y1 == pytest.approx(120.0, abs=1.5)
    # Nothing anywhere near the original line at y=40.
    assert y0 > 100.0


def test_raster_delta_page_with_no_additions_is_blank(tmp_path):
    diff, rgb, _ = make_diff(tmp_path, [(20.0, 40.0, "same")], [(20.0, 40.0, "same")])
    out = build_raster_delta([diff], [rgb], tmp_path / "delta.pdf")
    assert ink_bbox_mm(out, DPI) is None


def test_raster_delta_uses_real_transparency(tmp_path):
    """Without an SMask the addition would print inside a white box."""
    diff, rgb, _ = make_diff(tmp_path, [], [(60.0, 120.0, "Approved")])
    out = build_raster_delta([diff], [rgb], tmp_path / "delta.pdf")

    with pikepdf.open(out) as pdf:
        xobjects = list(pdf.pages[0].resources.XObject.values())
        assert xobjects, "expected an image on the page"
        assert "/SMask" in xobjects[0]


def test_unmatte_recovers_coverage_from_antialiasing():
    """A half-grey edge pixel must come back as half-opaque black, not grey."""
    rgb = np.array([[[128, 128, 128]]], dtype=np.uint8)
    mask = np.array([[True]])

    rgba = np.asarray(_unmatte(rgb, mask))

    assert rgba[0, 0, 3] == pytest.approx(127, abs=2)  # ~50% coverage
    assert rgba[0, 0, 0] < 12  # the ink itself is black


def test_unmatte_preserves_ink_colour():
    rgb = np.array([[[255, 0, 0]]], dtype=np.uint8)  # solid red
    rgba = np.asarray(_unmatte(rgb, np.array([[True]])))
    assert rgba[0, 0, 3] == 255
    assert rgba[0, 0, 0] > 240 and rgba[0, 0, 1] < 15


def test_unmatte_ignores_pixels_outside_the_mask():
    rgb = np.zeros((2, 2, 3), dtype=np.uint8)  # all black
    mask = np.array([[True, False], [False, False]])
    rgba = np.asarray(_unmatte(rgb, mask))
    assert rgba[0, 0, 3] == 255
    assert rgba[0, 1, 3] == 0 and rgba[1, 1, 3] == 0


def test_vector_delta_keeps_the_addition_and_drops_the_rest(tmp_path):
    diff, _, edited = make_diff(
        tmp_path,
        [(20.0, 40.0, "already printed")],
        [(20.0, 40.0, "already printed"), (60.0, 120.0, "Approved")],
    )
    out = build_vector_delta([diff], edited, tmp_path / "delta.pdf")

    bbox = ink_bbox_mm(out, DPI)
    assert bbox is not None
    x0, y0, x1, y1 = bbox
    assert x0 == pytest.approx(60.0, abs=1.5)
    assert y0 > 100.0


def test_vector_delta_stays_vector(tmp_path):
    """Its whole reason to exist is keeping text as text."""
    diff, _, edited = make_diff(tmp_path, [], [(60.0, 120.0, "Approved")])
    out = build_vector_delta([diff], edited, tmp_path / "delta.pdf")

    with pikepdf.open(out) as pdf:
        fonts = [
            obj
            for obj in pdf.objects
            if isinstance(obj, pikepdf.Dictionary)
            and obj.get("/Type") == pikepdf.Name.Font
        ]
    assert fonts, "vector mode should carry the font through, not rasterise"


def test_vector_delta_blank_page_when_nothing_changed(tmp_path):
    diff, _, edited = make_diff(tmp_path, [(20.0, 40.0, "x")], [(20.0, 40.0, "x")])
    out = build_vector_delta([diff], edited, tmp_path / "delta.pdf")
    assert ink_bbox_mm(out, DPI) is None


def test_both_modes_agree_on_where_the_ink_goes(tmp_path):
    diff, rgb, edited = make_diff(tmp_path, [], [(45.0, 90.0, "Signed")])
    raster = build_raster_delta([diff], [rgb], tmp_path / "r.pdf")
    vector = build_vector_delta([diff], edited, tmp_path / "v.pdf", pad_mm=0.0)

    rb = ink_bbox_mm(raster, DPI)
    vb = ink_bbox_mm(vector, DPI)
    for got, want in zip(vb, rb):
        assert got == pytest.approx(want, abs=0.5)


# --- calibration correction ------------------------------------------------


def correction_for(error: Similarity) -> Similarity:
    return error.inverse()


def test_correction_shifts_ink_by_the_measured_error(tmp_path):
    """If the printer drifts +2 mm right, the delta must be drawn 2 mm left."""
    source = make_pdf(tmp_path / "src.pdf", [(60.0, 120.0, "Approved")])
    before = ink_bbox_mm(source, DPI)

    printer_error = Similarity(dx_mm=2.0, dy_mm=3.0)
    out = apply_correction(
        source, tmp_path / "out.pdf", correction_for(printer_error), [A4]
    )
    after = ink_bbox_mm(out, DPI)

    assert after[0] == pytest.approx(before[0] - 2.0, abs=0.15)
    assert after[1] == pytest.approx(before[1] - 3.0, abs=0.15)


def test_correction_scale_is_about_the_page_centre(tmp_path):
    source = make_pdf(tmp_path / "src.pdf", [(100.0, 148.5, "x")])
    before = ink_bbox_mm(source, DPI)

    out = apply_correction(
        source, tmp_path / "out.pdf", Similarity(scale=1.02), [A4]
    )
    after = ink_bbox_mm(out, DPI)

    # A mark essentially at the centre barely moves under a centre scale.
    assert after[0] == pytest.approx(before[0], abs=0.5)
    assert after[1] == pytest.approx(before[1], abs=0.5)


def test_correction_scale_moves_a_corner_mark(tmp_path):
    source = make_pdf(tmp_path / "src.pdf", [(25.0, 25.0, "x")])
    before = ink_bbox_mm(source, DPI)

    out = apply_correction(source, tmp_path / "out.pdf", Similarity(scale=1.02), [A4])
    after = ink_bbox_mm(out, DPI)

    cx, cy = A4.center_mm
    assert after[0] == pytest.approx(cx + (before[0] - cx) * 1.02, abs=0.2)
    assert after[1] == pytest.approx(cy + (before[1] - cy) * 1.02, abs=0.2)


def test_correction_rotation_direction(tmp_path):
    """A clockwise correction takes a mark above centre to the right."""
    cx, cy = A4.center_mm
    source = make_pdf(tmp_path / "src.pdf", [(cx, cy - 80.0, "x")])
    before = ink_bbox_mm(source, DPI)

    out = apply_correction(
        source, tmp_path / "out.pdf", Similarity(rotation_deg=2.0), [A4]
    )
    after = ink_bbox_mm(out, DPI)

    assert after[0] > before[0] + 1.0
    assert after[1] > before[1]


def test_identity_correction_leaves_the_file_alone(tmp_path):
    source = make_pdf(tmp_path / "src.pdf", [(60.0, 120.0, "Approved")])
    out = apply_correction(
        source, tmp_path / "out.pdf", Similarity.identity(), [A4]
    )
    assert out.read_bytes() == source.read_bytes()


def test_correction_preserves_page_size(tmp_path):
    source = make_pdf(tmp_path / "src.pdf", [(60.0, 120.0, "x")])
    out = apply_correction(
        source, tmp_path / "out.pdf", Similarity(dx_mm=1.0, rotation_deg=0.5), [A4]
    )
    with pikepdf.open(out) as pdf:
        box = [float(v) for v in pdf.pages[0].MediaBox]
    assert box[2] == pytest.approx(A4.width_pt, abs=0.5)
    assert box[3] == pytest.approx(A4.height_pt, abs=0.5)


def test_correction_handles_multiple_pages(tmp_path):
    source = make_pdf(tmp_path / "src.pdf", [(60.0, 120.0, "x")], pages=3)
    out = apply_correction(
        source, tmp_path / "out.pdf", Similarity(dx_mm=2.0), [A4, A4, A4]
    )
    with pikepdf.open(out) as pdf:
        assert len(pdf.pages) == 3
    for page in range(3):
        assert ink_bbox_mm(out, DPI, page)[0] == pytest.approx(
            ink_bbox_mm(source, DPI, page)[0] + 2.0, abs=0.15
        )


def test_correction_round_trip_returns_ink_to_source(tmp_path):
    """error then correction should cancel to within a rendered pixel."""
    error = Similarity(dx_mm=0.8, dy_mm=-0.5, rotation_deg=0.3, scale=1.002)
    source = make_pdf(tmp_path / "src.pdf", [(70.0, 200.0, "Approved")])

    corrected = apply_correction(
        source, tmp_path / "corrected.pdf", error.inverse(), [A4]
    )
    reprinted = apply_correction(corrected, tmp_path / "final.pdf", error, [A4])

    for got, want in zip(ink_bbox_mm(reprinted, DPI), ink_bbox_mm(source, DPI)):
        assert got == pytest.approx(want, abs=0.15)


# --- preview ---------------------------------------------------------------


def test_preview_marks_additions_in_red(tmp_path):
    diff, _, _ = make_diff(
        tmp_path, [(20.0, 40.0, "old")], [(20.0, 40.0, "old"), (60.0, 120.0, "new")]
    )
    with Document(make_pdf(tmp_path / "a2.pdf", [(20.0, 40.0, "old")])) as doc:
        old_gray = doc.render(0, DPI).gray

    image = np.asarray(preview_page(diff, old_gray, max_width=800))

    reddish = (image[:, :, 0] > 150) & (image[:, :, 1] < 120) & (image[:, :, 2] < 120)
    assert reddish.any()
    assert image.shape[1] == 800
