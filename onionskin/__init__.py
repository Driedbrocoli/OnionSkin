"""Onionskin — add words to a page that is already printed.

Onionskin writes a *delta PDF*: the same page size as your document, blank
except for the additions. Put the printed sheet back in the tray, print the
delta at 100%, and the new words land in the gaps.

Two ways to say what the additions are. Type them onto the page — nothing
reflows, because the text is placed at an absolute position::

    from onionskin import TextBox, compose_run

    compose_run(
        "po.docx",
        [TextBox(page=0, x_mm=45, y_mm=63, text="Approved 25 July")],
        "delta.pdf",
    )

Or edit in Word and let Onionskin work out what changed::

    from onionskin import run

    result = run("report.docx", "report-edited.docx", "delta.pdf")
    for check in result.checks:
        print(check.format())
"""

from .compose import TextBox
from .geometry import PageSize, Similarity, mm_to_pt, pt_to_mm, solve_similarity
from .pipeline import Options, Result, compose_run, run
from .safety import BLOCKER, NOTE, WARNING, Check

__version__ = "0.1.0"

__all__ = [
    "Options",
    "Result",
    "run",
    "compose_run",
    "TextBox",
    "PageSize",
    "Similarity",
    "Check",
    "BLOCKER",
    "WARNING",
    "NOTE",
    "solve_similarity",
    "mm_to_pt",
    "pt_to_mm",
    "__version__",
]
