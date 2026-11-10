"""Compare pawdf's layout analysis with pdfminer.six's on the same glyph boxes.

Run: uv run --group validation python python/validation/compare_pdfminer.py [PDF ...] [--pages 0,1]

Generated fixture PDFs always run; PDFs given on the command line run too. Every page runs
under every settings combination in ``SETTINGS``.

pdfminer's parser is not used. Glyphs come from ``Page.content()``: hidden ones and non-finite
boxes are dropped, boxes are mapped into pdfminer's page space with its own
``apply_matrix_rect`` and the CTM of ``PDFPageInterpreter.process_page``, and each becomes an
``LTChar`` added straight to an ``LTPage``. pdfminer breaks ties between equally distant boxes
with ``id()``, a memory address; ``pdfminer.layout.id`` is patched to number objects in the
order it first sees them, which is what pawdf does. This patches a pdfminer internal, so
recheck it before raising the pdfminer version bound.

pdfminer has no words. They are derived with pawdf's rule: a word is a run of a line's
characters between inserted spaces (``LTAnno(' ')``) and characters whose text is whitespace.

Blocks, lines and words are compared exactly: order, glyph indices, orientation, text and
boxes, with no rounding. A page where pawdf reports ``layout_limit`` is a skip, not a pass.
The exit status is 1 when anything mismatched or was skipped.
"""

from __future__ import annotations

import argparse
import builtins
import math
import sys
from pathlib import Path
from typing import Any

import pdfminer.layout as pml
from pdfminer.layout import (
    LAParams,
    LTAnno,
    LTChar,
    LTComponent,
    LTPage,
    LTTextBox,
    LTTextBoxVertical,
    LTTextLineVertical,
)
from pdfminer.utils import apply_matrix_rect

import pawdf

SHOW = 5

SETTINGS: list[dict[str, Any]] = [
    {},
    {'boxes_flow': None},
    {'boxes_flow': -1.0},
    {'boxes_flow': 0.0},
    {'boxes_flow': 1.0},
    {'detect_vertical': True},
    {'word_margin': 0.0},
    {'char_margin': 1.0, 'line_margin': 0.3, 'line_overlap': 0.3},
]

Rect = tuple[float, float, float, float]
Matrix = tuple[float, float, float, float, float, float]
Word = tuple[Rect, str, list[int]]
Line = tuple[Rect, str, str, list[int], list[Word]]
Block = tuple[Rect, str, str, list[Line]]
Tree = list[Block]


class _Char(LTChar):
    """An ``LTChar`` that remembers its glyph index; built without ``__init__``, which needs a PDF font."""

    glyph: int


def page_space(media_box: Rect, rotate: int) -> tuple[Matrix, Rect]:
    """The CTM of ``PDFPageInterpreter.process_page`` and the bound of ``PDFLayoutAnalyzer.begin_page``."""
    x0, y0, x1, y1 = media_box
    ctm: Matrix = {
        90: (0, -1, 1, 0, -y0, x1),
        180: (-1, 0, 0, -1, x1, y1),
        270: (0, 1, -1, 0, y1, -x0),
    }.get(rotate, (1, 0, 0, 1, -x0, -y0))
    tx0, ty0, tx1, ty1 = apply_matrix_rect(ctm, media_box)
    return ctm, (0, 0, abs(tx0 - tx1), abs(ty0 - ty1))


def _finite(r: Rect) -> bool:
    return all(math.isfinite(v) for v in r)


def _chars(page: pawdf.Page, content: pawdf.PageContent) -> tuple[list[_Char], Rect]:
    ctm, bound = page_space(page.media_box, page.rotate)
    chars = []
    for i, g in enumerate(content.glyphs):
        if g.hidden or not _finite(g.bbox):
            continue
        box = apply_matrix_rect(ctm, g.bbox)
        if not _finite(box):
            continue
        c = _Char.__new__(_Char)
        LTComponent.__init__(c, box)
        c._text = g.text
        c.glyph = i
        chars.append(c)
    return chars, bound


def _creation_order_ids() -> Any:  # noqa: ANN401
    seen: dict[int, int] = {}
    keep: list[object] = []

    def stable(obj: object) -> int:
        key = builtins.id(obj)
        if key not in seen:
            seen[key] = len(seen)
            keep.append(obj)
        return seen[key]

    return stable


def _union(acc: Rect, r: Rect) -> Rect:
    return (min(acc[0], r[0]), min(acc[1], r[1]), max(acc[2], r[2]), max(acc[3], r[3]))


