import json
from pathlib import Path

import pytest

from onionskin import calibrate, pipeline
from onionskin.compose import (
    Composition,
    LayoutError,
    TextBox,
    compose,
    load_layout,
    parse_text_spec,
    save_layout,
)
from onionskin.geometry import Similarity

from helpers import A4, ink_bbox_mm, make_docx, make_pdf, requires_soffice

DPI = 250.0


def opts(**kwargs):
    kwargs.setdefault("dpi", DPI)
    return pipeline.Options(**kwargs)


# --- placement -------------------------------------------------------------


def test_text_lands_where_it_was_asked_for(tmp_path):
    box = TextBox(page=0, x_mm=60.0, y_mm=150.0, text="Approved", size_pt=12.0)
    out = compose(Composition([A4], [box]), tmp_path / "delta.pdf")

    x0, y0, x1, y1 = ink_bbox_mm(out, 400.0)
    assert x0 == pytest.approx(60.0, abs=0.6)
    # y_mm is the top of the block, so ink starts just below it.
    assert y0 == pytest.approx(150.0, abs=1.2)
    assert y0 >= 150.0 - 0.2


def test_page_size_is_preserved(tmp_path):
    out = compose(
        Composition([A4], [TextBox(text="x", x_mm=50, y_mm=50)]), tmp_path / "d.pdf"
    )
    import pikepdf

    with pikepdf.open(out) as pdf:
        box = [float(v) for v in pdf.pages[0].MediaBox]
    assert box[2] == pytest.approx(A4.width_pt, abs=0.5)


def test_text_goes_on_the_page_it_was_given(tmp_path):
    boxes = [TextBox(page=1, x_mm=40.0, y_mm=100.0, text="second page only")]
    out = compose(Composition([A4, A4, A4], boxes), tmp_path / "d.pdf")

    assert ink_bbox_mm(out, 200.0, page=0) is None
    assert ink_bbox_mm(out, 200.0, page=1) is not None
    assert ink_bbox_mm(out, 200.0, page=2) is None


def test_pages_without_boxes_are_blank(tmp_path):
    out = compose(
        Composition([A4, A4], [TextBox(page=0, text="only here", x_mm=30, y_mm=30)]),
        tmp_path / "d.pdf",
    )
    assert ink_bbox_mm(out, 200.0, page=1) is None


def test_larger_type_makes_larger_ink(tmp_path):
    small = compose(
        Composition([A4], [TextBox(text="Onionskin", x_mm=30, y_mm=100, size_pt=8)]),
        tmp_path / "small.pdf",
    )
    large = compose(
        Composition([A4], [TextBox(text="Onionskin", x_mm=30, y_mm=100, size_pt=24)]),
        tmp_path / "large.pdf",
    )
    small_box = ink_bbox_mm(small, 300.0)
    large_box = ink_bbox_mm(large, 300.0)
    assert (large_box[2] - large_box[0]) > 2.5 * (small_box[2] - small_box[0])


def test_wrapping_respects_the_width(tmp_path):
    long_text = "The quarterly figures have been reviewed and are approved for filing."
    box = TextBox(text=long_text, x_mm=20.0, y_mm=100.0, width_mm=50.0, size_pt=10.0)

    assert len(box.lines()) > 1
    out = compose(Composition([A4], [box]), tmp_path / "d.pdf")
    x0, y0, x1, y1 = ink_bbox_mm(out, 300.0)
    assert x1 - x0 <= 50.5
    assert y1 - y0 > box.line_height_mm  # more than one line of ink


def test_explicit_newlines_make_separate_lines():
    box = TextBox(text="first\nsecond\nthird")
    assert box.lines() == ["first", "second", "third"]


def test_unwrapped_text_stays_on_one_line():
    box = TextBox(text="a fairly long single line of text with no wrap width")
    assert len(box.lines()) == 1


def test_alignment_shifts_the_block(tmp_path):
    common = dict(text="right", x_mm=40.0, y_mm=100.0, width_mm=60.0, size_pt=12.0)
    left = compose(
        Composition([A4], [TextBox(align="left", **common)]), tmp_path / "l.pdf"
    )
    right = compose(
        Composition([A4], [TextBox(align="right", **common)]), tmp_path / "r.pdf"
    )
    assert ink_bbox_mm(right, 300.0)[0] > ink_bbox_mm(left, 300.0)[0] + 20.0


def test_rotation_turns_the_block(tmp_path):
    upright = compose(
        Composition([A4], [TextBox(text="Onionskin", x_mm=60, y_mm=150, size_pt=14)]),
        tmp_path / "u.pdf",
    )
    turned = compose(
        Composition(
            [A4],
            [TextBox(text="Onionskin", x_mm=60, y_mm=150, size_pt=14, rotation_deg=90)],
        ),
        tmp_path / "t.pdf",
    )
    ub = ink_bbox_mm(upright, 300.0)
    tb = ink_bbox_mm(turned, 300.0)
    assert (ub[2] - ub[0]) > (ub[3] - ub[1])  # wide when upright
    assert (tb[3] - tb[1]) > (tb[2] - tb[0])  # tall when turned


