"""Pages that are not the simple case.

A page is not always "a box starting at (0,0), the right way up". Media boxes
have non-zero origins, crop boxes shrink the visible area, and ``/Rotate`` turns
the page a quarter turn — all three are ordinary in the wild, and all three move
where ink lands on paper.

Onionskin renders and diffs in display space, then writes the delta back into
the source's own frame. If it did not, the delta would print somewhere other
than where the preview showed it, which is the worst thing this app can do.
"""

import pikepdf
import pytest

from onionskin import compose, pipeline
from onionskin.render import Document, DocumentError, PageFrame

from helpers import A4, ink_bbox_mm, make_pdf

DPI = 200.0
A4_PT = (595.276, 841.89)


def mutate(src, dst, **changes):
    with pikepdf.open(src) as pdf:
        page = pdf.pages[0]
        for key, value in changes.items():
            setattr(page, key, value)
        pdf.save(dst)
    return dst


def offset_page(tmp_path, items, shift_pt=28.35, name="offset.pdf"):
    src = make_pdf(tmp_path / f"src-{name}", items)
    with pikepdf.open(src) as pdf:
        box = [float(v) for v in pdf.pages[0].MediaBox]
        pdf.pages[0].MediaBox = [v + shift_pt for v in box]
        pdf.save(tmp_path / name)
    return tmp_path / name


# --- reading the frame -----------------------------------------------------


def test_a_plain_page_is_simple(tmp_path):
    with Document(make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "x")])) as doc:
        frame = doc.frames[0]
    assert frame.is_simple
    assert frame.rotate == 0
    assert frame.describe() == "standard"


def test_rotation_swaps_the_display_size(tmp_path):
    src = mutate(make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "x")]),
                 tmp_path / "r.pdf", Rotate=90)
    with Document(src) as doc:
        assert doc.page_sizes[0].width_mm == pytest.approx(297.0, abs=0.5)
        assert doc.page_sizes[0].height_mm == pytest.approx(210.0, abs=0.5)
        assert not doc.frames[0].is_simple


def test_rotation_inherited_from_the_page_tree_is_seen(tmp_path):
    """/Rotate may live on the Pages node rather than the page itself.

    The page's own /Rotate wins when present, so it has to be removed for the
    inherited value to apply at all — which is exactly the shape a document
    written by a tool that rotates whole files takes.
    """
    src = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "x")])
    with pikepdf.open(src) as pdf:
        del pdf.pages[0].Rotate
        pdf.Root.Pages.Rotate = 90
        pdf.save(tmp_path / "inherited.pdf")

    with Document(tmp_path / "inherited.pdf") as doc:
        assert doc.frames[0].rotate == 90
        assert doc.page_sizes[0].width_mm == pytest.approx(297.0, abs=0.5)


def test_a_crop_box_defines_the_visible_page(tmp_path):
    src = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "x")])
    cropped = mutate(src, tmp_path / "c.pdf", CropBox=[28.35, 28.35, 566.9, 813.5])
    with Document(cropped) as doc:
        assert doc.page_sizes[0].width_mm == pytest.approx(190.0, abs=0.5)
        assert not doc.frames[0].is_simple


def test_a_crop_box_outside_the_media_box_is_ignored(tmp_path):
    """A degenerate crop must not produce a zero-sized page."""
    src = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "x")])
    broken = mutate(src, tmp_path / "c.pdf", CropBox=[9000, 9000, 9100, 9100])
    with Document(broken) as doc:
        assert doc.page_sizes[0].width_mm == pytest.approx(210.0, abs=0.5)


def test_rotation_normalises_beyond_a_full_turn(tmp_path):
    src = mutate(make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "x")]),
                 tmp_path / "r.pdf", Rotate=450)
    with Document(src) as doc:
        assert doc.frames[0].rotate == 90


def test_a_rotation_that_is_not_a_quarter_turn_is_refused(tmp_path):
    src = mutate(make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "x")]),
                 tmp_path / "r.pdf", Rotate=45)
    with pytest.raises(DocumentError, match="multiple of 90"):
        Document(src)


def test_frame_describes_what_is_unusual():
    frame = PageFrame(media=(0, 0, 595, 842), crop=(10, 10, 500, 800), rotate=90)
    text = frame.describe()
    assert "rotated 90" in text and "cropped" in text


# --- the delta must land in the source's frame -----------------------------


def compose_at(src, x_mm, y_mm, out, text="XXXX"):
    pipeline.compose_run(
        src,
        [compose.TextBox(page=0, x_mm=x_mm, y_mm=y_mm, text=text, size_pt=10)],
        out,
        pipeline.Options(dpi=DPI),
    )
    return out


def geometry_of(path):
    with pikepdf.open(path) as pdf:
        page = pdf.pages[0]
        return (
            [round(float(v), 1) for v in page.MediaBox],
            int(page.get("/Rotate") or 0),
        )


