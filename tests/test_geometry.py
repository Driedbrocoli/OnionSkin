import math

import pytest

from onionskin.geometry import (
    PageSize,
    Similarity,
    mm_to_pt,
    pt_to_mm,
    solve_similarity,
)

A4 = PageSize(210.0, 297.0)


def test_unit_roundtrip():
    assert pt_to_mm(mm_to_pt(123.4)) == pytest.approx(123.4)
    assert mm_to_pt(25.4) == pytest.approx(72.0)


def test_page_size_from_pt_recognises_a4():
    page = PageSize.from_pt(595.276, 841.89)
    assert page.width_mm == pytest.approx(210.0, abs=0.05)
    assert "A4" in page.describe()


def test_translation_moves_point():
    t = Similarity(dx_mm=2.0, dy_mm=-3.0)
    assert t.apply((100.0, 100.0), A4) == pytest.approx((102.0, 97.0))


def test_rotation_is_clockwise_on_the_page():
    """+90 degrees must take a point above centre to the right of centre."""
    t = Similarity(rotation_deg=90.0)
    cx, cy = A4.center_mm
    x, y = t.apply((cx, cy - 50.0), A4)
    assert x == pytest.approx(cx + 50.0)
    assert y == pytest.approx(cy)


def test_scale_is_about_the_page_centre():
    t = Similarity(scale=2.0)
    cx, cy = A4.center_mm
    assert t.apply((cx, cy), A4) == pytest.approx((cx, cy))
    assert t.apply((cx + 10.0, cy), A4) == pytest.approx((cx + 20.0, cy))


@pytest.mark.parametrize(
    "transform",
    [
        Similarity(dx_mm=0.7, dy_mm=-0.4),
        Similarity(rotation_deg=0.35),
        Similarity(scale=1.004),
        Similarity(dx_mm=-1.2, dy_mm=0.9, rotation_deg=-0.22, scale=0.997),
    ],
)
def test_inverse_undoes_the_transform(transform):
    inverse = transform.inverse()
    for point in [(10.0, 10.0), (200.0, 287.0), (105.0, 148.5), (55.0, 240.0)]:
        moved = transform.apply(point, A4)
        back = inverse.apply(moved, A4)
        assert back == pytest.approx(point, abs=1e-9)


def test_solve_recovers_a_known_transform():
    truth = Similarity(dx_mm=0.42, dy_mm=-0.31, rotation_deg=0.18, scale=1.0021)
    nominal = [(25.0, 25.0), (185.0, 25.0), (25.0, 272.0), (185.0, 272.0), (105.0, 148.5)]
    observed = [truth.apply(p, A4) for p in nominal]

    fit = solve_similarity(nominal, observed, A4)

    assert fit.transform.dx_mm == pytest.approx(truth.dx_mm, abs=1e-6)
    assert fit.transform.dy_mm == pytest.approx(truth.dy_mm, abs=1e-6)
    assert fit.transform.rotation_deg == pytest.approx(truth.rotation_deg, abs=1e-6)
    assert fit.transform.scale == pytest.approx(truth.scale, abs=1e-9)
    assert fit.rms_residual_mm < 1e-9
    assert fit.n_points == 5


def test_solve_is_robust_to_measurement_noise():
    truth = Similarity(dx_mm=0.5, dy_mm=0.3, rotation_deg=0.1, scale=1.001)
    nominal = [(25.0, 25.0), (185.0, 25.0), (25.0, 272.0), (185.0, 272.0), (105.0, 148.5)]
    # A person reading a printed ruler resolves about a quarter millimetre.
    noise = [(0.1, -0.1), (-0.1, 0.1), (0.12, 0.08), (-0.08, -0.12), (0.05, 0.05)]
    observed = [
        (truth.apply(p, A4)[0] + nx, truth.apply(p, A4)[1] + ny)
        for p, (nx, ny) in zip(nominal, noise)
    ]

    fit = solve_similarity(nominal, observed, A4)

    assert fit.transform.dx_mm == pytest.approx(truth.dx_mm, abs=0.15)
    assert fit.transform.dy_mm == pytest.approx(truth.dy_mm, abs=0.15)
    assert fit.rms_residual_mm < 0.2


def test_solve_rejects_degenerate_input():
    with pytest.raises(ValueError, match="at least 2"):
        solve_similarity([(1.0, 1.0)], [(1.0, 1.0)], A4)
    with pytest.raises(ValueError, match="coincident"):
        solve_similarity([(5.0, 5.0)] * 3, [(5.0, 5.0)] * 3, A4)


def test_pdf_matrix_matches_page_space_translation():
    """The PDF matrix must reproduce apply(), including the y-axis flip."""
    transform = Similarity(dx_mm=2.0, dy_mm=3.0)
    matrix = transform.to_pdf_matrix(A4)

    point_mm = (50.0, 80.0)
    expected_mm = transform.apply(point_mm, A4)

    x_pt, y_pt = mm_to_pt(point_mm[0]), A4.height_pt - mm_to_pt(point_mm[1])
    got_x = matrix.a * x_pt + matrix.c * y_pt + matrix.e
    got_y = matrix.b * x_pt + matrix.d * y_pt + matrix.f

    assert pt_to_mm(got_x) == pytest.approx(expected_mm[0], abs=1e-6)
    assert pt_to_mm(A4.height_pt - got_y) == pytest.approx(expected_mm[1], abs=1e-6)


def test_pdf_matrix_matches_page_space_rotation_and_scale():
    transform = Similarity(dx_mm=-0.6, dy_mm=0.4, rotation_deg=0.75, scale=1.003)
    matrix = transform.to_pdf_matrix(A4)

    for point_mm in [(20.0, 20.0), (190.0, 30.0), (105.0, 148.5), (40.0, 270.0)]:
        expected = transform.apply(point_mm, A4)
        x_pt, y_pt = mm_to_pt(point_mm[0]), A4.height_pt - mm_to_pt(point_mm[1])
        got_x = matrix.a * x_pt + matrix.c * y_pt + matrix.e
        got_y = matrix.b * x_pt + matrix.d * y_pt + matrix.f
        assert pt_to_mm(got_x) == pytest.approx(expected[0], abs=1e-6)
        assert pt_to_mm(A4.height_pt - got_y) == pytest.approx(expected[1], abs=1e-6)


def test_identity_detection():
    assert Similarity().is_identity
    assert not Similarity(dx_mm=0.01).is_identity
    assert "identity" in Similarity().describe()


def test_describe_reports_direction():
    text = Similarity(dx_mm=0.4, dy_mm=-0.2, rotation_deg=0.1, scale=1.002).describe()
    assert "+0.40" in text and "-0.20" in text
    assert "cw" in text
    assert "%" in text


def test_rotation_sign_survives_the_pdf_flip():
    """A clockwise page rotation is counter-clockwise in y-up PDF space."""
    matrix = Similarity(rotation_deg=1.0).to_pdf_matrix(A4)
    assert math.degrees(math.atan2(matrix.b, matrix.a)) == pytest.approx(-1.0, abs=1e-9)
