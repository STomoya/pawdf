"""Compare pawdf's extraction of a PDF against pdfium (via pypdfium2).

Run: uv run --group validation python python/validation/compare_pdfium.py PDF [--pages 0,1]

Per page, it compares:

- Glyphs. pawdf glyphs are expanded to one (char, origin) pair per character of their
  text. pdfium characters come from its text page, skipping generated ones
  (FPDFText_IsGenerated). The two strings are aligned with difflib; unaligned runs are
  Unicode mismatches, and aligned characters whose origins differ by more than 0.5 pt in
  x or y are position mismatches.
- Paths. pdfium page objects are walked in order, descending into forms. FPDFPath_CountSegments
  counts points, so a pawdf path counts MoveTo and LineTo as 1, CurveTo as 3, and Close as
  1 only when the current point differs from the subpath start (pdfium's Handle_ClosePath
  adds a closing line point in that case, and only sets a flag otherwise).
- Images. Pixel sizes of image objects, in order.

Documented pdfium policies the comparison accounts for (core/fpdftext/cpdf_textpage.cpp):

- CloseTempLine deletes a space that directly follows another space in a line, so runs
  of real spaces in pawdf's output are collapsed before comparing.
- A hyphen that ends a line is reported as U+0002 (ProcessInsertObject's kHyphen case);
  it is compared as '-'.
- A code without Unicode is reported as the code itself (CharType::kNotUnicode), where
  pawdf reports U+FFFD; such a glyph is compared as chr(code), so it matches only that.
- ProcessTextObject reorders text objects on one line by x. Characters that difflib
  cannot align are therefore re-paired by character and origin before being reported.
- ProcessTextObject skips a text object whose bounds are narrower than kSizeEpsilon (0.01),
  such as one showing a lone space or a bitmap emoji, whose glyphs have no outline. Such
  objects are found through FPDFPageObj_GetBounds, and pawdf characters at their origins
  are not reported.

Exit status is 1 if any page mismatches.
"""

from __future__ import annotations

import argparse
import ctypes
import difflib
import sys
from dataclasses import dataclass, field
from pathlib import Path

import pypdfium2 as pdfium
import pypdfium2.raw as pdfium_c

import pawdf

TOLERANCE = 0.5
SHOW = 20
MAX_FORM_DEPTH = 32
SIZE_EPSILON = 0.01
HYPHEN_MARK = 0x2
UNMAPPED = '\ufffd'


@dataclass
class PageReport:
    """Mismatches found on one page."""

    index: int
    glyphs: int = 0
    unicode: list[str] = field(default_factory=list)
    position: list[str] = field(default_factory=list)
    paths: list[str] = field(default_factory=list)
    images: list[str] = field(default_factory=list)

    @property
    def ok(self) -> bool:
        """Whether nothing mismatched."""
        return not (self.unicode or self.position or self.paths or self.images)


def pawdf_chars(content: pawdf.PageContent) -> list[tuple[str, float, float]]:
    """One (char, x, y) per character of each glyph's text, with space runs collapsed."""
    out: list[tuple[str, float, float]] = []
    for g in content.glyphs:
        text = chr(g.code) if g.text == UNMAPPED and g.code else g.text
        for ch in text:
            if ch == ' ' and out and out[-1][0] == ' ':
                continue
            out.append((ch, *g.origin))
    return out


def pdfium_chars(page: pdfium.PdfPage) -> list[tuple[str, float, float]]:
    """Non-generated characters of pdfium's text page with their origins."""
    textpage = page.get_textpage()
    out = []
    x, y = ctypes.c_double(), ctypes.c_double()
    try:
        for i in range(pdfium_c.FPDFText_CountChars(textpage.raw)):
            if pdfium_c.FPDFText_IsGenerated(textpage.raw, i) == 1:
                continue
            code = pdfium_c.FPDFText_GetUnicode(textpage.raw, i)
            pdfium_c.FPDFText_GetCharOrigin(textpage.raw, i, ctypes.byref(x), ctypes.byref(y))
            out.append(('-' if code == HYPHEN_MARK else chr(code), x.value, y.value))
    finally:
        textpage.close()
    return out


def pdfium_objects(page: pdfium.PdfPage) -> tuple[list[int], list[tuple[int, int]], list[tuple[float, float]]]:
    """Segment counts of path objects and pixel sizes of image objects, in content order,
    and the origins of the text objects the text page skips for having no width.
    """
    segments, sizes, skipped = [], [], []
    w, h = ctypes.c_uint(), ctypes.c_uint()
    bounds = [ctypes.c_float() for _ in range(4)]
    for obj in page.get_objects(max_depth=MAX_FORM_DEPTH):
        if obj.type == pdfium_c.FPDF_PAGEOBJ_TEXT:
            pdfium_c.FPDFPageObj_GetBounds(obj.raw, *map(ctypes.byref, bounds))
            left, bottom, right, _ = (b.value for b in bounds)
            if abs(right - left) < SIZE_EPSILON:
                skipped.append((left, bottom))
        elif obj.type == pdfium_c.FPDF_PAGEOBJ_PATH:
            segments.append(pdfium_c.FPDFPath_CountSegments(obj.raw))
        elif obj.type == pdfium_c.FPDF_PAGEOBJ_IMAGE:
            pdfium_c.FPDFImageObj_GetImagePixelSize(obj.raw, ctypes.byref(w), ctypes.byref(h))
            sizes.append((w.value, h.value))
    return segments, sizes, skipped


