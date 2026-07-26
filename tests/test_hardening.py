"""Regressions for the ways Onionskin could damage or expose a user's files.

A local tool gets pointed at people's real documents. Destroying one, or
leaving it readable to everyone on a shared machine, is worse than any
misprint — you cannot un-delete a contract.
"""

import os
import stat
import sys

import numpy as np
import pytest
from PIL import Image, ImageFilter

from onionskin import calibrate, compose, pipeline
from onionskin.diff import _dilate
from onionskin.render import DocumentError

from helpers import make_pdf


# --- never overwrite an input ----------------------------------------------


def test_delta_refuses_to_overwrite_the_original(tmp_path):
    original = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "ORIGINAL")])
    edited = make_pdf(tmp_path / "b.pdf", [(20.0, 40.0, "ORIGINAL"), (60.0, 150.0, "x")])
    before = original.read_bytes()

    with pytest.raises(DocumentError, match="refusing to write"):
        pipeline.run(original, edited, original, pipeline.Options(dpi=150))

    assert original.read_bytes() == before


def test_delta_refuses_to_overwrite_the_edited_copy(tmp_path):
    original = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "ORIGINAL")])
    edited = make_pdf(tmp_path / "b.pdf", [(20.0, 40.0, "ORIGINAL"), (60.0, 150.0, "x")])
    before = edited.read_bytes()

    with pytest.raises(DocumentError, match="refusing to write"):
        pipeline.run(original, edited, edited, pipeline.Options(dpi=150))

    assert edited.read_bytes() == before


def test_add_refuses_to_overwrite_its_source(tmp_path):
    source = make_pdf(tmp_path / "form.pdf", [(20.0, 40.0, "PRECIOUS")])
    before = source.read_bytes()

    with pytest.raises(DocumentError, match="refusing to write"):
        pipeline.compose_run(
            source,
            [compose.TextBox(page=0, x_mm=60.0, y_mm=150.0, text="hi")],
            source,
            pipeline.Options(dpi=150),
        )

    assert source.read_bytes() == before


def test_the_guard_is_not_fooled_by_a_roundabout_path(tmp_path):
    original = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "ORIGINAL")])
    edited = make_pdf(tmp_path / "b.pdf", [(20.0, 40.0, "ORIGINAL"), (60.0, 150.0, "x")])
    sneaky = tmp_path / "sub" / ".." / "a.pdf"
    (tmp_path / "sub").mkdir()

    with pytest.raises(DocumentError, match="refusing to write"):
        pipeline.run(original, edited, sneaky, pipeline.Options(dpi=150))


def test_a_different_output_is_fine(tmp_path):
    original = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "ORIGINAL")])
    edited = make_pdf(tmp_path / "b.pdf", [(20.0, 40.0, "ORIGINAL"), (60.0, 150.0, "x")])
    result = pipeline.run(original, edited, tmp_path / "d.pdf", pipeline.Options(dpi=150))
    assert result.output.is_file()


# --- keep the user's documents to themselves --------------------------------


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX permission bits")
def test_calibration_profiles_are_private(onionskin_home):
    from onionskin.geometry import Similarity

    calibrate.save_profile(calibrate.Profile(name="p", error=Similarity(dx_mm=0.1)))

    directory = stat.S_IMODE(os.stat(calibrate.profiles_dir()).st_mode)
    profile = stat.S_IMODE(os.stat(calibrate.profile_path("p")).st_mode)
    assert directory & 0o077 == 0
    assert profile & 0o077 == 0


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX permission bits")
def test_uploaded_documents_are_not_world_readable(tmp_path):
    """A shared machine lists /tmp freely; the job id is no protection there."""
    fastapi = pytest.importorskip("fastapi")
    from fastapi.testclient import TestClient

    from onionskin.web.app import create_app, jobs_root

    original = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "x")])
    edited = make_pdf(tmp_path / "b.pdf", [(20.0, 40.0, "x"), (60.0, 150.0, "y")])

    client = TestClient(create_app())
    with original.open("rb") as f1, edited.open("rb") as f2:
        job = client.post(
            "/api/delta",
            files={"original": ("a.pdf", f1), "edited": ("b.pdf", f2)},
            data={"dpi": "150"},
        ).json()["job"]

    root = jobs_root() / job
    assert stat.S_IMODE(os.stat(jobs_root()).st_mode) & 0o077 == 0
    for path in root.rglob("*"):
        assert stat.S_IMODE(os.stat(path).st_mode) & 0o077 == 0, f"{path} is exposed"