def test_rotation_is_clockwise_on_the_page(tmp_path):
    """+90 must take the text downward, matching the rest of Onionskin."""
    out = compose(
        Composition(
            [A4],
            [TextBox(text="Onionskin", x_mm=100.0, y_mm=100.0, rotation_deg=90.0)],
        ),
        tmp_path / "d.pdf",
    )
    x0, y0, x1, y1 = ink_bbox_mm(out, 300.0)
    assert y1 > 100.0  # the block runs down the page from its anchor
    assert x0 == pytest.approx(100.0, abs=4.0)


def test_colour_is_carried_through(tmp_path):
    import numpy as np

    from onionskin.render import Document

    out = compose(
        Composition([A4], [TextBox(text="RED", x_mm=50, y_mm=100, size_pt=40,
                                   colour="#ff0000")]),
        tmp_path / "d.pdf",
    )
    with Document(out) as doc:
        rgb = doc.render(0, 150).rgb
    ink = rgb.reshape(-1, 3)
    reddest = ink[np.argmin(ink[:, 1].astype(int) + ink[:, 2].astype(int))]
    assert reddest[0] > 200 and reddest[1] < 60


# --- validation ------------------------------------------------------------


def test_empty_text_is_refused():
    with pytest.raises(LayoutError, match="no text"):
        TextBox(text="   ").validate(1)


def test_page_out_of_range_is_refused():
    with pytest.raises(LayoutError, match="not in the document"):
        TextBox(page=3, text="x").validate(2)


def test_unknown_font_is_refused_by_name():
    with pytest.raises(LayoutError, match="unknown font"):
        TextBox(text="x", font="Comic Sans").validate(1)


@pytest.mark.parametrize(
    "kwargs, message",
    [
        ({"size_pt": 0}, "out of range"),
        ({"size_pt": 900}, "out of range"),
        ({"align": "middle"}, "align must be"),
        ({"width_mm": -5}, "width must be positive"),
        ({"line_spacing": 0}, "spacing must be positive"),
        ({"colour": "not-a-colour"}, "not a colour"),
    ],
)
def test_bad_settings_are_refused(kwargs, message):
    with pytest.raises(LayoutError, match=message):
        TextBox(text="x", **kwargs).validate(1)


def test_composing_nothing_is_refused():
    with pytest.raises(LayoutError, match="at least one"):
        Composition([A4], []).validate()


# --- specs and layout files ------------------------------------------------


def test_parse_text_spec_reads_page_position_and_words():
    box = parse_text_spec("2:60,150:Approved 25 July")
    assert box.page == 1  # 1-based in, 0-based inside
    assert (box.x_mm, box.y_mm) == (60.0, 150.0)
    assert box.text == "Approved 25 July"


def test_parse_text_spec_keeps_colons_in_the_words():
    assert parse_text_spec("1:20,30:Note: see clause 4").text == "Note: see clause 4"


@pytest.mark.parametrize(
    "bad", ["no colons", "1:20,30", "x:20,30:text", "1:20:text", "1:a,b:text", "0:1,1:t"]
)
def test_parse_text_spec_rejects_nonsense(bad):
    with pytest.raises(LayoutError):
        parse_text_spec(bad)


def test_layout_round_trips_through_disk(tmp_path):
    boxes = [
        TextBox(page=0, x_mm=20, y_mm=30, text="one", size_pt=9),
        TextBox(page=1, x_mm=40, y_mm=50, text="two", align="right", width_mm=60),
    ]
    path = save_layout(boxes, tmp_path / "layout.json")
    assert load_layout(path) == boxes


def test_layout_files_number_pages_from_one(tmp_path):
    path = tmp_path / "layout.json"
    path.write_text(json.dumps([{"page": 1, "x_mm": 10, "y_mm": 10, "text": "hi"}]))
    assert load_layout(path)[0].page == 0


def test_layout_rejects_page_zero(tmp_path):
    path = tmp_path / "layout.json"
    path.write_text(json.dumps([{"page": 0, "text": "hi"}]))
    with pytest.raises(LayoutError, match="numbered from 1"):
        load_layout(path)


def test_layout_names_an_unknown_setting(tmp_path):
    path = tmp_path / "layout.json"
    path.write_text(json.dumps([{"page": 1, "text": "hi", "bold": True}]))
    with pytest.raises(LayoutError, match="bold"):
        load_layout(path)