def pdfium_point_count(path: pawdf.Path) -> int:
    """The number of points pdfium stores for this path."""
    count = 0
    start = current = None
    for op, pts in path.segments:
        if op == 'm':
            start = current = pts
            count += 1
        elif op == 'l':
            current = pts
            count += 1
        elif op == 'c':
            current = pts[4:]
            count += 3
        else:
            count += current != start
            current = start
    return count


def near(a: tuple[str, float, float], b: tuple[str, float, float]) -> bool:
    """Whether two characters' origins agree within the tolerance."""
    return abs(a[1] - b[1]) <= TOLERANCE and abs(a[2] - b[2]) <= TOLERANCE


def compare_glyphs(report: PageReport, ours: list, theirs: list, skipped: list[tuple[float, float]]) -> None:
    """Align the two character sequences and record Unicode and position mismatches."""
    a = ''.join(c for c, _, _ in ours)
    b = ''.join(c for c, _, _ in theirs)
    matcher = difflib.SequenceMatcher(None, a, b, autojunk=False)
    left, right = [], []
    for tag, i1, i2, j1, j2 in matcher.get_opcodes():
        if tag != 'equal':
            left.extend(ours[i1:i2])
            right.extend(theirs[j1:j2])
            continue
        for k in range(i2 - i1):
            ch, x0, y0 = ours[i1 + k]
            _, x1, y1 = theirs[j1 + k]
            if abs(x0 - x1) > TOLERANCE or abs(y0 - y1) > TOLERANCE:
                report.position.append(f'{ch!r} ours=({x0:.2f}, {y0:.2f}) pdfium=({x1:.2f}, {y1:.2f})')
    for o in left:
        match = next(
            (t for t in right if t[0] == o[0] and near(o, t)),
            None,
        )
        if match is None and any(near(o, ('', x, y)) for x, y in skipped):
            continue
        if match is None:
            report.unicode.append(f'ours {o[0]!r} at ({o[1]:.2f}, {o[2]:.2f}) not in pdfium')
        else:
            right.remove(match)
    report.unicode.extend(f'pdfium {t[0]!r} at ({t[1]:.2f}, {t[2]:.2f}) not in ours' for t in right)


def compare_page(index: int, ours: pawdf.Page, theirs: pdfium.PdfPage) -> PageReport:
    """Compare one page."""
    content = ours.content()
    report = PageReport(index, glyphs=len(content.glyphs))
    segments, sizes, skipped = pdfium_objects(theirs)
    compare_glyphs(report, pawdf_chars(content), pdfium_chars(theirs), skipped)
    counts = [pdfium_point_count(p) for p in content.paths]
    if counts != segments:
        matcher = difflib.SequenceMatcher(None, counts, segments, autojunk=False)
        for tag, i1, i2, j1, j2 in matcher.get_opcodes():
            if tag != 'equal':
                report.paths.append(f'{tag} ours[{i1}:{i2}]={counts[i1:i2]} pdfium[{j1}:{j2}]={segments[j1:j2]}')
    image_sizes = [(i.width, i.height) for i in content.images]
    if image_sizes != sizes:
        report.images.append(f'ours={image_sizes} pdfium={sizes}')
    return report


def main() -> int:
    """Compare every requested page and print a report."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('pdf', type=Path)
    parser.add_argument('--pages', help='comma-separated 0-based page indices')
    args = parser.parse_args()

    ours = pawdf.open(args.pdf)
    theirs = pdfium.PdfDocument(args.pdf)
    try:
        pages = [int(p) for p in args.pages.split(',')] if args.pages else range(len(ours))
        failed = False
        for i in pages:
            page = theirs[i]
            try:
                report = compare_page(i, ours[i], page)
            finally:
                page.close()
            failed |= not report.ok
            print(
                f'page {i}: glyphs {report.glyphs} unicode {len(report.unicode)} position {len(report.position)} '
                f'paths {len(report.paths)} images {len(report.images)}'
            )
            for name in ('unicode', 'position', 'paths', 'images'):
                for line in getattr(report, name)[:SHOW]:
                    print(f'  {name}: {line}')
    finally:
        theirs.close()
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main())
