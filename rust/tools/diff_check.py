#!/usr/bin/env python3
"""Check the Rust port against the Python it replaces, number for number.

A rewrite earns trust by agreeing with the implementation it replaces, not by
passing its own tests — its own tests can encode the same misunderstanding
twice. Each ported module gets an `examples/dump_*.rs` that prints its results
as JSON; this script recomputes them in Python and compares.

    cargo run --quiet --example dump_geometry | python3 tools/diff_check.py geometry
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO))

TOLERANCE = 1e-8


def check_geometry(rows: list[dict]) -> tuple[int, float, list]:
    from onionskin.geometry import PageSize, Similarity, solve_similarity

    checked, worst, bad = 0, 0.0, []

    def compare(label: str, got, want, row, tol=TOLERANCE):
        nonlocal checked, worst
        for a, b in zip(got, want):
            delta = abs(a - b)
            worst = max(worst, delta)
            checked += 1
            if delta > tol:
                bad.append((label, row, a, b))

    for row in rows:
        if "fit" in row:
            page = PageSize(210.0, 297.0)
            nominal = [(25.0, 25.0), (185.0, 25.0), (25.0, 272.0),
                       (185.0, 272.0), (105.0, 148.5)]
            truth = Similarity(0.42, -0.31, 0.18, 1.0021)
            noise = [(0.1, -0.1), (-0.1, 0.1), (0.12, 0.08),
                     (-0.08, -0.12), (0.05, 0.05)]
            observed = [
                (truth.apply(p, page)[0] + n[0], truth.apply(p, page)[1] + n[1])
                for p, n in zip(nominal, noise)
            ]
            fit = solve_similarity(nominal, observed, page)
            compare("fit", [fit.transform.dx_mm, fit.transform.dy_mm,
                            fit.transform.rotation_deg, fit.transform.scale],
                    row["fit"], row)
            compare("residuals", [fit.rms_residual_mm, fit.max_residual_mm],
                    [row["rms"], row["max"]], row)
            continue

        page = PageSize(*row["page"])
        transform = Similarity(*row["t"])
        point = tuple(row["p"])

        compare("apply", transform.apply(point, page), row["apply"], row)

        inverse = transform.inverse()
        compare("inverse",
                [inverse.dx_mm, inverse.dy_mm, inverse.rotation_deg, inverse.scale],
                row["inv"], row)

        matrix = transform.to_pdf_matrix(page)
        compare("matrix",
                [matrix.a, matrix.b, matrix.c, matrix.d, matrix.e, matrix.f],
                row["m"], row)

    return checked, worst, bad


CHECKERS = {"geometry": check_geometry}


def main() -> int:
    if len(sys.argv) != 2 or sys.argv[1] not in CHECKERS:
        print(f"usage: {sys.argv[0]} [{'|'.join(CHECKERS)}]", file=sys.stderr)
        return 2

    module = sys.argv[1]
    rows = json.load(sys.stdin)
    checked, worst, bad = CHECKERS[module](rows)

    print(f"{module}: compared {checked} values, worst difference {worst:.2e}")
    if bad:
        print(f"{len(bad)} MISMATCH(ES):", file=sys.stderr)
        for label, row, got, want in bad[:10]:
            print(f"  {label}: python={got!r} rust={want!r}\n    row={row}", file=sys.stderr)
        return 1
    print("identical within tolerance")
    return 0


if __name__ == "__main__":
    sys.exit(main())