def _line(line: Any) -> Line:  # noqa: ANN401
    items = list(line)[:-1]
    words: list[Word] = []
    current: list[_Char] = []
    for item in [*items, LTAnno(' ')]:
        if isinstance(item, _Char) and not item.get_text().isspace():
            current.append(item)
            continue
        if current:
            box = current[0].bbox
            for c in current[1:]:
                box = _union(box, c.bbox)
            words.append((box, ''.join(c.get_text() for c in current), [c.glyph for c in current]))
            current = []
    orientation = 'vertical' if isinstance(line, LTTextLineVertical) else 'horizontal'
    text = ''.join(item.get_text() for item in items)
    glyphs = [item.glyph for item in items if isinstance(item, _Char)]
    return (line.bbox, orientation, text, glyphs, words)


def pdfminer_layout(page: pawdf.Page, settings: dict[str, Any], content: pawdf.PageContent | None = None) -> Tree:
    """Run pdfminer's layout analysis on pawdf's glyphs for ``page``."""
    chars, bound = _chars(page, content or page.content())
    lt = LTPage(page.index, bound)
    for c in chars:
        lt.add(c)
    vars(pml)['id'] = _creation_order_ids()
    try:
        if chars:
            lt.analyze(LAParams(**settings))
    finally:
        del vars(pml)['id']
    tree: Tree = []
    for box in lt:
        if not isinstance(box, LTTextBox):
            continue
        lines = [_line(line) for line in box]
        orientation = 'vertical' if isinstance(box, LTTextBoxVertical) else 'horizontal'
        tree.append((box.bbox, orientation, '\n'.join(line[2] for line in lines), lines))
    return tree


def _pawdf_line(ln: pawdf.layout.Line) -> Line:
    return (ln.bbox, ln.orientation, ln.text, ln.glyphs, [(w.bbox, w.text, w.glyphs) for w in ln.words])


def pawdf_layout(page: pawdf.Page, settings: dict[str, Any]) -> tuple[Tree, bool]:
    """Pawdf's layout as a tree, and whether a layout limit was hit."""
    layout = pawdf.layout.pdfminer(page, **settings)
    tree: Tree = [
        (
            b.bbox,
            b.orientation,
            b.text,
            [_pawdf_line(ln) for ln in b.lines],
        )
        for b in layout.blocks
    ]
    return tree, any(d.kind == 'layout_limit' for d in layout.diagnostics)


def compare_tree(ours: Tree, theirs: Tree) -> list[str]:
    """Differences between two trees, block by block."""
    out = [f'ours {len(ours)} blocks, pdfminer {len(theirs)}'] if len(ours) != len(theirs) else []
    for i, (a, b) in enumerate(zip(ours, theirs, strict=False)):
        if a != b:
            out.append(f'block {i}: ours {a!r}\n    pdfminer {b!r}')
    return out


def _stream(data: bytes, dict_extra: bytes = b'') -> bytes:
    return b'<< /Length %d %s >>\nstream\n' % (len(data), dict_extra) + data + b'\nendstream'


def _text(lines: list[tuple[float, float, str]], size: int = 10) -> bytes:
    """``BT`` blocks showing each string at its position in Helvetica."""
    return b''.join(b'BT /F1 %d Tf %g %g Td (%s) Tj ET\n' % (size, x, y, s.encode()) for x, y, s in lines)


FONTS = b'<< /Font << /F1 5 0 R /F2 8 0 R >> >>'
SHARED = (
    b'<< /Type /OCG /Name (L) >>',
    _stream(
        _text([(0, 0, 'form text'), (0, -12, 'second form line')]),
        b'/Type /XObject /Subtype /Form /BBox [0 0 300 300] /Resources ' + FONTS,
    ),
    b'<< /Type /Font /Subtype /Type0 /BaseFont /Foo /Encoding /Identity-V /DescendantFonts [9 0 R] >>',
    (
        b'<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Foo '
        b'/CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> >>'
    ),
)


def _pdf(content: bytes, page_extra: bytes = b'', media_box: bytes = b'[0 0 612 792]') -> bytes:
    """A one-page PDF with fonts /F1 (Helvetica) and /F2 (Identity-V), form /Fm0 and hidden group /L."""
    objects = [
        b'<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [6 0 R] /D << /OFF [6 0 R] >> >> >>',
        b'<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
        b'<< /Type /Page /Parent 2 0 R /MediaBox %s /Contents 4 0 R %s /Resources '
        b'<< /Font << /F1 5 0 R /F2 8 0 R >> /XObject << /Fm0 7 0 R >> /Properties << /L 6 0 R >> >> >>'
        % (media_box, page_extra),
        _stream(content),
        b'<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
        *SHARED,
    ]
    out = bytearray(b'%PDF-1.7\n')
    offsets = []
    for i, body in enumerate(objects, start=1):
        offsets.append(len(out))
        out += b'%d 0 obj\n' % i + body + b'\nendobj\n'
    xref = len(out)
    out += b'xref\n0 %d\n0000000000 65535 f \n' % (len(objects) + 1)
    for off in offsets:
        out += b'%010d 00000 n \n' % off
    out += b'trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n' % (len(objects) + 1, xref)
    return bytes(out)


