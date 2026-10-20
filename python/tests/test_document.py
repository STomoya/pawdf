"""Tests for the document and page-content binding."""

from __future__ import annotations

from typing import TYPE_CHECKING

import pytest

import pawdf

if TYPE_CHECKING:
    from pathlib import Path

HELV = b'<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>'


def _stream(data: bytes) -> bytes:
    return b'<< /Length %d >>\nstream\n' % len(data) + data + b'\nendstream'


def _pdf(
    content: bytes, resources: bytes = b'<< >>', trailer_extra: bytes = b'', extra: tuple[bytes, ...] = ()
) -> bytes:
    """Build a one-page PDF with correct xref offsets; object 5 is Helvetica, `extra` follows from 6."""
    objects = [
        b'<< /Type /Catalog /Pages 2 0 R >>',
        b'<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
        b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources ' + resources + b' >>',
        _stream(content),
        HELV,
        *extra,
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
    out += b'trailer\n<< /Size %d /Root 1 0 R %s >>\nstartxref\n%d\n%%%%EOF\n' % (len(objects) + 1, trailer_extra, xref)
    return bytes(out)


FONT_RES = b'<< /Font << /F1 5 0 R >> >>'


def test_document_basics() -> None:
    """Pages are indexable, iterable, and expose their boxes."""
    doc = pawdf.Document(_pdf(b''))
    assert len(doc) == 1
    page = doc[0]
    assert page.media_box == (0.0, 0.0, 612.0, 792.0)
    assert page.crop_box == page.media_box
    assert page.rotate == 0
    assert page.display_matrix == (1.0, 0.0, 0.0, -1.0, 0.0, 792.0)
    assert [p.index for p in doc] == [0]
    assert doc[-1].index == 0
    assert doc.diagnostics == []


def test_index_error() -> None:
    """An out-of-range page index raises IndexError."""
    doc = pawdf.Document(_pdf(b''))
    with pytest.raises(IndexError):
        doc[1]
    with pytest.raises(IndexError):
        doc[-2]
    with pytest.raises(IndexError):
        doc[2**70]
    with pytest.raises(IndexError):
        doc[-(2**70)]


def test_open_reads_path(tmp_path: Path) -> None:
    """pawdf.open reads a file from disk."""
    p = tmp_path / 'a.pdf'
    p.write_bytes(_pdf(b''))
    assert len(pawdf.open(p)) == 1
    assert len(pawdf.open(str(p))) == 1


def test_errors() -> None:
    """Unsupported encryption raises an EncryptedError, a PdfError subclass; garbage raises PdfError."""
    with pytest.raises(pawdf.UnsupportedEncryptionError, match='missing /Filter') as e:
        pawdf.Document(_pdf(b'', trailer_extra=b'/Encrypt << >>'))
    assert isinstance(e.value, pawdf.EncryptedError)
    assert isinstance(e.value, pawdf.PdfError)
    with pytest.raises(pawdf.PdfError):
        pawdf.Document(b'not a pdf')


def test_glyphs() -> None:
    """Glyphs carry text, geometry, and their font."""
    c = pawdf.Document(_pdf(b'BT /F1 10 Tf 100 200 Td (ab) Tj ET', FONT_RES))[0].content()
    assert [g.text for g in c.glyphs] == ['a', 'b']
    g = c.glyphs[0]
    assert g.origin == (100.0, 200.0)
    assert g.matrix == (10.0, 0.0, 0.0, 10.0, 100.0, 200.0)
    assert (g.code, g.font, g.size) == (97, 0, 10.0)
    assert g.wmode == 'horizontal'
    assert g.render_mode == 0
    assert g.fill.family == 'gray'
    assert g.fill.components == (0.0,)
    assert g.fill.pattern is None
    assert (g.fill_alpha, g.stroke_alpha) == (1.0, 1.0)
    assert g.bbox == pytest.approx((100.0, 198.0, 105.56, 208.0))
    assert g.advance == pytest.approx(5.56)
    f = c.fonts[0]
    assert (f.base_font, f.kind, f.embedded, f.wmode) == ('Helvetica', 'type1', False, 'horizontal')


def test_vertical_glyph() -> None:
    """A Type0 font with Identity-V yields vertical glyphs."""
    type0 = b'<< /Type /Font /Subtype /Type0 /BaseFont /Foo /Encoding /Identity-V /DescendantFonts [7 0 R] >>'
    cid = (
        b'<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Foo '
        b'/CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> >>'
    )
    c = pawdf.Document(_pdf(b'BT /F1 10 Tf <0001> Tj ET', b'<< /Font << /F1 6 0 R >> >>', extra=(type0, cid)))[
        0
    ].content()
    assert c.glyphs[0].wmode == 'vertical'
    assert (c.fonts[0].kind, c.fonts[0].wmode) == ('type0', 'vertical')


def test_paths() -> None:
    """Painted paths expose segments and paint state."""
    c = pawdf.Document(_pdf(b'1 0 0 rg 10 20 30 40 re f 0 0 m 1 2 3 4 5 6 c S'))[0].content()
    p = c.paths[0]
    assert p.segments[0] == ('m', (10.0, 20.0))
    assert p.segments[1] == ('l', (40.0, 20.0))
    assert p.segments[-1] == ('h', ())
    assert p.fill == 'nonzero'
    assert p.stroke is False
    assert p.fill_color.family == 'rgb'
    assert p.fill_color.components == (1.0, 0.0, 0.0)
    assert (p.line_width, p.ctm) == (1.0, (1.0, 0.0, 0.0, 1.0, 0.0, 0.0))
    q = c.paths[1]
    assert q.segments[1] == ('c', (1.0, 2.0, 3.0, 4.0, 5.0, 6.0))
    assert (q.fill, q.stroke) == (None, True)


def test_images() -> None:
    """Inline images expose dimensions and samples."""
    c = pawdf.Document(_pdf(b'BI /W 2 /H 2 /CS /G /BPC 8 ID \x00\x01\x02\x03 EI'))[0].content()
    i = c.images[0]
    assert (i.kind, i.data) == ('samples', b'\x00\x01\x02\x03')
    assert (i.width, i.height, i.bpc) == (2, 2, 8)
    assert i.color_space == 'gray'
    assert i.inline is True
    assert i.name is None
    assert (i.indexed, i.decode, i.smask, i.jbig2_globals, i.ccitt) == (None, None, None, None, None)
    assert i.image_mask is False
    assert i.bbox == (0.0, 0.0, 1.0, 1.0)


def test_diagnostics() -> None:
    """Diagnostics name their kind and where they happened."""
    c = pawdf.Document(_pdf(b'Q'))[0].content()
    d = c.diagnostics[0]
    assert (d.kind, d.offset, d.object) == ('unbalanced_restore', 0, (4, 0))


def test_reprs() -> None:
    """Every class has a readable repr."""
    doc = pawdf.Document(_pdf(b'BT /F1 10 Tf (a) Tj ET 0 0 m 1 1 l S Q', FONT_RES))
    c = doc[0].content()
    assert repr(doc).startswith('Document(')
    assert repr(doc[0]).startswith('Page(')
    assert repr(c).startswith('PageContent(')
    for obj in (c.glyphs[0], c.paths[0], c.fonts[0], c.diagnostics[0], c.glyphs[0].fill):
        assert repr(obj).startswith(type(obj).__name__ + '(')


def test_clip() -> None:
    """Glyphs and paths carry the clip bound in effect."""
    c = pawdf.Document(_pdf(b'0 0 10 10 re W n BT /F1 10 Tf (a) Tj ET 0 0 1 1 re f', FONT_RES))[0].content()
    assert c.glyphs[0].clip == c.paths[0].clip == (0.0, 0.0, 10.0, 10.0)
    c = pawdf.Document(_pdf(b'0 0 1 1 re f'))[0].content()
    assert c.paths[0].clip is None


def test_marked_contents() -> None:
    """Marked-content spans expose their properties and index ranges."""
    ops = b'/Span << /ActualText (fi) /MCID 2 >> BDC BT /F1 10 Tf (ab) Tj ET EMC'
    c = pawdf.Document(_pdf(ops, FONT_RES))[0].content()
    m = c.marked_contents[0]
    assert (m.tag, m.mcid, m.actual_text) == ('Span', 2, 'fi')
    assert m.glyphs == range(2)
    assert m.paths == range(0)
    assert m.images == range(0)
    assert m.artifact_type is None
    assert m.artifact_subtype is None
    assert repr(m).startswith('MarkedContent(')
    assert 'marked_contents=1' in repr(c)


def test_unbalanced_marked_content() -> None:
    """A stray EMC is reported."""
    c = pawdf.Document(_pdf(b'EMC'))[0].content()
    assert c.diagnostics[0].kind == 'unbalanced_marked_content'