def test_layout_rejects_broken_json(tmp_path):
    path = tmp_path / "layout.json"
    path.write_text("{not json")
    with pytest.raises(LayoutError, match="not valid JSON"):
        load_layout(path)


def test_missing_layout_file(tmp_path):
    with pytest.raises(LayoutError, match="no layout file"):
        load_layout(tmp_path / "nope.json")


# --- through the pipeline --------------------------------------------------


def test_compose_run_writes_a_delta_over_a_real_document(tmp_path):
    source = make_pdf(tmp_path / "form.pdf", [(20.0, 40.0, "Authorised by:")])
    boxes = [TextBox(page=0, x_mm=60.0, y_mm=150.0, text="J. Bezzina", size_pt=12)]

    result = pipeline.compose_run(source, boxes, tmp_path / "delta.pdf", opts())

    assert result.output.is_file()
    assert not result.blocked
    assert result.pages_with_additions == [1]
    assert result.mode == pipeline.COMPOSE

    x0, y0, _, _ = ink_bbox_mm(result.output, 300.0)
    assert x0 == pytest.approx(60.0, abs=1.0)
    assert y0 == pytest.approx(150.0, abs=1.5)


def test_compose_never_reports_reflow(tmp_path):
    """The point of this path: no ink can move, so nothing can block on it."""
    source = make_pdf(tmp_path / "a.pdf", [(20.0, 100.0, "existing text here")])
    boxes = [TextBox(page=0, x_mm=20.0, y_mm=101.0, text="written right over it")]

    result = pipeline.compose_run(source, boxes, tmp_path / "delta.pdf", opts())

    assert all(page.removed_ink_mm2 == 0 for page in result.pages)
    assert not any(c.code == "reflow" for c in result.checks)
    assert not result.blocked


def test_compose_still_warns_about_the_dead_border(tmp_path):
    source = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "base")])
    boxes = [TextBox(page=0, x_mm=1.0, y_mm=150.0, text="too close to the edge")]

    result = pipeline.compose_run(source, boxes, tmp_path / "delta.pdf", opts())

    assert any(c.code == "margin" for c in result.checks)


def test_compose_applies_calibration(tmp_path, onionskin_home):
    calibrate.save_profile(
        calibrate.Profile(name="office", error=Similarity(dx_mm=1.5, dy_mm=2.0))
    )
    source = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "base")])
    boxes = [TextBox(page=0, x_mm=60.0, y_mm=150.0, text="Approved")]

    plain = pipeline.compose_run(source, boxes, tmp_path / "plain.pdf", opts())
    fixed = pipeline.compose_run(
        source, boxes, tmp_path / "fixed.pdf", opts(profile="office")
    )

    before = ink_bbox_mm(plain.output, 300.0)
    after = ink_bbox_mm(fixed.output, 300.0)
    assert after[0] == pytest.approx(before[0] - 1.5, abs=0.3)
    assert after[1] == pytest.approx(before[1] - 2.0, abs=0.3)


def test_compose_writes_previews(tmp_path):
    source = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "base")], pages=2)
    boxes = [TextBox(page=1, x_mm=60.0, y_mm=150.0, text="Approved")]

    result = pipeline.compose_run(
        source, boxes, tmp_path / "delta.pdf", opts(preview_dir=tmp_path / "proof")
    )

    assert len(result.previews) == 2
    assert all(p.is_file() for p in result.previews)


def test_compose_rejects_a_page_that_does_not_exist(tmp_path):
    source = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "base")])
    with pytest.raises(LayoutError, match="not in the document"):
        pipeline.compose_run(
            source, [TextBox(page=4, text="x")], tmp_path / "d.pdf", opts()
        )


def test_compose_result_serialises(tmp_path):
    source = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "base")])
    boxes = [TextBox(page=0, x_mm=60.0, y_mm=150.0, text="Approved")]

    result = pipeline.compose_run(source, boxes, tmp_path / "d.pdf", opts())
    payload = json.loads(json.dumps(result.to_dict()))

    assert payload["mode"] == "compose"
    assert payload["total_regions"] >= 1


@requires_soffice
def test_compose_onto_a_word_document(tmp_path):
    source = make_docx(
        tmp_path / "form.docx", ["PURCHASE ORDER 4471", "", "Authorised by:"]
    )
    boxes = [
        TextBox(page=0, x_mm=45.0, y_mm=60.0, text="J. Bezzina — 25 July 2026",
                size_pt=11)
    ]

    result = pipeline.compose_run(source, boxes, tmp_path / "delta.pdf", opts())

    assert not result.blocked
    assert result.total_regions >= 1
    assert ink_bbox_mm(result.output, 300.0) is not None


# --- text the font cannot actually write ------------------------------------