@pytest.mark.parametrize("rotate", [0, 90, 180, 270])
def test_composed_text_lands_where_asked_on_a_rotated_page(tmp_path, rotate):
    """The single most important property: what the preview shows is what prints."""
    src = mutate(
        make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "base")]),
        tmp_path / f"r{rotate}.pdf",
        Rotate=rotate,
    )
    out = compose_at(src, 60.0, 150.0, tmp_path / f"d{rotate}.pdf")

    x0, y0, _, _ = ink_bbox_mm(out, 300.0)
    assert x0 == pytest.approx(60.0, abs=1.0)
    assert y0 == pytest.approx(150.0, abs=1.5)


def test_composed_text_lands_where_asked_with_an_offset_media_box(tmp_path):
    src = offset_page(tmp_path, [(20.0, 40.0, "base")])
    out = compose_at(src, 60.0, 150.0, tmp_path / "d.pdf")

    x0, y0, _, _ = ink_bbox_mm(out, 300.0)
    assert x0 == pytest.approx(60.0, abs=1.0)
    assert y0 == pytest.approx(150.0, abs=1.5)


def test_composed_text_lands_where_asked_on_a_cropped_page(tmp_path):
    src = mutate(make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "base")]),
                 tmp_path / "c.pdf", CropBox=[28.35, 28.35, 566.9, 813.5])
    out = compose_at(src, 60.0, 150.0, tmp_path / "d.pdf")

    x0, y0, _, _ = ink_bbox_mm(out, 300.0)
    assert x0 == pytest.approx(60.0, abs=1.0)
    assert y0 == pytest.approx(150.0, abs=1.5)


@pytest.mark.parametrize("rotate", [0, 90, 180, 270])
def test_delta_copies_the_source_page_geometry(tmp_path, rotate):
    """A driver places a page from its boxes and /Rotate. If the delta disagrees
    with the sheet about any of them, no amount of calibration can save it."""
    src = mutate(make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "base")]),
                 tmp_path / f"r{rotate}.pdf", Rotate=rotate)
    out = compose_at(src, 60.0, 150.0, tmp_path / f"d{rotate}.pdf")
    assert geometry_of(out) == geometry_of(src)


def test_delta_copies_an_offset_media_box(tmp_path):
    src = offset_page(tmp_path, [(20.0, 40.0, "base")])
    out = compose_at(src, 60.0, 150.0, tmp_path / "d.pdf")
    assert geometry_of(out) == geometry_of(src)


def test_a_plain_page_delta_is_left_untouched(tmp_path):
    """The simple case must not pay for the complicated one."""
    src = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "base")])
    out = compose_at(src, 60.0, 150.0, tmp_path / "d.pdf")
    media, rotate = geometry_of(out)
    assert rotate == 0
    assert media[0] == 0.0 and media[1] == 0.0


# --- the diff path, same requirement ---------------------------------------


@pytest.mark.parametrize("rotate", [90, 270])
def test_diff_delta_aligns_on_a_rotated_page(tmp_path, rotate):
    base = [(20.0, 40.0, "base")]
    original = mutate(make_pdf(tmp_path / "a0.pdf", base),
                      tmp_path / "a.pdf", Rotate=rotate)
    edited = mutate(make_pdf(tmp_path / "b0.pdf", base + [(60.0, 150.0, "ADDED")]),
                    tmp_path / "b.pdf", Rotate=rotate)

    result = pipeline.run(original, edited, tmp_path / "d.pdf",
                          pipeline.Options(dpi=DPI))

    region = result.pages[0].added_regions[0]
    assert ink_bbox_mm(result.output, 300.0)[0] == pytest.approx(region.x0_mm, abs=1.0)
    assert geometry_of(result.output) == geometry_of(original)


def test_diff_delta_aligns_with_an_offset_media_box(tmp_path):
    base = [(20.0, 40.0, "base")]
    original = offset_page(tmp_path, base, name="a.pdf")
    edited = offset_page(tmp_path, base + [(60.0, 150.0, "ADDED")], name="b.pdf")

    result = pipeline.run(original, edited, tmp_path / "d.pdf",
                          pipeline.Options(dpi=DPI))

    region = result.pages[0].added_regions[0]
    assert ink_bbox_mm(result.output, 300.0)[0] == pytest.approx(region.x0_mm, abs=1.0)
    assert geometry_of(result.output) == geometry_of(original)


def test_calibration_and_frame_conforming_compose(tmp_path, onionskin_home):
    """The correction happens in display space, the conforming after it. Both
    have to survive together."""
    from onionskin import calibrate
    from onionskin.geometry import Similarity

    calibrate.save_profile(
        calibrate.Profile(name="p", error=Similarity(dx_mm=2.0), page=A4)
    )
    src = mutate(make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "base")]),
                 tmp_path / "r.pdf", Rotate=90)

    plain = compose_at(src, 60.0, 150.0, tmp_path / "plain.pdf")
    fixed = pipeline.compose_run(
        src,
        [compose.TextBox(page=0, x_mm=60.0, y_mm=150.0, text="XXXX", size_pt=10)],
        tmp_path / "fixed.pdf",
        pipeline.Options(dpi=DPI, profile="p"),
    )

    before = ink_bbox_mm(plain, 300.0)
    after = ink_bbox_mm(fixed.output, 300.0)
    assert after[0] == pytest.approx(before[0] - 2.0, abs=0.4)
    assert geometry_of(fixed.output) == geometry_of(src)