# --- the optimisation must not change the answer ----------------------------


@pytest.mark.parametrize("radius", [1, 2, 3, 5])
@pytest.mark.parametrize("shape", [(200, 300), (64, 64), (17, 5)])
def test_separable_dilation_matches_a_square_max_filter(radius, shape):
    """Speed is only worth having if the result is identical."""
    rng = np.random.default_rng(7)
    mask = rng.random(shape) < 0.05

    expected = np.asarray(
        Image.fromarray((mask * 255).astype(np.uint8), "L").filter(
            ImageFilter.MaxFilter(2 * radius + 1)
        )
    ) > 0

    assert np.array_equal(_dilate(mask, radius), expected)


def test_dilation_of_nothing_is_nothing():
    assert not _dilate(np.zeros((50, 50), dtype=bool), 3).any()


def test_zero_radius_leaves_the_mask_alone():
    rng = np.random.default_rng(1)
    mask = rng.random((40, 40)) < 0.2
    assert np.array_equal(_dilate(mask, 0), mask)


def test_dilation_reaches_the_edges():
    mask = np.zeros((20, 20), dtype=bool)
    mask[0, 0] = True
    grown = _dilate(mask, 2)
    assert grown[0:3, 0:3].all()
    assert not grown[5, 5]


# --- every printer's paper --------------------------------------------------


@pytest.mark.parametrize(
    "spec, width, height",
    [
        ("a4", 210.0, 297.0), ("a3", 297.0, 420.0), ("a5", 148.0, 210.0),
        ("legal", 215.9, 355.6), ("tabloid", 279.4, 431.8),
        ("executive", 184.15, 266.7), ("LETTER", 215.9, 279.4),
        ("210x297", 210.0, 297.0), ("100*150", 100.0, 150.0),
        ("  a4  ", 210.0, 297.0),
    ],
)
def test_page_sizes_by_name_or_measurement(spec, width, height):
    size = calibrate.parse_page(spec)
    assert size.width_mm == pytest.approx(width)
    assert size.height_mm == pytest.approx(height)


@pytest.mark.parametrize("spec", ["", "nonsense", "10x10", "9000x9000", "a4x", "1x"])
def test_impossible_page_sizes_are_refused(spec):
    with pytest.raises(ValueError):
        calibrate.parse_page(spec)


@pytest.mark.parametrize("spec", ["a6", "a5", "a4", "legal", "tabloid", "150x200"])
def test_a_target_can_be_made_for_any_of_them(tmp_path, spec):
    from onionskin.render import Document

    size = calibrate.parse_page(spec)
    out = calibrate.make_target(tmp_path / "t.pdf", size)

    with Document(out) as doc:
        assert doc.page_sizes[0].width_mm == pytest.approx(size.width_mm, abs=0.2)
        assert doc.page_sizes[0].height_mm == pytest.approx(size.height_mm, abs=0.2)


def test_small_sheets_pull_the_fiducials_in():
    """25 mm of inset would hang the scales off the edge of a small sheet.

    A 100 mm sheet still takes the full 25 mm — a quarter of the short side —
    so the threshold only bites below that.
    """
    assert calibrate.default_inset(calibrate.A4) == pytest.approx(25.0)
    assert calibrate.default_inset(calibrate.parse_page("100x150")) == pytest.approx(25.0)

    smallest = calibrate.parse_page("90x120")
    assert calibrate.default_inset(smallest) < 25.0
    assert calibrate.default_inset(smallest) >= 15.0


