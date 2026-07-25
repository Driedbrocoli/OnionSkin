import json

import pytest

from onionskin import calibrate
from onionskin.cli import main
from onionskin.geometry import Similarity

from helpers import ink_bbox_mm, make_pdf

FAST = ["--dpi", "200"]


@pytest.fixture
def docs(tmp_path):
    original = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "Invoice 4471")])
    edited = make_pdf(
        tmp_path / "b.pdf",
        [(20.0, 40.0, "Invoice 4471"), (60.0, 150.0, "PAID 25 July")],
    )
    return original, edited


def test_delta_writes_the_file_and_reports(tmp_path, docs, capsys):
    original, edited = docs
    out = tmp_path / "delta.pdf"

    code = main(["delta", str(original), str(edited), "-o", str(out)] + FAST)

    assert code == 0
    assert out.is_file()
    text = capsys.readouterr().out
    assert "1 addition(s)" in text
    assert "Actual size" in text  # the printing instructions must be there


def test_delta_json_output_is_machine_readable(tmp_path, docs, capsys):
    original, edited = docs
    out = tmp_path / "delta.pdf"

    main(["delta", str(original), str(edited), "-o", str(out), "--json"] + FAST)

    payload = json.loads(capsys.readouterr().out)
    assert payload["total_regions"] == 1
    assert payload["blocked"] is False
    assert payload["pages"][0]["added_regions"][0]["x_mm"] == pytest.approx(60.0, abs=2)


def test_delta_exits_nonzero_when_blocked(tmp_path, capsys):
    """A reflowed edit must not quietly produce a printable-looking file."""
    original = make_pdf(tmp_path / "a.pdf", [(20.0, 100.0, "line one")])
    edited = make_pdf(tmp_path / "b.pdf", [(20.0, 108.0, "line one")])

    code = main(
        ["delta", str(original), str(edited), "-o", str(tmp_path / "d.pdf")] + FAST
    )

    assert code == 2
    err = capsys.readouterr().err
    assert "BLOCKER" in err
    assert "Refusing" in err


def test_force_overrides_a_blocker(tmp_path, capsys):
    original = make_pdf(tmp_path / "a.pdf", [(20.0, 100.0, "line one")])
    edited = make_pdf(tmp_path / "b.pdf", [(20.0, 108.0, "line one")])

    code = main(
        ["delta", str(original), str(edited), "-o", str(tmp_path / "d.pdf"), "--force"]
        + FAST
    )
    assert code == 0


def test_delta_writes_previews_when_asked(tmp_path, docs):
    original, edited = docs
    proof = tmp_path / "proof"

    main(
        ["delta", str(original), str(edited), "-o", str(tmp_path / "d.pdf"),
         "--preview", str(proof)] + FAST
    )

    assert list(proof.glob("*.png"))


def test_vector_mode_selectable(tmp_path, docs):
    original, edited = docs
    out = tmp_path / "v.pdf"
    assert main(
        ["delta", str(original), str(edited), "-o", str(out), "--mode", "vector"] + FAST
    ) == 0
    assert ink_bbox_mm(out, 200.0) is not None


def test_inspect_reports_without_writing(tmp_path, docs, capsys):
    original, edited = docs
    code = main(["inspect", str(original), str(edited)] + FAST)

    assert code == 0
    text = capsys.readouterr().out
    assert "1 addition(s)" in text
    assert not list(tmp_path.glob("*delta*"))


def test_missing_file_is_a_clean_error(tmp_path, docs, capsys):
    _, edited = docs
    code = main(
        ["delta", str(tmp_path / "nope.pdf"), str(edited), "-o", str(tmp_path / "d.pdf")]
    )
    assert code == 1
    assert "no such file" in capsys.readouterr().err


def test_unsupported_type_is_a_clean_error(tmp_path, docs, capsys):
    _, edited = docs
    bad = tmp_path / "notes.xyz"
    bad.write_text("x")
    code = main(["delta", str(bad), str(edited), "-o", str(tmp_path / "d.pdf")])
    assert code == 1
    assert "unsupported file type" in capsys.readouterr().err


# --- calibration ------------------------------------------------------------


def test_calibrate_target_writes_a_page(tmp_path, capsys):
    out = tmp_path / "target.pdf"
    assert main(["calibrate", "target", "-o", str(out)]) == 0
    assert out.is_file()
    assert "100%" in capsys.readouterr().out


