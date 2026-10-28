"""Tests for the structure tree binding."""

from __future__ import annotations

import pawdf


def _pdf(tree: tuple[bytes, ...] = ()) -> bytes:
    """Build a one-page PDF with one link annotation (object 4); `tree` objects start at 5, the root first."""
    catalog = b'<< /Type /Catalog /Pages 2 0 R' + (b' /StructTreeRoot 5 0 R' if tree else b'') + b' >>'
    objects = [
        catalog,
        b'<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
        b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [4 0 R] >>',
        b'<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] >>',
        *tree,
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


def test_structure_tree() -> None:
    """Elements expose their entries and kids of every kind."""
    t = pawdf.Document(
        _pdf(
            (
                b'<< /K 6 0 R /RoleMap << /Para /P >> >>',
                (
                    b'<< /S /Para /P 5 0 R /Pg 3 0 R /Alt (a) /T (t) /Lang (en) /ActualText (x) /E (e)'
                    b' /K [0 << /Type /MCR /MCID 1 /Stm 9 0 R >> << /Type /OBJR /Obj 4 0 R >> 7 0 R] >>'
                ),
                b'<< /S /Span /P 6 0 R >>',
            )
        )
    ).structure_tree()
    assert t.diagnostics == []
    (p,) = t.roots
    assert (p.role, p.standard, p.page) == ('Para', 'P', 0)
    assert (p.alt, p.title, p.lang, p.actual_text, p.expansion) == ('a', 't', 'en', 'x', 'e')
    mcid, mcr, objr, span = p.children
    assert isinstance(mcid, pawdf.MarkedContentRef)
    assert (mcid.page, mcid.mcid, mcid.stream) == (0, 0, None)
    assert isinstance(mcr, pawdf.MarkedContentRef)
    assert mcr.stream == (9, 0)
    assert isinstance(objr, pawdf.ObjectRef)
    assert (objr.page, objr.annotation) == (0, 0)
    assert isinstance(span, pawdf.StructElement)
    assert span.title is None
    assert repr(p) == "StructElement(role='Para', standard='P', page=0, children=4)"
    assert repr(mcr) == 'MarkedContentRef(page=0, mcid=1, stream=(9, 0))'
    assert repr(objr) == 'ObjectRef(page=0, annotation=0)'
    assert repr(t) == 'StructTree(roots=1, diagnostics=0)'


def test_structure_tree_diagnostics() -> None:
    """A broken element is reported as bad_struct_tree."""
    t = pawdf.Document(_pdf((b'<< /K 6 0 R >>', b'<< /P 5 0 R >>'))).structure_tree()
    assert [(d.kind, d.object) for d in t.diagnostics] == [('bad_struct_tree', (6, 0))]
    assert repr(t.roots[0]) == "StructElement(role='', standard='', page=None, children=0)"


def test_untagged() -> None:
    """A document without a structure tree gives an empty one."""
    t = pawdf.Document(_pdf()).structure_tree()
    assert t.roots == []
    assert t.diagnostics == []
