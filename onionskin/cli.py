"""Command line interface."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from . import __version__, calibrate, pipeline, safety
from .geometry import PageSize
from .render import ConversionError, DocumentError, find_soffice

PRINT_INSTRUCTIONS = """
Printing the delta
  1. Put the already-printed sheet back in the tray. Check which way up and
     which end first — a page printed upside down is the usual first mistake.
  2. Print at 100% / "Actual size". Turn OFF "Fit to page" and "Shrink to fit";
     they scale by a few percent and nothing will line up.
  3. Print one page first and hold it to the light against the original before
     committing the rest.
""".strip()


def _fmt_mm(value: float) -> str:
    return f"{value:.1f}"


def _report(result: pipeline.Result, quiet: bool = False) -> None:
    out = sys.stdout
    if not quiet:
        print(f"\nWrote {result.output}", file=out)
        print(
            f"  {result.total_regions} addition(s) across "
            f"{len(result.pages_with_additions)} of {len(result.pages)} page(s)"
            f" — {result.total_added_mm2:.0f} mm² of new ink",
            file=out,
        )
        print(f"  mode {result.mode} at {result.dpi:g} dpi", file=out)
        if result.profile:
            print(
                f"  correcting for '{result.profile.name}': "
                f"{result.profile.correction.describe()}",
                file=out,
            )
        if result.previews:
            print(f"  previews in {result.previews[0].parent}", file=out)

        for page in result.pages:
            if not page.has_additions:
                continue
            print(f"\n  page {page.index + 1}:", file=out)
            for region in page.added_regions[:12]:
                print(
                    f"    {_fmt_mm(region.width_mm)}×{_fmt_mm(region.height_mm)} mm "
                    f"at ({_fmt_mm(region.x0_mm)}, {_fmt_mm(region.y0_mm)}) mm",
                    file=out,
                )
            if len(page.added_regions) > 12:
                print(
                    f"    … and {len(page.added_regions) - 12} more", file=out
                )

    if result.checks:
        print("", file=out)
        for check in result.checks:
            stream = sys.stderr if check.severity == safety.BLOCKER else out
            print(check.format(), file=stream)

    if not quiet and not result.blocked:
        print(f"\n{PRINT_INSTRUCTIONS}", file=out)


def cmd_delta(args: argparse.Namespace) -> int:
    options = pipeline.Options(
        dpi=args.dpi,
        mode=args.mode,
        margin_mm=args.margin,
        profile=args.profile,
        ink_threshold=args.ink_threshold,
        tolerance_mm=args.tolerance,
        group_mm=args.group,
        pad_mm=args.pad,
        preview_dir=Path(args.preview) if args.preview else None,
    )
    result = pipeline.run(args.original, args.edited, args.output, options)

    if args.json:
        print(json.dumps(result.to_dict(), indent=2))
    else:
        _report(result)

    if result.blocked and not args.force:
        print(
            "\nRefusing to recommend printing — see the blockers above. "
            "Pass --force if you know better.",
            file=sys.stderr,
        )
        return 2
    return 0


def cmd_inspect(args: argparse.Namespace) -> int:
    """Analyse without producing a delta anyone might print by accident."""
    import tempfile

    with tempfile.TemporaryDirectory(prefix="onionskin-inspect-") as tmp:
        options = pipeline.Options(
            dpi=args.dpi,
            mode=pipeline.RASTER,
            margin_mm=args.margin,
            preview_dir=Path(args.preview) if args.preview else None,
        )
        result = pipeline.run(
            args.original, args.edited, Path(tmp) / "scratch.pdf", options
        )
        result.output = Path("(not written)")
        if args.json:
            print(json.dumps(result.to_dict(), indent=2))
        else:
            print(
                f"\n{result.total_regions} addition(s) across "
                f"{len(result.pages_with_additions)} of {len(result.pages)} page(s)"
            )
            for page in result.pages:
                if page.has_additions or page.removed_ink_mm2:
                    print(
                        f"  page {page.index + 1}: "
                        f"+{page.added_ink_mm2:.0f} mm² added, "
                        f"-{page.removed_ink_mm2:.0f} mm² displaced, "
                        f"{len(page.added_regions)} region(s)"
                    )
            print("")
            for check in result.checks:
                print(check.format())
    return 2 if result.blocked else 0


def cmd_calibrate_target(args: argparse.Namespace) -> int:
    page = calibrate.PAGE_PRESETS[args.page]
    path = calibrate.make_target(args.output, page, inset_mm=args.inset)
    print(f"Wrote {path} ({page.describe()})")
    print(
        "\nPrint it at 100% on blank paper, put the same sheet back in the tray,\n"
        "then print the same file again. Read each crosshair's second impression\n"
        "against the ruler printed beside it, and feed those offsets to:\n"
        "  onionskin calibrate solve --point 'P1:+0.4,-0.2' ..."
    )
    return 0


def cmd_calibrate_solve(args: argparse.Namespace) -> int:
    page = calibrate.PAGE_PRESETS[args.page]
    try:
        offsets = [calibrate.parse_point(spec) for spec in args.point]
        fit = calibrate.solve_from_offsets(offsets, page, inset_mm=args.inset)
    except ValueError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1

    profile = calibrate.Profile(
        name=args.name,
        error=fit.transform,
        page=page,
        rms_residual_mm=fit.rms_residual_mm,
        max_residual_mm=fit.max_residual_mm,
        n_points=fit.n_points,
        notes=args.notes or "",
    )
    path = calibrate.save_profile(profile)
    print(profile.describe())
    print(f"\nSaved to {path}")

    if fit.n_points < 3:
        print(
            "\nnote: with only 2 points the fit has no redundancy — measure all "
            "five for a residual you can trust.",
            file=sys.stderr,
        )
    if fit.max_residual_mm > 0.4:
        print(
            f"\nwarning: worst point is off by {fit.max_residual_mm:.2f} mm. That is "
            "more than a similarity transform should leave behind — re-check the "
            "readings, or the paper may have skewed non-uniformly.",
            file=sys.stderr,
        )
    return 0


def cmd_calibrate_set(args: argparse.Namespace) -> int:
    from .geometry import Similarity

    profile = calibrate.Profile(
        name=args.name,
        error=Similarity(
            dx_mm=args.dx,
            dy_mm=args.dy,
            rotation_deg=args.rotation,
            scale=args.scale,
        ),
        page=calibrate.PAGE_PRESETS[args.page],
        notes=args.notes or "entered manually",
    )
    path = calibrate.save_profile(profile)
    print(profile.describe())
    print(f"\nSaved to {path}")
    return 0


def cmd_calibrate_list(args: argparse.Namespace) -> int:
    profiles = calibrate.list_profiles()
    if not profiles:
        print(
            f"No calibration profiles yet (looking in {calibrate.profiles_dir()}).\n"
            "Create one with: onionskin calibrate target -o target.pdf"
        )
        return 0
    for profile in profiles:
        print(profile.describe())
        print("")
    return 0


def cmd_calibrate_show(args: argparse.Namespace) -> int:
    try:
        print(calibrate.load_profile(args.name).describe())
    except FileNotFoundError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
    return 0


def cmd_calibrate_delete(args: argparse.Namespace) -> int:
    if calibrate.delete_profile(args.name):
        print(f"Deleted profile '{args.name}'")
        return 0
    print(f"error: no profile '{args.name}'", file=sys.stderr)
    return 1


def cmd_serve(args: argparse.Namespace) -> int:
    try:
        import uvicorn
    except ImportError:
        print(
            "error: the web app needs fastapi and uvicorn.\n"
            "  pip install 'onionskin[web]'",
            file=sys.stderr,
        )
        return 1
    from .web.app import create_app

    print(f"Onionskin running at http://{args.host}:{args.port}")
    uvicorn.run(create_app(), host=args.host, port=args.port, log_level="warning")
    return 0


def cmd_doctor(args: argparse.Namespace) -> int:
    ok = True
    soffice = find_soffice()
    if soffice:
        print(f"  LibreOffice   {soffice}")
    else:
        ok = False
        print(
            "  LibreOffice   NOT FOUND — Word documents cannot be converted.\n"
            "                Install it, or set ONIONSKIN_SOFFICE, or pass PDFs."
        )

    for module, label in (
        ("pypdfium2", "pypdfium2"),
        ("pikepdf", "pikepdf"),
        ("reportlab", "reportlab"),
        ("PIL", "Pillow"),
        ("numpy", "numpy"),
    ):
        try:
            __import__(module)
            print(f"  {label:<13} ok")
        except ImportError:
            ok = False
            print(f"  {label:<13} MISSING")

    try:
        import fastapi  # noqa: F401
        import uvicorn  # noqa: F401

        print("  web app       ok")
    except ImportError:
        print("  web app       not installed (pip install 'onionskin[web]')")

    profiles = calibrate.list_profiles()
    print(f"  profiles      {len(profiles)} in {calibrate.profiles_dir()}")
    for profile in profiles:
        print(f"                - {profile.name}: {profile.error.describe()}")
    if not profiles:
        print("                run 'onionskin calibrate target' to make one")

    return 0 if ok else 1


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="onionskin",
        description="Add words to a page that is already printed.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=(
            "examples:\n"
            "  onionskin delta report.docx report-v2.docx -o delta.pdf\n"
            "  onionskin delta a.docx b.docx -o delta.pdf --profile office --preview ./proof\n"
            "  onionskin inspect a.docx b.docx\n"
            "  onionskin calibrate target -o target.pdf\n"
            "  onionskin serve\n"
        ),
    )
    parser.add_argument("--version", action="version", version=f"onionskin {__version__}")
    subs = parser.add_subparsers(dest="command", required=True)

    def add_diff_options(sub: argparse.ArgumentParser) -> None:
        sub.add_argument("original", help="the document as it was printed")
        sub.add_argument("edited", help="the same document with additions")
        sub.add_argument(
            "--dpi", type=float, default=pipeline.DEFAULT_DPI,
            help=f"comparison resolution (default {pipeline.DEFAULT_DPI:g})",
        )
        sub.add_argument(
            "--margin", type=float, default=safety.DEFAULT_MARGIN_MM,
            help="non-printable border to warn about, mm (default 5)",
        )
        sub.add_argument("--preview", help="directory to write proof PNGs into")
        sub.add_argument("--json", action="store_true", help="machine-readable output")

    delta = subs.add_parser("delta", help="write the delta PDF")
    add_diff_options(delta)
    delta.add_argument("-o", "--output", required=True, help="delta PDF to write")
    delta.add_argument(
        "--mode", choices=pipeline.MODES, default=pipeline.RASTER,
        help=(
            "raster prints only genuinely-new pixels (default); "
            "vector keeps crisp text but may re-print ink adjacent to an addition"
        ),
    )
    delta.add_argument("--profile", help="calibration profile to correct with")
    delta.add_argument(
        "--ink-threshold", type=int, default=200,
        help="grey level at or below which a pixel counts as ink (default 200)",
    )
    delta.add_argument(
        "--tolerance", type=float, default=0.12,
        help="how far a mark may move and still count as unchanged, mm (default 0.12)",
    )
    delta.add_argument(
        "--group", type=float, default=2.0,
        help="merge additions closer than this into one region, mm (default 2)",
    )
    delta.add_argument(
        "--pad", type=float, default=0.3,
        help="vector mode only: grow each clip box by this much, mm (default 0.3)",
    )
    delta.add_argument(
        "--force", action="store_true",
        help="exit 0 even when a blocker was found",
    )
    delta.set_defaults(func=cmd_delta)

    inspect = subs.add_parser(
        "inspect", help="report what changed without writing a delta"
    )
    add_diff_options(inspect)
    inspect.set_defaults(func=cmd_inspect)

    cal = subs.add_parser("calibrate", help="measure and store printer registration")
    cal_subs = cal.add_subparsers(dest="calibrate_command", required=True)

    target = cal_subs.add_parser("target", help="write the two-pass target page")
    target.add_argument("-o", "--output", default="onionskin-target.pdf")
    target.add_argument("--page", choices=sorted(calibrate.PAGE_PRESETS), default="a4")
    target.add_argument(
        "--inset", type=float, default=25.0,
        help="how far the corner crosshairs sit from the edges, mm (default 25)",
    )
    target.set_defaults(func=cmd_calibrate_target)

    solve = cal_subs.add_parser("solve", help="fit a profile from measured offsets")
    solve.add_argument(
        "--point", action="append", required=True, metavar="P1:DX,DY",
        help="measured offset at a crosshair, mm; repeat for each one measured",
    )
    solve.add_argument("--name", default="default", help="profile name")
    solve.add_argument("--page", choices=sorted(calibrate.PAGE_PRESETS), default="a4")
    solve.add_argument("--inset", type=float, default=25.0)
    solve.add_argument("--notes", help="free text, e.g. the printer and tray")
    solve.set_defaults(func=cmd_calibrate_solve)

    manual = cal_subs.add_parser("set", help="enter a correction by hand")
    manual.add_argument("--name", default="default")
    manual.add_argument("--dx", type=float, default=0.0, help="printer shift right, mm")
    manual.add_argument("--dy", type=float, default=0.0, help="printer shift down, mm")
    manual.add_argument(
        "--rotation", type=float, default=0.0, help="printer rotation, degrees clockwise"
    )
    manual.add_argument("--scale", type=float, default=1.0, help="printer scale factor")
    manual.add_argument("--page", choices=sorted(calibrate.PAGE_PRESETS), default="a4")
    manual.add_argument("--notes")
    manual.set_defaults(func=cmd_calibrate_set)

    listing = cal_subs.add_parser("list", help="list stored profiles")
    listing.set_defaults(func=cmd_calibrate_list)

    show = cal_subs.add_parser("show", help="print one profile")
    show.add_argument("name")
    show.set_defaults(func=cmd_calibrate_show)

    remove = cal_subs.add_parser("delete", help="remove a profile")
    remove.add_argument("name")
    remove.set_defaults(func=cmd_calibrate_delete)

    serve = subs.add_parser("serve", help="run the browser app")
    serve.add_argument("--host", default="127.0.0.1")
    serve.add_argument("--port", type=int, default=8000)
    serve.set_defaults(func=cmd_serve)

    doctor = subs.add_parser("doctor", help="check the local setup")
    doctor.set_defaults(func=cmd_doctor)

    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        return args.func(args)
    except (ConversionError, DocumentError, FileNotFoundError, ValueError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print("\ninterrupted", file=sys.stderr)
        return 130


if __name__ == "__main__":
    sys.exit(main())