def test_calibrate_solve_stores_a_profile(onionskin_home, capsys):
    code = main(
        [
            "calibrate", "solve",
            "--point", "P1:+0.40,-0.20",
            "--point", "P2:+0.40,-0.20",
            "--point", "P3:+0.40,-0.20",
            "--point", "P4:+0.40,-0.20",
            "--name", "office",
        ]
    )
    assert code == 0
    assert "office" in capsys.readouterr().out

    profile = calibrate.load_profile("office")
    assert profile.error.dx_mm == pytest.approx(0.40, abs=1e-6)
    assert profile.correction.dx_mm == pytest.approx(-0.40, abs=1e-6)


def test_calibrate_solve_rejects_bad_measurements(onionskin_home, capsys):
    assert main(["calibrate", "solve", "--point", "nonsense"]) == 1
    assert "error:" in capsys.readouterr().err


def test_calibrate_solve_warns_when_the_fit_is_poor(onionskin_home, capsys):
    """Readings a similarity cannot explain mean the measurement is wrong."""
    main(
        [
            "calibrate", "solve",
            "--point", "P1:+2.0,0.0",
            "--point", "P2:-2.0,0.0",
            "--point", "P3:+2.0,0.0",
            "--point", "P4:-2.0,0.0",
            "--name", "bad",
        ]
    )
    assert "warning" in capsys.readouterr().err


def test_calibrate_set_accepts_manual_numbers(onionskin_home, capsys):
    code = main(
        ["calibrate", "set", "--name", "manual", "--dx", "0.5", "--rotation", "0.2"]
    )
    assert code == 0
    profile = calibrate.load_profile("manual")
    assert profile.error.dx_mm == pytest.approx(0.5)
    assert profile.error.rotation_deg == pytest.approx(0.2)


def test_calibrate_list_and_show_and_delete(onionskin_home, capsys):
    calibrate.save_profile(
        calibrate.Profile(name="office", error=Similarity(dx_mm=0.3))
    )

    assert main(["calibrate", "list"]) == 0
    assert "office" in capsys.readouterr().out

    assert main(["calibrate", "show", "office"]) == 0
    assert "printer error" in capsys.readouterr().out

    assert main(["calibrate", "show", "missing"]) == 1
    assert main(["calibrate", "delete", "office"]) == 0
    assert main(["calibrate", "delete", "office"]) == 1


def test_calibrate_list_is_helpful_when_empty(onionskin_home, capsys):
    assert main(["calibrate", "list"]) == 0
    assert "calibrate target" in capsys.readouterr().out


def test_delta_applies_a_named_profile(tmp_path, docs, onionskin_home):
    calibrate.save_profile(
        calibrate.Profile(name="office", error=Similarity(dx_mm=1.5))
    )
    original, edited = docs

    main(["delta", str(original), str(edited), "-o", str(tmp_path / "plain.pdf")] + FAST)
    main(
        ["delta", str(original), str(edited), "-o", str(tmp_path / "fixed.pdf"),
         "--profile", "office"] + FAST
    )

    plain = ink_bbox_mm(tmp_path / "plain.pdf", 300.0)
    fixed = ink_bbox_mm(tmp_path / "fixed.pdf", 300.0)
    assert fixed[0] == pytest.approx(plain[0] - 1.5, abs=0.3)


def test_unknown_profile_is_a_clean_error(tmp_path, docs, onionskin_home, capsys):
    original, edited = docs
    code = main(
        ["delta", str(original), str(edited), "-o", str(tmp_path / "d.pdf"),
         "--profile", "ghost"]
    )
    assert code == 1
    assert "ghost" in capsys.readouterr().err


# --- misc -------------------------------------------------------------------


def test_doctor_reports_the_environment(capsys, onionskin_home):
    main(["doctor"])
    text = capsys.readouterr().out
    assert "pypdfium2" in text and "pikepdf" in text
    assert "profiles" in text


def test_version_flag(capsys):
    with pytest.raises(SystemExit) as exc:
        main(["--version"])
    assert exc.value.code == 0
    assert "onionskin" in capsys.readouterr().out


def test_no_command_is_an_error(capsys):
    with pytest.raises(SystemExit) as exc:
        main([])
    assert exc.value.code == 2


# --- typing on the page -----------------------------------------------------


