import pytest

from onionskin import calibrate
from onionskin.geometry import PageSize, Similarity
from onionskin.render import Document

from helpers import ink_bbox_mm

A4 = calibrate.A4


# --- the target page -------------------------------------------------------


def test_target_is_one_page_of_the_right_size(tmp_path):
    out = calibrate.make_target(tmp_path / "target.pdf", A4)
    with Document(out) as doc:
        assert len(doc) == 1
        assert doc.page_sizes[0].width_mm == pytest.approx(210.0, abs=0.1)


def test_target_puts_fiducials_near_the_corners(tmp_path):
    out = calibrate.make_target(tmp_path / "target.pdf", A4, inset_mm=25.0)
    x0, y0, x1, y1 = ink_bbox_mm(out, 200.0)
    # Crosshair arms reach 5 mm, rulers a little further.
    assert x0 < 22.0 and y0 < 22.0
    assert x1 > 188.0 and y1 > 275.0


def test_fiducials_are_spread_across_the_page():
    """Rotation and scale are unobservable from clustered points."""
    points = calibrate.fiducials(A4, inset_mm=25.0)
    assert len(points) == 5
    xs = [p[0] for p in points]
    ys = [p[1] for p in points]
    assert max(xs) - min(xs) > 150.0
    assert max(ys) - min(ys) > 200.0


def test_letter_target_uses_letter_geometry(tmp_path):
    out = calibrate.make_target(tmp_path / "t.pdf", calibrate.LETTER)
    with Document(out) as doc:
        assert doc.page_sizes[0].width_mm == pytest.approx(215.9, abs=0.2)


# --- parsing measurements --------------------------------------------------


def test_parse_point_reads_a_signed_offset():
    assert calibrate.parse_point("P1:+0.40,-0.15") == (1, 0.40, -0.15)
    assert calibrate.parse_point("3:0.25,0.5") == (3, 0.25, 0.5)
    assert calibrate.parse_point(" p2 : -1.0 , 2.0 ") == (2, -1.0, 2.0)


@pytest.mark.parametrize("bad", ["P1", "P1:0.4", "Px:0.1,0.2", "P1:a,b", "P1:1,2,3"])
def test_parse_point_rejects_nonsense(bad):
    with pytest.raises(ValueError):
        calibrate.parse_point(bad)


# --- solving ---------------------------------------------------------------


def test_solve_recovers_a_pure_shift():
    offsets = [(i, 0.5, -0.3) for i in range(1, 6)]
    fit = calibrate.solve_from_offsets(offsets, A4)
    assert fit.transform.dx_mm == pytest.approx(0.5, abs=1e-9)
    assert fit.transform.dy_mm == pytest.approx(-0.3, abs=1e-9)
    assert fit.transform.rotation_deg == pytest.approx(0.0, abs=1e-9)
    assert fit.transform.scale == pytest.approx(1.0, abs=1e-12)


def test_solve_recovers_rotation_and_scale_from_readings():
    truth = Similarity(dx_mm=0.3, dy_mm=-0.2, rotation_deg=0.25, scale=1.0015)
    points = calibrate.fiducials(A4)
    offsets = []
    for i, point in enumerate(points, start=1):
        moved = truth.apply(point, A4)
        offsets.append((i, moved[0] - point[0], moved[1] - point[1]))

    fit = calibrate.solve_from_offsets(offsets, A4)

    assert fit.transform.rotation_deg == pytest.approx(0.25, abs=1e-6)
    assert fit.transform.scale == pytest.approx(1.0015, abs=1e-9)
    assert fit.max_residual_mm < 1e-6


def test_solve_rejects_an_unknown_fiducial():
    with pytest.raises(ValueError, match="not on the target"):
        calibrate.solve_from_offsets([(9, 0.1, 0.1), (1, 0.1, 0.1)], A4)


def test_two_points_are_enough_to_solve():
    offsets = [(1, 0.4, 0.2), (4, 0.4, 0.2)]
    fit = calibrate.solve_from_offsets(offsets, A4)
    assert fit.n_points == 2
    assert fit.transform.dx_mm == pytest.approx(0.4, abs=1e-9)


# --- profiles --------------------------------------------------------------


def test_profile_round_trips_through_disk(onionskin_home):
    profile = calibrate.Profile(
        name="office",
        error=Similarity(dx_mm=0.4, dy_mm=-0.2, rotation_deg=0.1, scale=1.001),
        page=A4,
        rms_residual_mm=0.05,
        max_residual_mm=0.09,
        n_points=5,
        notes="tray 2",
    )
    calibrate.save_profile(profile)

    loaded = calibrate.load_profile("office")

    assert loaded.error.dx_mm == pytest.approx(0.4)
    assert loaded.error.rotation_deg == pytest.approx(0.1)
    assert loaded.n_points == 5
    assert loaded.notes == "tray 2"


def test_correction_is_the_inverse_of_the_measured_error(onionskin_home):
    """This is the whole contract: correct by the opposite of what was measured."""
    error = Similarity(dx_mm=0.6, dy_mm=-0.4, rotation_deg=0.2, scale=1.002)
    profile = calibrate.Profile(name="p", error=error, page=A4)

    for point in [(25.0, 25.0), (185.0, 272.0), (105.0, 148.5)]:
        # Apply the correction, then let the printer apply its error.
        corrected = profile.correction.apply(point, A4)
        landed = error.apply(corrected, A4)
        assert landed == pytest.approx(point, abs=1e-9)


def test_profiles_are_stored_under_onionskin_home(onionskin_home):
    calibrate.save_profile(calibrate.Profile(name="x", error=Similarity()))
    assert (onionskin_home / "profiles" / "x.json").is_file()


def test_missing_profile_names_the_alternatives(onionskin_home):
    calibrate.save_profile(calibrate.Profile(name="office", error=Similarity()))
    with pytest.raises(FileNotFoundError, match="office"):
        calibrate.load_profile("home")


def test_profile_names_are_sanitised(onionskin_home):
    calibrate.save_profile(calibrate.Profile(name="../../evil", error=Similarity()))
    written = list((onionskin_home / "profiles").glob("*.json"))
    assert len(written) == 1
    assert ".." not in written[0].name


def test_listing_and_deleting(onionskin_home):
    for name in ("a", "b"):
        calibrate.save_profile(calibrate.Profile(name=name, error=Similarity()))
    assert {p.name for p in calibrate.list_profiles()} == {"a", "b"}

    assert calibrate.delete_profile("a") is True
    assert calibrate.delete_profile("a") is False
    assert {p.name for p in calibrate.list_profiles()} == {"b"}


def test_corrupt_profiles_are_skipped_not_fatal(onionskin_home):
    calibrate.save_profile(calibrate.Profile(name="good", error=Similarity()))
    (onionskin_home / "profiles" / "broken.json").write_text("{not json")
    assert {p.name for p in calibrate.list_profiles()} == {"good"}


def test_describe_shows_both_error_and_correction(onionskin_home):
    profile = calibrate.Profile(
        name="office", error=Similarity(dx_mm=0.4), page=A4, n_points=5,
        rms_residual_mm=0.03, max_residual_mm=0.06,
    )
    text = profile.describe()
    assert "printer error" in text and "correction" in text
    assert "+0.40" in text and "-0.40" in text