@pytest.mark.parametrize(
    "text, label",
    [
        ("季度报告", "Chinese"),
        ("承認済み", "Japanese"),
        ("تمت الموافقة", "Arabic"),
        ("Утверждено", "Cyrillic"),
        ("Έγκριση", "Greek"),
        ("אושר", "Hebrew"),
        ("Approved ✅", "emoji"),
    ],
)
def test_text_the_builtin_fonts_cannot_write_is_refused(text, label):
    """reportlab silently substitutes a black box for every character it cannot
    encode. Printed onto someone's only copy, that is the worst outcome there
    is — so it has to be an error, not a surprise."""
    with pytest.raises(LayoutError) as exc:
        TextBox(text=text).validate(1)
    assert "cannot write these characters" in str(exc.value)
    assert "--font-file" in str(exc.value)


def test_western_european_text_still_works():
    TextBox(text="café — naïve Ärger £50 «déjà»").validate(1)


def test_decomposed_accents_are_accepted(tmp_path):
    """macOS hands out decomposed Unicode, so "café" can arrive as e + combining
    acute. Rejecting that would break ordinary French on one platform only."""
    decomposed = "café"
    assert decomposed != "café"

    box = TextBox(text=decomposed)
    box.validate(1)

    assert box.lines() == ["café"]
    out = compose(Composition([A4], [TextBox(text=decomposed, x_mm=30, y_mm=100)]),
                  tmp_path / "d.pdf")
    assert ink_bbox_mm(out, 300.0) is not None


def test_a_registered_font_lifts_the_restriction(tmp_path):
    from onionskin.compose import register_font

    font_path = Path("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf")
    if not font_path.is_file():
        pytest.skip("DejaVuSans is not installed")

    name = register_font(font_path)
    box = TextBox(text="Утверждено", font=name, x_mm=30.0, y_mm=100.0, size_pt=14)
    box.validate(1)

    out = compose(Composition([A4], [box]), tmp_path / "ru.pdf")
    assert ink_bbox_mm(out, 300.0) is not None


def test_symbol_font_is_not_held_to_latin_coverage():
    TextBox(text="abg", font="Symbol").validate(1)


# --- numbers that are not numbers -------------------------------------------


@pytest.mark.parametrize(
    "kwargs, message",
    [
        ({"x_mm": float("inf")}, "finite"),
        ({"y_mm": float("-inf")}, "finite"),
        ({"x_mm": float("nan")}, "not a number"),
        ({"size_pt": float("nan")}, "not a number"),
        ({"rotation_deg": float("nan")}, "not a number"),
        ({"rotation_deg": float("inf")}, "finite"),
        ({"line_spacing": float("inf")}, "finite"),
        ({"width_mm": float("nan")}, "not a number"),
    ],
)
def test_non_finite_numbers_are_refused_cleanly(kwargs, message):
    """NaN slips past every range check — comparisons against it are all false —
    and only surfaces later as a traceback from inside the PDF writer."""
    with pytest.raises(LayoutError, match=message):
        TextBox(text="hi", **kwargs).validate(1)


@pytest.mark.parametrize(
    "kwargs, message",
    [
        ({"text": 123}, "text must be text"),
        ({"text": None}, "text must be text"),
        ({"text": {"a": 1}}, "text must be text"),
        ({"size_pt": "big"}, "must be a number"),
        ({"width_mm": []}, "must be a number"),
        ({"x_mm": "abc"}, "must be a number"),
        ({"line_spacing": None}, "must be a number"),
        ({"colour": 12345}, "colour must be text"),
        ({"font": 7}, "unknown font"),
    ],
)
def test_wrong_types_give_a_clean_error_not_a_traceback(kwargs, message):
    base = {"text": "hi"}
    base.update(kwargs)
    with pytest.raises(LayoutError, match=message):
        TextBox(**base).validate(1)


@pytest.mark.parametrize("page", [1e400, None, [1], {"a": 1}, "abc"])
def test_from_dict_rejects_impossible_page_numbers(page):
    """1e400 parses from JSON straight to inf, and OverflowError is neither
    TypeError nor ValueError, so it used to escape the handler entirely."""
    with pytest.raises(LayoutError, match="page number"):
        TextBox.from_dict({"page": page, "text": "hi"})


def test_bad_layout_reaches_the_user_as_an_error_not_a_crash(tmp_path):
    from onionskin import pipeline

    source = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "base")])
    for bad in ({"page": 1, "text": 123}, {"page": 1, "text": "x", "size_pt": "big"}):
        path = tmp_path / "layout.json"
        path.write_text(json.dumps([bad]))
        with pytest.raises(LayoutError):
            pipeline.compose_run(
                source, load_layout(path), tmp_path / "d.pdf",
                pipeline.Options(dpi=150),
            )
