"""FastAPI application.

Uploaded files are never trusted for anything but their bytes: each job gets a
UUID directory, and the only thing taken from the client's filename is a
suffix checked against a fixed allowlist.
"""

from __future__ import annotations

import json
import re
import shutil
import tempfile
import time
import uuid
from pathlib import Path

from fastapi import FastAPI, File, Form, HTTPException, UploadFile
from fastapi.responses import FileResponse, HTMLResponse, JSONResponse
from PIL import Image

from .. import __version__, calibrate, compose, pipeline
from ..geometry import Similarity
from ..render import (
    SUPPORTED,
    ConversionError,
    Document,
    DocumentError,
    Workspace,
    find_soffice,
    to_pdf,
)

MAX_UPLOAD_BYTES = 40 * 1024 * 1024
JOB_TTL_SECONDS = 60 * 60
JOB_ID_RE = re.compile(r"^[0-9a-f]{32}$")

STATIC = Path(__file__).parent / "static"


def jobs_root() -> Path:
    root = Path(tempfile.gettempdir()) / "onionskin-web"
    root.mkdir(parents=True, exist_ok=True)
    return root


def sweep_old_jobs(ttl: int = JOB_TTL_SECONDS) -> None:
    cutoff = time.time() - ttl
    for path in jobs_root().iterdir():
        try:
            if path.is_dir() and path.stat().st_mtime < cutoff:
                shutil.rmtree(path, ignore_errors=True)
        except OSError:
            continue


def job_dir(job_id: str) -> Path:
    """Resolve a job id to its directory, rejecting anything path-shaped."""
    if not JOB_ID_RE.match(job_id):
        raise HTTPException(status_code=404, detail="unknown job")
    path = jobs_root() / job_id
    if not path.is_dir():
        raise HTTPException(status_code=404, detail="unknown job")
    return path


async def save_upload(upload: UploadFile, destination: Path) -> Path:
    suffix = Path(upload.filename or "").suffix.lower()
    if suffix not in SUPPORTED:
        raise HTTPException(
            status_code=400,
            detail=(
                f"'{suffix or upload.filename}' is not a supported file type. "
                f"Use one of: {', '.join(sorted(SUPPORTED))}"
            ),
        )

    path = destination.with_suffix(suffix)
    size = 0
    with path.open("wb") as out:
        while chunk := await upload.read(1024 * 1024):
            size += len(chunk)
            if size > MAX_UPLOAD_BYTES:
                path.unlink(missing_ok=True)
                raise HTTPException(
                    status_code=413,
                    detail=f"file is larger than {MAX_UPLOAD_BYTES // (1024 * 1024)} MB",
                )
            out.write(chunk)
    if size == 0:
        path.unlink(missing_ok=True)
        raise HTTPException(status_code=400, detail="that file is empty")
    return path


