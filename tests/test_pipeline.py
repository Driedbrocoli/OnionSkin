import pytest

from onionskin import calibrate, pipeline, safety
from onionskin.geometry import Similarity

from helpers import A4, ink_bbox_mm, make_docx, make_pdf, requires_soffice

DPI = 250.0  # fast, still well above the feature size we care about


def opts(**kwargs):
    kwargs.setdefault("dpi", DPI)
    return pipeline.Options(**kwargs)


def codes(result):
    return {c.code for c in result.checks}


def test_delta_carries_the_addition_and_nothing_else(tmp_path):
    original = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "Invoice 4471")])
    edited = make_pdf(
        tmp_path / "b.pdf",
        [(20.0, 40.0, "Invoice 4471"), (60.0, 150.0, "PAID 25 July")],
    )

    result = pipeline.run(original, edited, tmp_path / "delta.pdf", opts())

    assert result.output.is_file()
    assert result.total_regions == 1
    assert result.pages_with_additions == [1]
    assert not result.blocked

    x0, y0, _, y1 = ink_bbox_mm(result.output, 300.0)
    assert x0 == pytest.approx(60.0, abs=1.5)
    assert y1 == pytest.approx(150.0, abs=2.0)
    assert y0 > 100.0  # the original line at y=40 is absent


def test_only_the_edited_page_gets_ink(tmp_path):
    original = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "page text")], pages=3)
    edited_items = [(20.0, 40.0, "page text")]
    # Rebuild page 2 with an extra mark by writing the pages individually.
    from reportlab.pdfgen import canvas

    from onionskin.geometry import mm_to_pt

    path = tmp_path / "b.pdf"
    pdf = canvas.Canvas(str(path), pagesize=(A4.width_pt, A4.height_pt))
    for page_index in range(3):
        pdf.setFont("Helvetica", 12)
        for x_mm, y_mm, text in edited_items:
            pdf.drawString(mm_to_pt(x_mm), A4.height_pt - mm_to_pt(y_mm), text)
        if page_index == 1:
            pdf.drawString(mm_to_pt(70.0), A4.height_pt - mm_to_pt(200.0), "initialled")
        pdf.showPage()
    pdf.save()

    result = pipeline.run(original, path, tmp_path / "delta.pdf", opts())

    assert result.pages_with_additions == [2]
    assert len(result.pages) == 3
    assert ink_bbox_mm(result.output, 200.0, page=0) is None
    assert ink_bbox_mm(result.output, 200.0, page=1) is not None
    assert ink_bbox_mm(result.output, 200.0, page=2) is None


def test_vector_mode_produces_the_same_placement(tmp_path):
    original = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "base")])
    edited = make_pdf(tmp_path / "b.pdf", [(20.0, 40.0, "base"), (60.0, 150.0, "added")])

    raster = pipeline.run(original, edited, tmp_path / "r.pdf", opts())
    vector = pipeline.run(
        original, edited, tmp_path / "v.pdf", opts(mode=pipeline.VECTOR, pad_mm=0.0)
    )

    for got, want in zip(
        ink_bbox_mm(vector.output, 300.0), ink_bbox_mm(raster.output, 300.0)
    ):
        assert got == pytest.approx(want, abs=0.6)


def test_reflow_blocks_the_run(tmp_path):
    """Text pushed down the page cannot be overlaid, and must say so."""
    original = make_pdf(
        tmp_path / "a.pdf", [(20.0, 100.0, "First line"), (20.0, 110.0, "Second line")]
    )
    edited = make_pdf(
        tmp_path / "b.pdf", [(20.0, 100.0, "First line"), (20.0, 118.0, "Second line")]
    )

    result = pipeline.run(original, edited, tmp_path / "delta.pdf", opts())

    assert result.blocked
    assert "reflow" in codes(result)


def test_identical_documents_are_reported_as_empty(tmp_path):
    items = [(20.0, 40.0, "unchanged")]
    original = make_pdf(tmp_path / "a.pdf", items)
    edited = make_pdf(tmp_path / "b.pdf", items)

    result = pipeline.run(original, edited, tmp_path / "delta.pdf", opts())

    assert result.blocked
    assert "empty_delta" in codes(result)
    assert result.total_regions == 0


def test_an_extra_page_warns_and_is_printed_whole(tmp_path):
    original = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "one")], pages=1)
    edited = make_pdf(tmp_path / "b.pdf", [(20.0, 40.0, "one")], pages=2)

    result = pipeline.run(original, edited, tmp_path / "delta.pdf", opts())

    assert "pages_added" in codes(result)
    assert result.pages_with_additions == [2]
    assert ink_bbox_mm(result.output, 200.0, page=1) is not None


def test_addition_in_the_dead_border_warns(tmp_path):
    original = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "base")])
    edited = make_pdf(tmp_path / "b.pdf", [(20.0, 40.0, "base"), (2.0, 290.0, "x")])

    result = pipeline.run(original, edited, tmp_path / "delta.pdf", opts())

    assert "margin" in codes(result)


def test_previews_are_written_one_per_page(tmp_path):
    original = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "base")], pages=2)
    edited = make_pdf(tmp_path / "b.pdf", [(20.0, 40.0, "base"), (60.0, 150.0, "new")], pages=2)

    result = pipeline.run(
        original, edited, tmp_path / "delta.pdf",
        opts(preview_dir=tmp_path / "proof"),
    )

    assert len(result.previews) == 2
    assert all(p.is_file() and p.stat().st_size > 0 for p in result.previews)
    assert result.previews[0].name == "page-001.png"


