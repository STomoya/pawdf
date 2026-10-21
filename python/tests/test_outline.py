"""Tests for the outline binding."""

from __future__ import annotations

import pawdf


def _pdf(outline: tuple[bytes, ...] = ()) -> bytes:
    """Build a one-page PDF; `outline` objects start at 4 and the root, if any, is object 4."""
    catalog = b'<< /Type /Catalog /Pages 2 0 R' + (b' /Outlines 4 0 R' if outline else b'') + b' >>'
    objects = [
        catalog,
        b'<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
        b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>',
        *outline,
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


def test_outline() -> None:
    """Items expose titles, destinations, and children."""
    o = pawdf.Document(
        _pdf(
            (
                b'<< /First 5 0 R >>',
                b'<< /Title (A) /Dest [3 0 R /XYZ null 700 0] /First 7 0 R /Next 6 0 R >>',
                b'<< /Title (B) /A << /S /URI /URI (https://example.com) >> >>',
                b'<< /Title (A1) >>',
            )
        )
    ).outline()
    assert [i.title for i in o.items] == ['A', 'B']
    a = o.items[0]
    assert a.dest is not None
    assert (a.dest.page, a.dest.left, a.dest.top) == (0, None, 700.0)
    assert a.children[0].title == 'A1'
    assert a.children[0].dest is None
    assert o.items[1].dest is None
    assert o.diagnostics == []
    assert repr(a.dest) == 'Destination(page=0, left=None, top=700.0)'
    assert repr(a) == "OutlineItem(title='A', dest=Destination(page=0, left=None, top=700.0), children=1)"


def test_outline_diagnostics_and_reprs() -> None:
    """A broken item is reported as bad_outline."""
    o = pawdf.Document(_pdf((b'<< /First 5 0 R >>', b'<< >>'))).outline()
    assert o.diagnostics[0].kind == 'bad_outline'
    assert o.diagnostics[0].object == (5, 0)
    assert repr(o) == 'Outline(items=1, diagnostics=1)'
    assert repr(o.items[0]) == "OutlineItem(title='', dest=None, children=0)"


def test_no_outline() -> None:
    """A document without an outline gives an empty one."""
    o = pawdf.Document(_pdf()).outline()
    assert o.items == []
    assert o.diagnostics == []
