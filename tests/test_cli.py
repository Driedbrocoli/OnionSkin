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