def _columns() -> bytes:
    words = [
        'layout',
        'analysis',
        'groups',
        'glyphs',
        'into',
        'words',
        'lines',
        'and',
        'blocks',
        'in',
        'reading',
        'order',
    ]
    out = [(72.0, 720.0, 'A heading over both columns')]
    for col, x in enumerate((72.0, 320.0)):
        for row in range(8):
            y = 690.0 - row * 12 - (40 if row >= 4 else 0)  # noqa: PLR2004 -- a paragraph break.
            out.append((x, y, ' '.join(words[(row + col) % 5 : (row + col) % 5 + 6])))
    # Kerned gaps without space glyphs, so that char_margin and word_margin decide the grouping.
    return _text(out) + b'BT /F1 10 Tf 300 400 Td [(gap) -800 (ped) -1500 (words) -300 (here)] TJ ET\n'


def fixtures() -> dict[str, bytes]:
    """Small PDFs covering what the real PDFs don't: rotation, vertical text, hidden content and more."""
    body = _columns()
    vertical = b''.join(b'BT /F2 12 Tf %d 700 Td <0001000200030004000500060007> Tj ET\n' % x for x in (500, 486, 400))
    return {
        'columns': _pdf(body),
        'rotate90': _pdf(body, b'/Rotate 90'),
        'rotate180': _pdf(body, b'/Rotate 180'),
        'rotate270': _pdf(body, b'/Rotate 270'),
        'origin': _pdf(body, media_box=b'[100 50 712 842]'),
        'vertical': _pdf(vertical + _text([(72, 100, 'horizontal text too')])),
        'hidden': _pdf(b'/OC /L BDC ' + _text([(72, 700, 'hidden line')]) + b' EMC ' + _text([(72, 688, 'visible')])),
        'invisible': _pdf(b'3 Tr ' + _text([(72, 700, 'invisible OCR text'), (72, 688, 'more of it')])),
        'form': _pdf(b'q 1 0 0 1 72 600 cm /Fm0 Do Q ' + _text([(72, 700, 'page text')])),
        'whitespace': _pdf(_text([(72, 700, '     '), (72, 688, 'text'), (72, 676, '\t \t')])),
        'offpage': _pdf(_text([(-50, 700, 'left of page'), (72, -30, 'below page'), (72, 700, 'on page')])),
        'ties': _pdf(_text([(100, 100, 'a'), (300, 100, 'b'), (100, 300, 'c'), (300, 300, 'd'), (200, 200, 'e')])),
    }


def _run(name: str, doc: pawdf.Document, pages: list[int] | range) -> tuple[int, int]:
    mismatched = skipped = 0
    for i in pages:
        page = doc[i]
        content = page.content()
        for settings in SETTINGS:
            ours, limited = pawdf_layout(page, settings)
            if limited:
                skipped += 1
                print(f'{name} page {i} {settings}: skipped (layout_limit)')
                continue
            diffs = compare_tree(ours, pdfminer_layout(page, settings, content))
            if diffs:
                mismatched += 1
                print(f'{name} page {i} {settings}: {len(diffs)} mismatches')
                for d in diffs[:SHOW]:
                    print(f'  {d}')
    print(f'{name}: {len(pages)} pages x {len(SETTINGS)} settings, {mismatched} mismatched, {skipped} skipped')
    return mismatched, skipped


def main() -> int:
    """Compare the fixtures and every given PDF, and print a report."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('pdf', type=Path, nargs='*')
    parser.add_argument('--pages', help='comma-separated 0-based page indices, for the given PDFs')
    args = parser.parse_args()

    bad = 0
    for name, data in fixtures().items():
        doc = pawdf.Document(data)
        bad += sum(_run(f'fixture {name}', doc, range(len(doc))))
    for path in args.pdf:
        doc = pawdf.open(path)
        pages = [int(p) for p in args.pages.split(',')] if args.pages else range(len(doc))
        bad += sum(_run(str(path), doc, pages))
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
