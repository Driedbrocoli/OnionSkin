import json

import pytest

pytest.importorskip("fastapi")
from fastapi.testclient import TestClient  # noqa: E402

from onionskin import calibrate  # noqa: E402
from onionskin.geometry import Similarity  # noqa: E402
from onionskin.web.app import create_app  # noqa: E402

from helpers import make_pdf  # noqa: E402


@pytest.fixture
def client():
    return TestClient(create_app())


@pytest.fixture
def docs(tmp_path):
    original = make_pdf(tmp_path / "a.pdf", [(20.0, 40.0, "Invoice 4471")])
    edited = make_pdf(
        tmp_path / "b.pdf",
        [(20.0, 40.0, "Invoice 4471"), (60.0, 150.0, "PAID 25 July")],
    )
    return original, edited


def post_delta(client, docs, **extra):
    original, edited = docs
    data = {"dpi": "200", "mode": "raster", "margin": "5", "profile": ""}
    data.update({k: str(v) for k, v in extra.items()})
    with original.open("rb") as f1, edited.open("rb") as f2:
        return client.post(
            "/api/delta",
            files={
                "original": ("a.pdf", f1, "application/pdf"),
                "edited": ("b.pdf", f2, "application/pdf"),
            },
            data=data,
        )


def test_index_serves_the_app(client):
    res = client.get("/")
    assert res.status_code == 200
    assert "Onion" in res.text


def test_status_reports_capabilities(client):
    body = client.get("/api/status").json()
    assert ".pdf" in body["supported"]
    assert isinstance(body["libreoffice"], bool)


def test_delta_returns_findings_and_links(client, docs):
    res = post_delta(client, docs)
    assert res.status_code == 200
    body = res.json()

    assert body["total_regions"] == 1
    assert body["blocked"] is False
    assert body["download"].endswith("delta.pdf")
    assert len(body["previews"]) == 1
    assert body["pages"][0]["added_regions"][0]["x_mm"] == pytest.approx(60.0, abs=2)


def test_delta_pdf_is_downloadable(client, docs):
    body = post_delta(client, docs).json()
    res = client.get(body["download"], params={"name": "invoice-4471"})
    assert res.status_code == 200
    assert res.headers["content-type"] == "application/pdf"
    assert "invoice-4471.pdf" in res.headers["content-disposition"]
    assert res.content.startswith(b"%PDF")


def test_download_filename_is_sanitised(client, docs):
    body = post_delta(client, docs).json()
    res = client.get(body["download"], params={"name": "../../etc/passwd"})
    assert res.status_code == 200
    disposition = res.headers["content-disposition"]
    assert "/" not in disposition.split("filename=")[-1]


def test_previews_are_served(client, docs):
    body = post_delta(client, docs).json()
    res = client.get(body["previews"][0])
    assert res.status_code == 200
    assert res.headers["content-type"] == "image/png"
    assert res.content[:8] == b"\x89PNG\r\n\x1a\n"


def test_reflow_is_reported_as_blocked(client, tmp_path):
    original = make_pdf(tmp_path / "a.pdf", [(20.0, 100.0, "line one")])
    edited = make_pdf(tmp_path / "b.pdf", [(20.0, 108.0, "line one")])

    body = post_delta(client, (original, edited)).json()

    assert body["blocked"] is True
    assert any(c["code"] == "reflow" for c in body["checks"])


def test_unsupported_upload_is_rejected(client, tmp_path, docs):
    original, _ = docs
    bad = tmp_path / "notes.xyz"
    bad.write_text("nope")
    with original.open("rb") as f1, bad.open("rb") as f2:
        res = client.post(
            "/api/delta",
            files={"original": ("a.pdf", f1), "edited": ("notes.xyz", f2)},
            data={"dpi": "200"},
        )
    assert res.status_code == 400
    assert "not a supported file type" in res.json()["detail"]


def test_empty_upload_is_rejected(client, docs, tmp_path):
    original, _ = docs
    empty = tmp_path / "empty.pdf"
    empty.write_bytes(b"")
    with original.open("rb") as f1, empty.open("rb") as f2:
        res = client.post(
            "/api/delta",
            files={"original": ("a.pdf", f1), "edited": ("empty.pdf", f2)},
            data={"dpi": "200"},
        )
    assert res.status_code == 400


def test_bad_options_are_rejected(client, docs):
    assert post_delta(client, docs, mode="sideways").status_code == 400
    assert post_delta(client, docs, dpi=5).status_code == 400


@pytest.mark.parametrize(
    "job_id", ["../../etc", "..%2f..", "nonhex-job-id", "0" * 31, "0" * 33]
)
def test_job_ids_that_are_path_shaped_are_refused(client, job_id):
    assert client.get(f"/api/jobs/{job_id}/delta.pdf").status_code == 404


def test_unknown_job_is_a_clean_404(client):
    assert client.get(f"/api/jobs/{'a' * 32}/delta.pdf").status_code == 404
    assert client.get(f"/api/jobs/{'a' * 32}/preview/1").status_code == 404