def test_solving_uses_the_same_inset_the_target_was_drawn_with():
    """A mismatch here would silently corrupt the fitted rotation and scale."""
    from onionskin.geometry import Similarity

    page = calibrate.parse_page("100x150")
    truth = Similarity(dx_mm=0.3, rotation_deg=0.2, scale=1.001)
    points = calibrate.fiducials(page, calibrate.default_inset(page))
    offsets = [
        (i, truth.apply(p, page)[0] - p[0], truth.apply(p, page)[1] - p[1])
        for i, p in enumerate(points, start=1)
    ]

    fit = calibrate.solve_from_offsets(offsets, page)

    assert fit.transform.rotation_deg == pytest.approx(0.2, abs=1e-6)
    assert fit.transform.scale == pytest.approx(1.001, abs=1e-9)


def test_a_profile_from_another_paper_size_warns(tmp_path, onionskin_home):
    from onionskin.geometry import Similarity

    calibrate.save_profile(
        calibrate.Profile(
            name="a4rot", error=Similarity(dx_mm=0.4, rotation_deg=0.3),
            page=calibrate.A4,
        )
    )
    source = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "x")],
                      page=calibrate.parse_page("legal"))

    result = pipeline.compose_run(
        source, [compose.TextBox(page=0, x_mm=40, y_mm=80, text="hi")],
        tmp_path / "d.pdf", pipeline.Options(dpi=150, profile="a4rot"),
    )

    assert any(c.code == "profile_page_mismatch" for c in result.checks)


def test_a_shift_only_profile_transfers_to_any_paper(tmp_path, onionskin_home):
    """The paper path pushes every sheet the same way, so a shift carries over."""
    from onionskin.geometry import Similarity

    calibrate.save_profile(
        calibrate.Profile(name="shift", error=Similarity(dx_mm=0.4),
                          page=calibrate.A4)
    )
    source = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "x")],
                      page=calibrate.parse_page("legal"))

    result = pipeline.compose_run(
        source, [compose.TextBox(page=0, x_mm=40, y_mm=80, text="hi")],
        tmp_path / "d.pdf", pipeline.Options(dpi=150, profile="shift"),
    )

    assert not any(c.code == "profile_page_mismatch" for c in result.checks)


# --- the delta carries ink, not empty page ----------------------------------


def test_the_embedded_image_is_cropped_to_the_ink(tmp_path):
    """Encoding a full page to say "three words here" dominated the run time
    and bloated the file that has to travel to the printer."""
    import pikepdf

    original = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "base")])
    edited = make_pdf(tmp_path / "b.pdf", [(20.0, 40.0, "base"), (60.0, 150.0, "new")])

    result = pipeline.run(original, edited, tmp_path / "d.pdf",
                          pipeline.Options(dpi=300))

    with pikepdf.open(result.output) as pdf:
        image = list(pdf.pages[0].resources.XObject.values())[0]
        width, height = int(image.Width), int(image.Height)

    full_page_px = (210 / 25.4 * 300) * (297 / 25.4 * 300)
    assert width * height < full_page_px * 0.05, "the image should be a crop"
    assert width > 10 and height > 10


def test_cropping_does_not_move_the_ink(tmp_path):
    """The crop is an optimisation; it must be invisible in the output."""
    from helpers import ink_bbox_mm

    original = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "base")])
    edited = make_pdf(tmp_path / "b.pdf", [(20.0, 40.0, "base"), (60.0, 150.0, "new")])

    result = pipeline.run(original, edited, tmp_path / "d.pdf",
                          pipeline.Options(dpi=300))

    region = result.pages[0].added_regions[0]
    x0, y0, x1, y1 = ink_bbox_mm(result.output, 300.0)
    assert x0 == pytest.approx(region.x0_mm, abs=0.3)
    assert y0 == pytest.approx(region.y0_mm, abs=0.3)
    assert x1 == pytest.approx(region.x1_mm, abs=0.3)
    assert y1 == pytest.approx(region.y1_mm, abs=0.3)


