"""Onionskin — add words to a page that is already printed.

Feed Onionskin the original document and an edited copy. It renders both,
works out which ink is new, and writes a *delta PDF*: the same page size, blank
except for the additions. Put the printed sheet back in the tray, print the
delta at 100%, and the new words land in the gaps.

    from onionskin import pipeline

    result = pipeline.run("report.docx", "report-edited.docx", "delta.pdf")
    for check in result.checks:
        print(check.format())
"""

from .geometry import PageSize, Similarity, mm_to_pt, pt_to_mm, solve_similarity
from .pipeline import Options, Result, run
from .safety import BLOCKER, NOTE, WARNING, Check

__version__ = "0.1.0"

__all__ = [
    "Options",
    "Result",
    "run",
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
