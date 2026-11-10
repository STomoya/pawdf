"""Tests for the layout binding."""

from __future__ import annotations

from typing import Any

import pytest

import pawdf
from tests.test_document import FONT_RES, _pdf


def _page(content: bytes, rotate: int = 0) -> pawdf.Page:
    data = _pdf(content, FONT_RES)
    if rotate:
        data = data.replace(b'/MediaBox [0 0 612 792]', b'/MediaBox [0 0 612 792] /Rotate %d' % rotate)
        data = _pdf_fix_xref(data)
    return pawdf.Document(data)[0]


def _pdf_fix_xref(data: bytes) -> bytes:
    # Dropping the xref makes the reader rebuild it by scanning, which tolerates the edit.
    return data[: data.index(b'xref\n')] + data[data.index(b'trailer') :]


def test_structure() -> None:
    """Blocks, lines and words carry text, boxes and glyph indices."""
    page = _page(b'BT /F1 10 Tf 100 200 Td (ab cd) Tj ET')
    glyphs = page.content().glyphs
    layout = pawdf.layout.pdfminer(page)
    [block] = layout.blocks
    [line] = block.lines
    assert block.text == line.text == 'ab cd'
    assert [w.text for w in line.words] == ['ab', 'cd']
    assert line.words[0].glyphs == [0, 1]
    assert line.glyphs == [0, 1, 2, 3, 4]
    assert line.orientation == block.orientation == 'horizontal'
    a, b = glyphs[0].bbox, glyphs[1].bbox
    assert line.words[0].bbox == pytest.approx((a[0], min(a[1], b[1]), b[2], max(a[3], b[3])))


def test_settings_are_keyword_only() -> None:
    """Settings can't be passed positionally."""
    untyped: Any = pawdf.layout.pdfminer
    with pytest.raises(TypeError):
        untyped(_page(b''), 0.5)


def test_invalid_setting_raises_value_error() -> None:
    """Out-of-range or non-finite settings raise ValueError."""
    page = _page(b'')
    with pytest.raises(ValueError, match='boxes_flow'):
        pawdf.layout.pdfminer(page, boxes_flow=2.0)
    with pytest.raises(ValueError, match='char_margin'):
        pawdf.layout.pdfminer(page, char_margin=float('nan'))


def test_settings_are_applied() -> None:
    """boxes_flow=None and a zero word_margin are accepted and change the result."""
    page = _page(b'BT /F1 10 Tf 100 200 Td (ab cd) Tj ET')
    assert pawdf.layout.pdfminer(page, boxes_flow=None).blocks[0].text == 'ab cd'
    spaced = _page(b'BT /F1 10 Tf 100 200 Td (a) Tj 8 0 Td (b) Tj ET')
    assert pawdf.layout.pdfminer(spaced).blocks[0].text == 'a b'
    assert pawdf.layout.pdfminer(spaced, word_margin=0).blocks[0].text == 'ab'


def test_read_diagnostics_come_first() -> None:
    """Reading diagnostics are kept, before layout's."""
    layout = pawdf.layout.pdfminer(_page(b'Q'))
    assert layout.diagnostics[0].kind == 'unbalanced_restore'
    assert layout.blocks == []


def test_reprs() -> None:
    """Every layout class has a readable repr."""
    layout = pawdf.layout.pdfminer(_page(b'BT /F1 10 Tf 100 200 Td (ab) Tj ET'))
    block = layout.blocks[0]
    for obj in (layout, block, block.lines[0], block.lines[0].words[0]):
        assert repr(obj).startswith(type(obj).__name__ + '(')


def test_rotated_page_boxes_are_upright() -> None:
    """Boxes are in upright page space: x spans the page's displayed width."""
    width, height, rotate = 612, 792, 90
    page = _page(b'BT /F1 10 Tf 100 200 Td (ab) Tj ET', rotate=rotate)
    assert page.rotate == rotate
    x0, y0, x1, y1 = pawdf.layout.pdfminer(page).blocks[0].bbox
    assert 0 <= x0 < x1 <= height
    assert 0 <= y0 < y1 <= width
    assert x0 == pytest.approx(200 - 2)
