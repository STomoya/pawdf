"""Tests for the form field binding."""

from __future__ import annotations

import pawdf


def _pdf(catalog_extra: bytes = b'') -> bytes:
    """Build a one-page PDF whose catalog has `catalog_extra` and whose page lists object 4 in `/Annots`."""
    objects = [
        b'<< /Type /Catalog /Pages 2 0 R ' + catalog_extra + b' >>',
        b'<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
        b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [4 0 R] >>',
        b'<< /T (a) /TU (Label) /FT /Tx /V (x) /Subtype /Widget /Rect [1 2 3 4] >>',
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


def test_form_fields() -> None:
    """Form fields expose their name, kind, values and widgets."""
    f = pawdf.Document(_pdf(b'/AcroForm << /Fields [4 0 R] >>')).form_fields()
    assert f.diagnostics == []
    (a,) = f.items
    assert (a.name, a.label, a.kind, a.values, a.flags) == ('a', 'Label', 'text', ['x'], 0)
    (w,) = a.widgets
    assert (w.page, w.rect) == (0, (1.0, 2.0, 3.0, 4.0))
    assert repr(a) == "FormField(name='a', kind='text', values=['x'])"
    assert repr(w) == 'Widget(page=0, rect=(1.0, 2.0, 3.0, 4.0))'
    assert repr(f) == 'FormFields(items=1, diagnostics=0)'


def test_form_field_diagnostics() -> None:
    """A malformed form is reported as bad_form_field."""
    f = pawdf.Document(_pdf(b'/AcroForm 5')).form_fields()
    assert f.items == []
    assert [d.kind for d in f.diagnostics] == ['bad_form_field']


def test_no_form() -> None:
    """A document without a form has no fields."""
    f = pawdf.Document(_pdf()).form_fields()
    assert (f.items, f.diagnostics) == ([], [])