def test_ink_at_the_very_edge_survives_the_crop(tmp_path):
    """The crop pads by a pixel; ink in the first row must not be clipped."""
    from helpers import ink_bbox_mm

    original = make_pdf(tmp_path / "a.pdf", [(100.0, 150.0, "base")])
    edited = make_pdf(tmp_path / "b.pdf", [(100.0, 150.0, "base"), (2.0, 6.0, "E")])

    result = pipeline.run(original, edited, tmp_path / "d.pdf",
                          pipeline.Options(dpi=300))

    assert ink_bbox_mm(result.output, 300.0) is not None


# --- unreadable PDFs must explain themselves, not traceback -----------------


def build_hostile(tmp_path):
    """Every way a .pdf can arrive broken, short of being fine."""
    import zipfile

    import pikepdf

    from helpers import make_docx

    good = make_pdf(tmp_path / "good.pdf", [(20.0, 40.0, "base")])
    files = {}

    with pikepdf.open(good) as pdf:
        pdf.save(tmp_path / "encrypted.pdf",
                 encryption=pikepdf.Encryption(owner="x", user="x"))
    files["encrypted"] = tmp_path / "encrypted.pdf"

    raw = good.read_bytes()
    (tmp_path / "truncated.pdf").write_bytes(raw[: len(raw) // 2])
    files["truncated"] = tmp_path / "truncated.pdf"

    (tmp_path / "empty.pdf").write_bytes(b"")
    files["empty"] = tmp_path / "empty.pdf"

    make_docx(tmp_path / "real.docx", ["hello"])
    (tmp_path / "mislabelled.pdf").write_bytes((tmp_path / "real.docx").read_bytes())
    files["mislabelled"] = tmp_path / "mislabelled.pdf"

    (tmp_path / "garbage.pdf").write_bytes(b"%PDF-1.4\nnot really\n%%EOF\n")
    files["garbage"] = tmp_path / "garbage.pdf"

    return good, files


@pytest.mark.parametrize(
    "kind",
    ["encrypted", "truncated", "empty", "mislabelled", "garbage"],
)
def test_an_unreadable_pdf_is_explained_not_crashed(tmp_path, kind):
    """pdfium raises its own exception type, which nothing was catching — a
    non-technical user saw a Python traceback."""
    good, files = build_hostile(tmp_path)
    bad = files[kind]

    with pytest.raises(DocumentError) as exc:
        pipeline.run(bad, good, tmp_path / "d.pdf", pipeline.Options(dpi=150))
    assert bad.name in str(exc.value)

    # ...whichever side it arrives on, and through compose too.
    with pytest.raises(DocumentError):
        pipeline.run(good, bad, tmp_path / "d.pdf", pipeline.Options(dpi=150))
    with pytest.raises(DocumentError):
        pipeline.compose_run(
            bad, [compose.TextBox(page=0, text="hi")], tmp_path / "d.pdf",
            pipeline.Options(dpi=150),
        )


def test_an_encrypted_pdf_says_so(tmp_path):
    good, files = build_hostile(tmp_path)
    with pytest.raises(DocumentError, match="password-protected"):
        pipeline.run(files["encrypted"], good, tmp_path / "d.pdf",
                     pipeline.Options(dpi=150))


def test_an_empty_pdf_says_so(tmp_path):
    good, files = build_hostile(tmp_path)
    with pytest.raises(DocumentError, match="empty"):
        pipeline.run(files["empty"], good, tmp_path / "d.pdf",
                     pipeline.Options(dpi=150))


@pytest.mark.parametrize("kind", ["encrypted", "truncated", "empty", "garbage"])
def test_the_cli_reports_unreadable_pdfs_cleanly(tmp_path, capsys, kind):
    from onionskin.cli import main

    good, files = build_hostile(tmp_path)
    code = main(["delta", str(files[kind]), str(good), "-o", str(tmp_path / "d.pdf")])

    assert code == 1
    captured = capsys.readouterr()
    assert captured.err.startswith("error:")
    assert "Traceback" not in captured.err and "Traceback" not in captured.out
