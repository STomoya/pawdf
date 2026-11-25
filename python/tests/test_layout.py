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


def test_text_in_regions() -> None:
    """Each display-space region gets its own layout, in the order given."""
    page = _page(b'Q BT /F1 10 Tf 100 200 Td (ab cd) Tj ET')
    split = page.content().glyphs[3].bbox[0]
    left, right = (0, 0, split, 792), (split, 0, 612, 792)
    layouts = pawdf.layout.text_in_regions(page, [right, left, (0, 0, 1, 1)])
    words = [[w.text for b in lay.blocks for ln in b.lines for w in ln.words] for lay in layouts]
    assert words == [['cd'], ['ab'], []]
    assert all(lay.diagnostics[0].kind == 'unbalanced_restore' for lay in layouts)
    assert layouts[0].diagnostics[0] is layouts[2].diagnostics[0]
    with pytest.raises(ValueError, match='regions'):
        pawdf.layout.text_in_regions(page, [(0, float('nan'), 1, 1)])


def test_tables_tagged() -> None:
    """Cells carry their grid position, header flag, display-space box and text."""
    shows = [(0, 100, 700, b'ab'), (1, 200, 700, b'cd'), (2, 100, 680, b'ef'), (3, 200, 680, b'gh')]
    content = b' '.join(b'/Span << /MCID %d >> BDC BT /F1 10 Tf %d %d Td (%s) Tj ET EMC' % s for s in shows)
    cells = tuple(
        b'<< /S /%s /P %d 0 R /Pg 3 0 R /K %d %s >>' % (role, 8 + i // 2, i, attrs)
        for i, (role, attrs) in enumerate(
            [(b'TH', b''), (b'TH', b''), (b'TD', b'/A << /O /Table /ColSpan 2 >>'), (b'TD', b'')]
        )
    )
    tree = (
        b'<< /Type /StructTreeRoot /K 7 0 R >>',
        b'<< /S /Table /P 6 0 R /K [8 0 R 9 0 R] >>',
        b'<< /S /TR /P 7 0 R /K [10 0 R 11 0 R] >>',
        b'<< /S /TR /P 7 0 R /K [12 0 R 13 0 R] >>',
        *cells,
    )
    doc = pawdf.Document(_pdf(content, FONT_RES, extra=tree, catalog_extra=b'/StructTreeRoot 6 0 R'))
    page, t = doc[0], doc.structure_tree()
    [table] = pawdf.layout.tables_tagged(page, t)
    assert (table.rows, table.cols) == (2, 3)
    got = [(c.row, c.col, c.row_span, c.col_span, c.header, c.layout.blocks[0].text) for c in table.cells]
    assert got == [
        (0, 0, 1, 1, True, 'ab'),
        (0, 1, 1, 1, True, 'cd'),
        (1, 0, 1, 2, False, 'ef'),
        (1, 2, 1, 1, False, 'gh'),
    ]
    bbox = table.cells[0].bbox
    assert bbox is not None
    a, b = (g.bbox for g in page.content().glyphs[:2])
    assert bbox == pytest.approx((a[0], 792 - max(a[3], b[3]), b[2], 792 - min(a[1], b[1])))
    assert table.bbox[0] == pytest.approx(100)
    assert repr(table) == 'Table(rows=2, cols=3, cells=4)'
    assert repr(table.cells[0]) == 'Cell(row=0, col=0, row_span=1, col_span=1, header=True)'
    assert table.cells[0].layout.diagnostics == []
    assert pawdf.layout.tables_tagged(page, t, (0, 0, 50, 50)) == []
    assert len(pawdf.layout.tables_tagged(page, t, (150, 150, 0, 0))) == 1
    with pytest.raises(ValueError, match='region'):
        pawdf.layout.tables_tagged(page, t, (float('nan'), 0, 1, 1))