def test_preview_page_out_of_range(client, docs):
    body = post_delta(client, docs).json()
    base = body["previews"][0].rsplit("/", 1)[0]
    assert client.get(f"{base}/99").status_code == 404
    assert client.get(f"{base}/0").status_code == 404


# --- calibration -----------------------------------------------------------


def test_target_downloads_as_a_pdf(client):
    res = client.post("/api/calibrate/target", data={"page": "a4"})
    assert res.status_code == 200
    assert res.content.startswith(b"%PDF")


def test_target_rejects_unknown_page_size(client):
    assert client.post("/api/calibrate/target", data={"page": "a9"}).status_code == 400


def test_solving_stores_a_usable_profile(client, onionskin_home):
    res = client.post(
        "/api/calibrate/solve",
        data={
            "name": "office",
            "page": "a4",
            "points": "P1:0.4,-0.2;P2:0.4,-0.2;P3:0.4,-0.2;P4:0.4,-0.2",
        },
    )
    assert res.status_code == 200
    assert res.json()["points"] == 4

    profile = calibrate.load_profile("office")
    assert profile.error.dx_mm == pytest.approx(0.4, abs=1e-6)


def test_solving_rejects_unparseable_readings(client, onionskin_home):
    res = client.post(
        "/api/calibrate/solve", data={"name": "x", "page": "a4", "points": "junk"}
    )
    assert res.status_code == 400


def test_manual_profile_entry(client, onionskin_home):
    res = client.post(
        "/api/calibrate/manual",
        data={"name": "byhand", "dx": "0.5", "dy": "0", "rotation": "0.1", "scale": "1"},
    )
    assert res.status_code == 200
    assert calibrate.load_profile("byhand").error.dx_mm == pytest.approx(0.5)


def test_profiles_are_listed_and_deletable(client, onionskin_home):
    calibrate.save_profile(
        calibrate.Profile(name="office", error=Similarity(dx_mm=0.3))
    )
    listed = client.get("/api/profiles").json()["profiles"]
    assert [p["name"] for p in listed] == ["office"]
    assert "+0.30" in listed[0]["error"]

    assert client.delete("/api/profiles/office").status_code == 200
    assert client.delete("/api/profiles/office").status_code == 404


def test_a_profile_moves_the_delta(client, docs, onionskin_home):
    calibrate.save_profile(
        calibrate.Profile(name="shifty", error=Similarity(dx_mm=1.5))
    )
    plain = post_delta(client, docs).json()
    fixed = post_delta(client, docs, profile="shifty").json()

    assert fixed["correction"] is not None
    assert plain["pages"][0]["added_regions"][0]["x_mm"] == pytest.approx(
        fixed["pages"][0]["added_regions"][0]["x_mm"], abs=0.1
    )  # regions describe the document, not the corrected plate

    left = plain["pages"][0]["added_regions"][0]["x_mm"]
    assert left > 0


# --- typing directly on the page -------------------------------------------


@pytest.fixture
def opened(client, tmp_path):
    """A document opened for editing, ready to receive text boxes."""
    source = make_pdf(tmp_path / "form.pdf", [(20.0, 40.0, "Authorised by:")], pages=2)
    with source.open("rb") as f:
        res = client.post(
            "/api/compose/open",
            files={"source": ("form.pdf", f, "application/pdf")},
            data={"dpi": "80"},
        )
    assert res.status_code == 200
    return res.json()


def box(**kwargs):
    base = {"page": 1, "x_mm": 60.0, "y_mm": 150.0, "text": "Approved", "size_pt": 12}
    base.update(kwargs)
    return base


def test_fonts_are_listed(client):
    fonts = client.get("/api/fonts").json()["fonts"]
    assert "Helvetica" in fonts and "Times-Roman" in fonts


def test_opening_returns_page_geometry_and_images(opened):
    assert len(opened["pages"]) == 2
    assert opened["pages"][0]["width_mm"] == pytest.approx(210.0, abs=0.5)
    assert "A4" in opened["pages"][0]["label"]
    assert len(opened["images"]) == 2


def test_page_images_are_served(client, opened):
    res = client.get(opened["images"][0])
    assert res.status_code == 200
    assert res.headers["content-type"] == "image/png"


def test_opening_rejects_an_unsupported_file(client, tmp_path):
    bad = tmp_path / "notes.xyz"
    bad.write_text("nope")
    with bad.open("rb") as f:
        res = client.post("/api/compose/open", files={"source": ("notes.xyz", f)})
    assert res.status_code == 400


def test_composing_places_text_and_returns_a_delta(client, opened):
    res = client.post(
        "/api/compose/render",
        data={"job": opened["job"], "boxes": json.dumps([box()]), "dpi": "200"},
    )
    assert res.status_code == 200
    body = res.json()

    assert body["mode"] == "compose"
    assert body["blocked"] is False
    assert body["pages_with_additions"] == [1]
    region = body["pages"][0]["added_regions"][0]
    assert region["x_mm"] == pytest.approx(60.0, abs=1.5)

    pdf = client.get(body["download"])
    assert pdf.status_code == 200
    assert pdf.content.startswith(b"%PDF")


