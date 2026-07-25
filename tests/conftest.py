"""Shared pytest fixtures. Document builders live in helpers.py."""

import pytest


@pytest.fixture
def onionskin_home(tmp_path, monkeypatch):
    """Point profile storage at a temp dir so tests never touch ~/.onionskin."""
    home = tmp_path / "home"
    monkeypatch.setenv("ONIONSKIN_HOME", str(home))
    return home
