"""Tests for the pdfminer layout comparison. Run with ``uv run --group validation pytest``."""

from __future__ import annotations

from typing import Any

import pytest

import pawdf

compare: Any = pytest.importorskip('validation.compare_pdfminer', reason='needs the validation dependency group')

WORD = ((0.0, 0.0, 10.0, 10.0), 'a', [0])
LINE = ((0.0, 0.0, 10.0, 10.0), 'horizontal', 'a', [0], [WORD])
BLOCK = ((0.0, 0.0, 10.0, 10.0), 'horizontal', 'a', [LINE])
OTHER = ((0.0, 20.0, 10.0, 30.0), 'horizontal', 'b', [((0.0, 20.0, 10.0, 30.0), 'horizontal', 'b', [1], [])])


def _pages(name: str) -> pawdf.Document:
    return pawdf.Document(compare.fixtures()[name])


def test_identical_trees_have_no_mismatches() -> None:
    """Equal trees compare clean."""
    assert compare.compare_tree([BLOCK, OTHER], [BLOCK, OTHER]) == []


def test_order_difference_is_reported() -> None:
    """The same blocks in another order are a mismatch."""
    assert compare.compare_tree([BLOCK, OTHER], [OTHER, BLOCK])


def test_bbox_difference_is_reported() -> None:
    """Boxes are compared exactly."""
    moved = ((0.0, 0.0, 10.0 + 1e-9, 10.0), *BLOCK[1:])
    assert compare.compare_tree([BLOCK], [moved])


def test_word_difference_is_reported() -> None:
    """A word with other glyphs is a mismatch."""
    word = (WORD[0], WORD[1], [1])
    line = (*LINE[:4], [word])
    assert compare.compare_tree([BLOCK], [(*BLOCK[:3], [line])])


def test_pdfminer_ties_are_repeatable() -> None:
    """With creation-order ids, pdfminer gives the same order every run."""
    page = _pages('ties')[0]
    settings = compare.SETTINGS[0]
    assert compare.pdfminer_layout(page, settings) == compare.pdfminer_layout(page, settings)


def test_settings_mismatch_is_caught() -> None:
    """A setting changed on one side only shows up as mismatches."""
    found = []
    for data in compare.fixtures().values():
        for page in pawdf.Document(data):
            ours, _ = compare.pawdf_layout(page, {'char_margin': 0.5})
            found += compare.compare_tree(ours, compare.pdfminer_layout(page, {}))
    assert found, 'no fixture is sensitive to char_margin'


def test_every_fixture_matches() -> None:
    """Every fixture page agrees under every settings combination, with no limit hit."""
    for name, data in compare.fixtures().items():
        for page in pawdf.Document(data):
            for settings in compare.SETTINGS:
                ours, skipped = compare.pawdf_layout(page, settings)
                assert not skipped, (name, settings)
                theirs = compare.pdfminer_layout(page, settings)
                assert ours, (name, settings)
                assert compare.compare_tree(ours, theirs) == [], (name, page.index, settings)


def test_pdfminer_layout_uses_the_patched_id(monkeypatch: pytest.MonkeyPatch) -> None:
    """Pdfminer still breaks ties through the module-level ``id`` that the script patches."""
    calls = []
    real = compare._creation_order_ids

    def counting() -> Any:  # noqa: ANN401
        stable = real()

        def wrapped(obj: object) -> int:
            calls.append(obj)
            return stable(obj)

        return wrapped

    monkeypatch.setattr(compare, '_creation_order_ids', counting)
    compare.pdfminer_layout(_pages('ties')[0], {})
    assert calls


def test_a_pdf_with_no_blocks_fails() -> None:
    """A given PDF that compares no blocks at all is a failure, not a vacuous pass."""
    empty = pawdf.Document(compare._pdf(b''))
    assert compare.check_pdf('empty', empty, range(len(empty)), require_blocks=True) > 0
    assert compare.check_pdf('ties', _pages('ties'), range(1), require_blocks=True) == 0