def test_composing_onto_the_second_page(client, opened):
    res = client.post(
        "/api/compose/render",
        data={
            "job": opened["job"],
            "boxes": json.dumps([box(page=2)]),
            "dpi": "200",
        },
    )
    assert res.json()["pages_with_additions"] == [2]


def test_composing_never_blocks_on_reflow(client, opened):
    """Absolutely positioned text cannot displace anything."""
    res = client.post(
        "/api/compose/render",
        data={
            "job": opened["job"],
            "boxes": json.dumps([box(x_mm=20.0, y_mm=40.0, text="right over it")]),
            "dpi": "200",
        },
    )
    body = res.json()
    assert body["blocked"] is False
    assert not any(c["code"] == "reflow" for c in body["checks"])


def test_composing_still_warns_about_the_border(client, opened):
    res = client.post(
        "/api/compose/render",
        data={
            "job": opened["job"],
            "boxes": json.dumps([box(x_mm=1.0, y_mm=150.0)]),
            "dpi": "200",
        },
    )
    assert any(c["code"] == "margin" for c in res.json()["checks"])


def test_composing_reports_a_page_that_does_not_exist(client, opened):
    res = client.post(
        "/api/compose/render",
        data={"job": opened["job"], "boxes": json.dumps([box(page=9)]), "dpi": "200"},
    )
    assert res.status_code == 400
    assert "not in the document" in res.json()["detail"]


@pytest.mark.parametrize(
    "boxes, message",
    [
        ("not json", "not valid JSON"),
        ('{"page": 1}', "must be a list"),
        ('[{"page": 1, "text": ""}]', "no text"),
        ('[{"page": 1, "text": "x", "font": "Comic Sans"}]', "unknown font"),
        ('[{"page": 1, "text": "x", "bold": true}]', "bold"),
    ],
)
def test_bad_box_payloads_are_reported(client, opened, boxes, message):
    res = client.post(
        "/api/compose/render",
        data={"job": opened["job"], "boxes": boxes, "dpi": "200"},
    )
    assert res.status_code == 400
    assert message in res.json()["detail"]


def test_composing_against_an_unknown_job(client):
    res = client.post(
        "/api/compose/render",
        data={"job": "a" * 32, "boxes": json.dumps([box()])},
    )
    assert res.status_code == 404


@pytest.mark.parametrize("job_id", ["../../etc", "not-hex", "0" * 31])
def test_compose_job_ids_are_validated(client, job_id):
    res = client.post(
        "/api/compose/render", data={"job": job_id, "boxes": json.dumps([box()])}
    )
    assert res.status_code == 404
    assert client.get(f"/api/jobs/{job_id}/page/1").status_code == 404


def test_composing_applies_a_calibration_profile(client, opened, onionskin_home):
    calibrate.save_profile(
        calibrate.Profile(name="shifty", error=Similarity(dx_mm=1.5))
    )
    res = client.post(
        "/api/compose/render",
        data={
            "job": opened["job"],
            "boxes": json.dumps([box()]),
            "dpi": "200",
            "profile": "shifty",
        },
    )
    assert res.status_code == 200
    assert res.json()["profile"] == "shifty"
    assert "-1.50" in res.json()["correction"]


# --- the served page itself -------------------------------------------------


def read_index() -> str:
    from onionskin.web.app import STATIC

    return (STATIC / "index.html").read_text(encoding="utf-8")


def test_head_has_no_stray_markup():
    """A data-URI favicon with double quotes once broke out of its attribute
    and rendered as text at the top of the page."""
    from html.parser import HTMLParser

    class Collector(HTMLParser):
        def __init__(self):
            super().__init__()
            self.stray = []

        def handle_data(self, data):
            if data.strip() and self.lasttag not in ("style", "title"):
                self.stray.append(data.strip()[:40])

    collector = Collector()
    collector.feed(read_index().split("</head>")[0])
    assert collector.stray == []


@pytest.mark.parametrize(
    "element_id",
    [
        "panel-compare", "panel-type",          # the two workflows
        "file-original", "file-edited", "file-source",
        "stage", "stage-img", "stage-boxes",    # the editor canvas
        "box-text", "box-size", "box-font", "box-x", "box-y", "box-delete",
        "type-go", "go", "download", "results", "checks", "summary",
        "profile", "type-profile",
    ],
)
def test_elements_the_script_drives_exist(element_id):
    assert f'id="{element_id}"' in read_index()


def test_the_page_is_self_contained():
    """No external hosts: this often runs on a machine with no internet."""
    import re

    html = read_index()
    remote = re.findall(r'(?:src|href)="(https?://[^"]+)"', html)
    assert remote == []