def test_a_calibration_profile_moves_the_ink(tmp_path, onionskin_home):
    calibrate.save_profile(
        calibrate.Profile(name="office", error=Similarity(dx_mm=1.5, dy_mm=2.0))
    )
    original = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "base")])
    edited = make_pdf(tmp_path / "b.pdf", [(20.0, 40.0, "base"), (60.0, 150.0, "new")])

    plain = pipeline.run(original, edited, tmp_path / "plain.pdf", opts())
    fixed = pipeline.run(
        original, edited, tmp_path / "fixed.pdf", opts(profile="office")
    )

    before = ink_bbox_mm(plain.output, 300.0)
    after = ink_bbox_mm(fixed.output, 300.0)
    assert after[0] == pytest.approx(before[0] - 1.5, abs=0.3)
    assert after[1] == pytest.approx(before[1] - 2.0, abs=0.3)
    assert fixed.profile is not None
    assert "calibrated" in codes(fixed)


def test_masks_are_released_after_the_run(tmp_path):
    """A long document must not hold every page's pixels."""
    original = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "base")], pages=4)
    edited = make_pdf(tmp_path / "b.pdf", [(20.0, 40.0, "base"), (60.0, 150.0, "new")], pages=4)

    result = pipeline.run(original, edited, tmp_path / "delta.pdf", opts())

    assert all(page.added.size == 0 for page in result.pages)
    assert result.total_added_mm2 > 0


def test_result_serialises_to_json(tmp_path):
    import json

    original = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "base")])
    edited = make_pdf(tmp_path / "b.pdf", [(20.0, 40.0, "base"), (60.0, 150.0, "new")])

    result = pipeline.run(original, edited, tmp_path / "delta.pdf", opts())
    payload = json.loads(json.dumps(result.to_dict()))

    assert payload["total_regions"] == 1
    assert payload["pages"][0]["added_regions"][0]["x_mm"] == pytest.approx(60.0, abs=1.5)
    assert payload["blocked"] is False


@pytest.mark.parametrize(
    "kwargs, message",
    [
        ({"mode": "sideways"}, "mode must be"),
        ({"dpi": 10.0}, "dpi must be"),
        ({"ink_threshold": 0}, "ink-threshold"),
    ],
)
def test_bad_options_are_rejected_early(kwargs, message):
    with pytest.raises(ValueError, match=message):
        opts(**kwargs).validate()


def test_unsupported_input_is_reported_clearly(tmp_path):
    bad = tmp_path / "notes.xyz"
    bad.write_text("hello")
    good = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "x")])

    with pytest.raises(ValueError, match="unsupported file type"):
        pipeline.run(bad, good, tmp_path / "delta.pdf", opts())


def test_missing_input_is_reported_clearly(tmp_path):
    good = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "x")])
    with pytest.raises(ValueError, match="no such file"):
        pipeline.run(tmp_path / "nope.pdf", good, tmp_path / "delta.pdf", opts())


# --- the real Word path ----------------------------------------------------


@requires_soffice
def test_word_documents_end_to_end(tmp_path):
    """The actual use case: a .docx with a line added at the end."""
    base = ["Quarterly report", "Prepared by the finance team."]
    original = make_docx(tmp_path / "report.docx", base)
    edited = make_docx(tmp_path / "report-v2.docx", base + ["Approved 25 July."])

    result = pipeline.run(original, edited, tmp_path / "delta.pdf", opts())

    assert not result.blocked
    assert result.total_regions == 1
    assert result.pages[0].removed_ink_mm2 == pytest.approx(0.0, abs=0.5)

    region = result.pages[0].added_regions[0]
    assert region.width_mm > 15.0
    assert ink_bbox_mm(result.output, 300.0) is not None


@requires_soffice
def test_word_edit_that_reflows_is_caught(tmp_path):
    """Inserting a line in the middle pushes everything down — must block."""
    original = make_docx(
        tmp_path / "a.docx", ["First paragraph.", "Second paragraph.", "Third paragraph."]
    )
    edited = make_docx(
        tmp_path / "b.docx",
        ["First paragraph.", "Inserted line.", "Second paragraph.", "Third paragraph."],
    )

    result = pipeline.run(original, edited, tmp_path / "delta.pdf", opts())

    assert result.blocked
    assert "reflow" in codes(result)


@requires_soffice
def test_unchanged_word_document_yields_nothing(tmp_path):
    paragraphs = ["Identical", "Content"]
    original = make_docx(tmp_path / "a.docx", paragraphs)
    edited = make_docx(tmp_path / "b.docx", paragraphs)

    result = pipeline.run(original, edited, tmp_path / "delta.pdf", opts())

    assert result.total_regions == 0
    assert "empty_delta" in codes(result)


@requires_soffice
def test_mixed_docx_and_pdf_inputs(tmp_path):
    """Comparing a .docx against an exported PDF of it should still work."""
    from onionskin.render import Workspace, to_pdf

    base = ["Statement of account", "Balance carried forward."]
    original_docx = make_docx(tmp_path / "a.docx", base)
    with Workspace() as work:
        exported = to_pdf(original_docx, work)
        original_pdf = tmp_path / "a.pdf"
        original_pdf.write_bytes(exported.read_bytes())

    edited = make_docx(tmp_path / "b.docx", base + ["Settled in full."])
    result = pipeline.run(original_pdf, edited, tmp_path / "delta.pdf", opts())

    assert result.total_regions == 1
    assert not result.blocked