def test_add_places_text_without_a_second_document(tmp_path, capsys):
    source = make_pdf(tmp_path / "form.pdf", [(20.0, 40.0, "Authorised by:")])
    out = tmp_path / "delta.pdf"

    code = main(
        ["add", str(source), "-o", str(out), "--text", "1:60,150:Approved"] + FAST
    )

    assert code == 0
    x0, y0, _, _ = ink_bbox_mm(out, 300.0)
    assert x0 == pytest.approx(60.0, abs=1.0)
    assert y0 == pytest.approx(150.0, abs=1.5)
    assert "mode compose" in capsys.readouterr().out


def test_add_takes_several_boxes_across_pages(tmp_path):
    source = make_pdf(tmp_path / "form.pdf", [(20.0, 40.0, "base")], pages=2)
    out = tmp_path / "delta.pdf"

    main(
        ["add", str(source), "-o", str(out),
         "--text", "1:30,100:first page", "--text", "2:30,100:second page"] + FAST
    )

    assert ink_bbox_mm(out, 200.0, page=0) is not None
    assert ink_bbox_mm(out, 200.0, page=1) is not None


def test_add_honours_size_and_alignment(tmp_path):
    source = make_pdf(tmp_path / "form.pdf", [(20.0, 40.0, "base")])
    small, large = tmp_path / "s.pdf", tmp_path / "l.pdf"

    main(["add", str(source), "-o", str(small), "--text", "1:30,100:Onionskin",
          "--size", "8"] + FAST)
    main(["add", str(source), "-o", str(large), "--text", "1:30,100:Onionskin",
          "--size", "24"] + FAST)

    small_box, large_box = ink_bbox_mm(small, 300.0), ink_bbox_mm(large, 300.0)
    assert (large_box[2] - large_box[0]) > 2.5 * (small_box[2] - small_box[0])


def test_add_round_trips_a_layout_file(tmp_path):
    source = make_pdf(tmp_path / "form.pdf", [(20.0, 40.0, "base")])
    layout = tmp_path / "layout.json"

    main(["add", str(source), "-o", str(tmp_path / "a.pdf"),
          "--text", "1:60,150:Approved", "--save-layout", str(layout)] + FAST)
    assert layout.is_file()

    main(["add", str(source), "-o", str(tmp_path / "b.pdf"),
          "--layout", str(layout)] + FAST)

    for got, want in zip(
        ink_bbox_mm(tmp_path / "b.pdf", 300.0), ink_bbox_mm(tmp_path / "a.pdf", 300.0)
    ):
        assert got == pytest.approx(want, abs=0.1)


def test_add_with_nothing_to_place_is_an_error(tmp_path, capsys):
    source = make_pdf(tmp_path / "form.pdf", [(20.0, 40.0, "base")])
    code = main(["add", str(source), "-o", str(tmp_path / "d.pdf")])
    assert code == 1
    assert "nothing to place" in capsys.readouterr().err


def test_add_reports_a_bad_text_spec(tmp_path, capsys):
    source = make_pdf(tmp_path / "form.pdf", [(20.0, 40.0, "base")])
    code = main(["add", str(source), "-o", str(tmp_path / "d.pdf"),
                 "--text", "nonsense"])
    assert code == 1
    assert "PAGE:X,Y" in capsys.readouterr().err


def test_add_reports_a_page_that_does_not_exist(tmp_path, capsys):
    source = make_pdf(tmp_path / "form.pdf", [(20.0, 40.0, "base")])
    code = main(["add", str(source), "-o", str(tmp_path / "d.pdf"),
                 "--text", "5:30,30:too far"])
    assert code == 1
    assert "not in the document" in capsys.readouterr().err


def test_add_applies_a_calibration_profile(tmp_path, onionskin_home):
    calibrate.save_profile(
        calibrate.Profile(name="office", error=Similarity(dx_mm=1.5))
    )
    source = make_pdf(tmp_path / "form.pdf", [(20.0, 40.0, "base")])

    main(["add", str(source), "-o", str(tmp_path / "plain.pdf"),
          "--text", "1:60,150:Approved"] + FAST)
    main(["add", str(source), "-o", str(tmp_path / "fixed.pdf"),
          "--text", "1:60,150:Approved", "--profile", "office"] + FAST)

    plain = ink_bbox_mm(tmp_path / "plain.pdf", 300.0)
    fixed = ink_bbox_mm(tmp_path / "fixed.pdf", 300.0)
    assert fixed[0] == pytest.approx(plain[0] - 1.5, abs=0.3)


def test_fonts_are_listed(capsys):
    assert main(["fonts"]) == 0
    text = capsys.readouterr().out
    assert "Helvetica" in text and "Times-Roman" in text