def create_app() -> FastAPI:
    app = FastAPI(title="Onionskin", version=__version__)

    @app.get("/", response_class=HTMLResponse)
    async def index() -> HTMLResponse:
        return HTMLResponse((STATIC / "index.html").read_text(encoding="utf-8"))

    @app.get("/api/status")
    async def status() -> dict:
        return {
            "version": __version__,
            "libreoffice": find_soffice() is not None,
            "supported": sorted(SUPPORTED),
            "max_upload_mb": MAX_UPLOAD_BYTES // (1024 * 1024),
        }

    @app.get("/api/profiles")
    async def profiles() -> dict:
        return {
            "profiles": [
                {
                    "name": p.name,
                    "error": p.error.describe(),
                    "correction": p.correction.describe(),
                    "points": p.n_points,
                    "rms_mm": p.rms_residual_mm,
                    "notes": p.notes,
                }
                for p in calibrate.list_profiles()
            ]
        }

    @app.post("/api/delta")
    async def create_delta(
        original: UploadFile = File(...),
        edited: UploadFile = File(...),
        dpi: float = Form(pipeline.DEFAULT_DPI),
        mode: str = Form(pipeline.RASTER),
        margin: float = Form(5.0),
        profile: str = Form(""),
    ) -> JSONResponse:
        sweep_old_jobs()
        job_id = uuid.uuid4().hex
        work = jobs_root() / job_id
        work.mkdir(parents=True)

        try:
            original_path = await save_upload(original, work / "original")
            edited_path = await save_upload(edited, work / "edited")

            options = pipeline.Options(
                dpi=dpi,
                mode=mode,
                margin_mm=margin,
                profile=profile or None,
                preview_dir=work / "previews",
            )
            options.validate()

            result = pipeline.run(
                original_path, edited_path, work / "delta.pdf", options
            )
        except HTTPException:
            shutil.rmtree(work, ignore_errors=True)
            raise
        except (ConversionError, DocumentError, FileNotFoundError, ValueError) as exc:
            shutil.rmtree(work, ignore_errors=True)
            raise HTTPException(status_code=400, detail=str(exc)) from exc
        except Exception:
            shutil.rmtree(work, ignore_errors=True)
            raise

        payload = result.to_dict()
        payload["job"] = job_id
        payload["download"] = f"/api/jobs/{job_id}/delta.pdf"
        payload["previews"] = [
            f"/api/jobs/{job_id}/preview/{i + 1}" for i in range(len(result.previews))
        ]
        payload["source_name"] = Path(edited.filename or "document").stem
        return JSONResponse(payload)

    @app.get("/api/jobs/{job_id}/delta.pdf")
    async def download(job_id: str, name: str = "onionskin-delta") -> FileResponse:
        path = job_dir(job_id) / "delta.pdf"
        if not path.is_file():
            raise HTTPException(status_code=404, detail="no delta for that job")
        safe = re.sub(r"[^A-Za-z0-9._-]", "_", name)[:60] or "onionskin-delta"
        return FileResponse(
            path, media_type="application/pdf", filename=f"{safe}.pdf"
        )

    @app.get("/api/jobs/{job_id}/preview/{page}")
    async def preview(job_id: str, page: int) -> FileResponse:
        if page < 1 or page > 9999:
            raise HTTPException(status_code=404, detail="no such page")
        path = job_dir(job_id) / "previews" / f"page-{page:03d}.png"
        if not path.is_file():
            raise HTTPException(status_code=404, detail="no such page")
        return FileResponse(path, media_type="image/png")

    # --- typing directly onto the page ------------------------------------

    @app.get("/api/fonts")
    async def fonts() -> dict:
        return {"fonts": list(compose.available_fonts())}

    @app.post("/api/compose/open")
    async def compose_open(
        source: UploadFile = File(...), dpi: float = Form(110.0)
    ) -> JSONResponse:
        """Take one document and hand back page images to place text on."""
        sweep_old_jobs()
        job_id = uuid.uuid4().hex
        work = jobs_root() / job_id
        work.mkdir(parents=True)

        try:
            stored = await save_upload(source, work / "source")
            pages_dir = work / "pages"
            pages_dir.mkdir()

            with Workspace() as scratch:
                source_pdf = to_pdf(stored, scratch)
                # Keep a converted copy so rendering the delta later does not
                # have to run LibreOffice a second time.
                kept = work / "source.pdf"
                kept.write_bytes(Path(source_pdf).read_bytes())

                with Document(kept) as doc:
                    sizes = [
                        {
                            "width_mm": round(size.width_mm, 2),
                            "height_mm": round(size.height_mm, 2),
                            "label": size.describe(),
                        }
                        for size in doc.page_sizes
                    ]
                    for index in range(len(doc)):
                        image = Image.fromarray(doc.render(index, dpi).rgb)
                        image.save(
                            pages_dir / f"page-{index + 1:03d}.png",
                            format="PNG",
                            optimize=True,
                        )
        except HTTPException:
            shutil.rmtree(work, ignore_errors=True)
            raise
        except (ConversionError, DocumentError, FileNotFoundError, ValueError) as exc:
            shutil.rmtree(work, ignore_errors=True)
            raise HTTPException(status_code=400, detail=str(exc)) from exc
        except Exception:
            shutil.rmtree(work, ignore_errors=True)
            raise

        return JSONResponse(
            {
                "job": job_id,
                "name": Path(source.filename or "document").stem,
                "pages": sizes,
                "images": [
                    f"/api/jobs/{job_id}/page/{i + 1}" for i in range(len(sizes))
                ],
            }
        )

    @app.get("/api/jobs/{job_id}/page/{page}")
    async def source_page(job_id: str, page: int) -> FileResponse:
        if page < 1 or page > 9999:
            raise HTTPException(status_code=404, detail="no such page")
        path = job_dir(job_id) / "pages" / f"page-{page:03d}.png"
        if not path.is_file():
            raise HTTPException(status_code=404, detail="no such page")
        return FileResponse(path, media_type="image/png")

    @app.post("/api/compose/render")
    async def compose_render(
        job: str = Form(...),
        boxes: str = Form(...),
        dpi: float = Form(pipeline.DEFAULT_DPI),
        margin: float = Form(5.0),
        profile: str = Form(""),
    ) -> JSONResponse:
        work = job_dir(job)
        source_pdf = work / "source.pdf"
        if not source_pdf.is_file():
            raise HTTPException(status_code=404, detail="that document has expired")

        try:
            payload = json.loads(boxes)
        except json.JSONDecodeError as exc:
            raise HTTPException(status_code=400, detail="boxes is not valid JSON") from exc
        if not isinstance(payload, list):
            raise HTTPException(status_code=400, detail="boxes must be a list")

        try:
            parsed = [compose.TextBox.from_dict(item) for item in payload]
            options = pipeline.Options(
                dpi=dpi,
                margin_mm=margin,
                profile=profile or None,
                preview_dir=work / "previews",
            )
            options.validate()
            result = pipeline.compose_run(
                source_pdf, parsed, work / "delta.pdf", options
            )
        except (compose.LayoutError, FileNotFoundError, ValueError) as exc:
            raise HTTPException(status_code=400, detail=str(exc)) from exc

        data = result.to_dict()
        data["job"] = job
        data["download"] = f"/api/jobs/{job}/delta.pdf"
        data["previews"] = [
            f"/api/jobs/{job}/preview/{i + 1}" for i in range(len(result.previews))
        ]
        return JSONResponse(data)

    @app.post("/api/calibrate/target")
    async def calibration_target(page: str = Form("a4")) -> FileResponse:
        if page not in calibrate.PAGE_PRESETS:
            raise HTTPException(status_code=400, detail="unknown page size")
        sweep_old_jobs()
        work = jobs_root() / uuid.uuid4().hex
        work.mkdir(parents=True)
        path = calibrate.make_target(work / "target.pdf", calibrate.PAGE_PRESETS[page])
        return FileResponse(
            path, media_type="application/pdf", filename="onionskin-target.pdf"
        )

    @app.post("/api/calibrate/solve")
    async def solve(
        name: str = Form("default"),
        page: str = Form("a4"),
        points: str = Form(...),
    ) -> dict:
        if page not in calibrate.PAGE_PRESETS:
            raise HTTPException(status_code=400, detail="unknown page size")
        size = calibrate.PAGE_PRESETS[page]
        try:
            offsets = [
                calibrate.parse_point(spec)
                for spec in points.replace("\n", ";").split(";")
                if spec.strip()
            ]
            fit = calibrate.solve_from_offsets(offsets, size)
        except ValueError as exc:
            raise HTTPException(status_code=400, detail=str(exc)) from exc

        stored = calibrate.Profile(
            name=name or "default",
            error=fit.transform,
            page=size,
            rms_residual_mm=fit.rms_residual_mm,
            max_residual_mm=fit.max_residual_mm,
            n_points=fit.n_points,
            notes="measured in the browser",
        )
        calibrate.save_profile(stored)
        return {
            "name": stored.name,
            "error": stored.error.describe(),
            "correction": stored.correction.describe(),
            "rms_mm": round(fit.rms_residual_mm, 3),
            "max_mm": round(fit.max_residual_mm, 3),
            "points": fit.n_points,
        }

    @app.post("/api/calibrate/manual")
    async def manual(
        name: str = Form("default"),
        dx: float = Form(0.0),
        dy: float = Form(0.0),
        rotation: float = Form(0.0),
        scale: float = Form(1.0),
        page: str = Form("a4"),
    ) -> dict:
        if page not in calibrate.PAGE_PRESETS:
            raise HTTPException(status_code=400, detail="unknown page size")
        stored = calibrate.Profile(
            name=name or "default",
            error=Similarity(dx_mm=dx, dy_mm=dy, rotation_deg=rotation, scale=scale),
            page=calibrate.PAGE_PRESETS[page],
            notes="entered by hand",
        )
        calibrate.save_profile(stored)
        return {
            "name": stored.name,
            "error": stored.error.describe(),
            "correction": stored.correction.describe(),
        }

    @app.delete("/api/profiles/{name}")
    async def remove_profile(name: str) -> dict:
        if not calibrate.delete_profile(name):
            raise HTTPException(status_code=404, detail="no such profile")
        return {"deleted": name}

    return app


app = create_app()
