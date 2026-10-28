"""Tests for the annotation binding."""

from __future__ import annotations

import pawdf


def _pdf(annots: bytes = b'') -> bytes:
    """Build a one-page PDF whose page has `annots` as its `/Annots` value."""
    page = b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792]' + (b' /Annots ' + annots if annots else b'') + b' >>'
    objects = [b'<< /Type /Catalog /Pages 2 0 R >>', b'<< /Type /Pages /Kids [3 0 R] /Count 1 >>', page]
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


def test_annotations() -> None:
    """Annotations expose their entries and link targets."""
    a = pawdf.Document(
        _pdf(
            b'[<< /Subtype /Link /Rect [10 20 30 40] /Dest [3 0 R /XYZ 5 6 0] >>'
            b' << /Subtype /Link /Rect [0 0 1 1] /A << /S /URI /URI (https://example.com) >> >>'
            b' << /Subtype /Highlight /Rect [0 0 9 9] /Contents (note) /T (me) /NM (h1) /F 4'
            b' /QuadPoints [1 2 3 4 5 6 7 8] >>]'
        )
    )[0].annotations()
    assert a.diagnostics == []
    link, uri, hl = a.items
    assert link.subtype == 'Link'
    assert link.rect == (10.0, 20.0, 30.0, 40.0)
    assert link.dest is not None
    assert (link.dest.page, link.dest.left, link.dest.top) == (0, 5.0, 6.0)
    assert link.uri is None
    assert uri.uri == 'https://example.com'
    assert uri.dest is None
    assert (hl.contents, hl.author, hl.name, hl.flags) == ('note', 'me', 'h1', 4)
    assert hl.quad_points == [((1.0, 2.0), (3.0, 4.0), (5.0, 6.0), (7.0, 8.0))]
    assert repr(link) == "Annotation(subtype='Link', rect=(10.0, 20.0, 30.0, 40.0))"
    assert repr(a) == 'Annotations(items=3, diagnostics=0)'


def test_annotation_diagnostics() -> None:
    """A malformed annotation is kept and reported as bad_annotation."""
    a = pawdf.Document(_pdf(b'[<< >>]'))[0].annotations()
    assert a.items[0].subtype == ''
    assert a.items[0].rect == (0.0, 0.0, 0.0, 0.0)
    assert [d.kind for d in a.diagnostics] == ['bad_annotation', 'bad_annotation']
    assert a.diagnostics[0].object == (3, 0)


def test_no_annotations() -> None:
    """A page without annotations gives none."""
    a = pawdf.Document(_pdf())[0].annotations()
    assert a.items == []
    assert a.diagnostics == []


def test_annotation_hidden() -> None:
    """An annotation whose /OC is not a known group stays visible and is reported."""
    a = pawdf.Document(_pdf(b'[<< /Subtype /Text /Rect [0 0 1 1] /OC << /Type /OCG >> >>]'))[0].annotations()
    assert not a.items[0].hidden
    assert [d.kind for d in a.diagnostics] == ['bad_optional_content']
