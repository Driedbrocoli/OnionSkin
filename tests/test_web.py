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
