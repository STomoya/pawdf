"""Regression tests for the pdfium comparison script. Run with ``uv run --group validation pytest``."""

from __future__ import annotations

from typing import Any

import pytest

compare: Any = pytest.importorskip('validation.compare_pdfium', reason='needs the validation dependency group')


def _report(ours: list, theirs: list) -> Any:  # noqa: ANN401
    report = compare.PageReport(0)
    compare.compare_glyphs(report, ours, theirs, [], [])
    return report


def test_reordered_repeats_are_not_mismatches() -> None:
    """Pdfium reorders text objects, so the same characters at the same origins agree in any order."""
    ours = [('a', 0.0, 10.0), ('b', 5.0, 10.0), ('a', 0.0, 0.0), ('b', 5.0, 0.0)]
    theirs = [('a', 0.0, 0.0), ('b', 5.0, 0.0), ('a', 0.0, 10.0), ('b', 5.0, 10.0)]
    assert _report(ours, theirs).ok


def test_moved_character_is_a_position_mismatch() -> None:
    """The same character at another origin is reported once, as a position mismatch."""
    report = _report([('x', 0.0, 0.0)], [('x', 5.0, 0.0)])
    assert len(report.position) == 1
    assert report.unicode == []


def test_different_characters_are_unicode_mismatches() -> None:
    """Different characters are reported on both sides."""
    report = _report([('a', 0.0, 0.0)], [('b', 0.0, 0.0)])
    assert report.position == []
    assert report.unicode == ["ours 'a' at (0.00, 0.00) not in pdfium", "pdfium 'b' at (0.00, 0.00) not in ours"]


def test_pdfium_space_may_match_a_collapsed_space() -> None:
    """Pdfium collapses spaces in its line order, so it can keep one that stream order collapsed."""
    ours = [('a', 0.0, 0.0)]
    theirs = [('a', 0.0, 0.0), (' ', 5.0, 0.0)]
    report = compare.PageReport(0)
    compare.compare_glyphs(report, ours, theirs, [], [(5.0, 0.0)])
    assert report.ok
    assert len(_report(ours, theirs).unicode) == 1


def test_pawdf_chars_returns_collapsed_spaces() -> None:
    """The second space of a run is left out of the characters and returned separately."""
    data = (
        b'%PDF-1.7\n1 0 obj\n<< /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Kids [3 0 R] /Count 1 >>\nendobj\n'
        b'3 0 obj\n<< /Parent 2 0 R /MediaBox [0 0 99 99] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>\n'
        b'endobj\n4 0 obj\n<< /Length 30 >>\nstream\nBT /F1 10 Tf (a  ) Tj ET      \nendstream\nendobj\n'
        b'5 0 obj\n<< /Subtype /Type1 /BaseFont /Courier >>\nendobj\ntrailer\n<< /Root 1 0 R >>\n'
    )
    chars, collapsed = compare.pawdf_chars(compare.pawdf.Document(data)[0].content())
    assert chars == [('a', 0.0, 0.0), (' ', 6.0, 0.0)]
    assert collapsed == [(12.0, 0.0)]


def test_decode_chars_joins_surrogates() -> None:
    """Pdfium reports an astral character as two UTF-16 code units."""
    chars = [(0xD835, 1.0, 2.0), (0xDC50, 1.0, 2.0), (0x2, 3.0, 4.0), (0x41, 5.0, 6.0), (0xD835, 7.0, 8.0)]
    assert compare.decode_chars(chars) == [
        ('\U0001d450', 1.0, 2.0),
        ('-', 3.0, 4.0),
        ('A', 5.0, 6.0),
        ('\ud835', 7.0, 8.0),
    ]
