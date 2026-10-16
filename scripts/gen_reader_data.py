"""Regenerate the bundled data in crates/pawdf_reader/data from upstream sources.

Run with: uv run --no-project python scripts/gen_reader_data.py
"""

from __future__ import annotations

import base64
import re
import urllib.request
import zlib
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / 'crates' / 'pawdf_reader' / 'data'

PDFIUM = 'https://pdfium.googlesource.com/pdfium/+/refs/heads/main/'
AGL = 'https://raw.githubusercontent.com/adobe-type-tools/agl-aglfn/master/'
AFM = 'https://raw.githubusercontent.com/apache/pdfbox/trunk/pdfbox/src/main/resources/org/apache/pdfbox/resources/afm/'

# pdfium array name -> name used in encodings.txt.
ENCODINGS = {
    'kStandardEncodingNames': 'StandardEncoding',
    'kAdobeWinAnsiEncodingNames': 'WinAnsiEncoding',
    'kMacRomanEncodingNames': 'MacRomanEncoding',
    'kMacExpertEncodingNames': 'MacExpertEncoding',
    'kAdobeSymbolEncodingNames': 'Symbol',
    'kZapfEncodingNames': 'ZapfDingbats',
}
FIRST_CODE = 32

STD14 = [
    'Courier',
    'Courier-Bold',
    'Courier-BoldOblique',
    'Courier-Oblique',
    'Helvetica',
    'Helvetica-Bold',
    'Helvetica-BoldOblique',
    'Helvetica-Oblique',
    'Times-Roman',
    'Times-Bold',
    'Times-BoldItalic',
    'Times-Italic',
    'Symbol',
    'ZapfDingbats',
]


def fetch(url: str) -> str:
    """Download a URL as UTF-8 text."""
    with urllib.request.urlopen(url) as r:
        return r.read().decode('utf-8')


def fetch_gitiles(path: str) -> str:
    """Download a file from pdfium's gitiles, which serves base64 with ?format=TEXT."""
    return base64.b64decode(fetch(f'{PDFIUM}{path}?format=TEXT')).decode('utf-8')


def write_deflated(name: str, text: str) -> None:
    """Write zlib-compressed UTF-8 text to OUT/name."""
    (OUT / name).write_bytes(zlib.compress(text.encode('utf-8'), 9))


def encodings() -> str:
    """One line per base encoding: its name, then glyph names for codes 32..=255."""
    src = fetch_gitiles('core/fpdfapi/font/cpdf_fontencoding.cpp')
    lines = []
    for array, name in ENCODINGS.items():
        m = re.search(array + r'\s*=\s*\{\{(.*?)\}\};', src, re.DOTALL)
        if m is None:
            msg = f'{array} not found in pdfium source'
            raise RuntimeError(msg)
        names = [g or '-' for g in re.findall(r'"([^"]*)"|nullptr', m.group(1))]
        if len(names) != 256 - FIRST_CODE:
            msg = f'{array}: expected {256 - FIRST_CODE} entries, got {len(names)}'
            raise RuntimeError(msg)
        lines.append(' '.join([name, *names]))
    return '\n'.join(lines) + '\n'


def glyph_list() -> str:
    """AGL glyphlist.txt plus zapfdingbats.txt, without comments."""
    out = []
    for f in ('glyphlist.txt', 'zapfdingbats.txt'):
        out.extend(line for line in fetch(AGL + f).splitlines() if line and not line.startswith('#'))
    return '\n'.join(out) + '\n'


def std14() -> tuple[str, str]:
    """Glyph widths of the 14 standard fonts, and their copyright notices."""
    widths, notices = [], []
    for font in STD14:
        afm = fetch(f'{AFM}{font}.afm')
        widths.append(f'#{font}')
        for line in afm.splitlines():
            m = re.match(r'C\s+-?\d+\s*;\s*WX\s+(\S+)\s*;\s*N\s+(\S+)\s*;', line)
            if m:
                widths.append(f'{m.group(2)} {m.group(1)}')
            elif line.startswith(('Notice', 'Comment Copyright')):
                notices.append(f'{font}: {line}')
    return '\n'.join(widths) + '\n', '\n'.join(notices) + '\n'


def main() -> None:
    """Fetch every source and rewrite the data directory."""
    OUT.mkdir(parents=True, exist_ok=True)
    write_deflated('encodings.txt.zz', encodings())
    write_deflated('glyphlist.txt.zz', glyph_list())
    widths, notices = std14()
    write_deflated('std14.txt.zz', widths)
    header = (
        'Glyph widths in std14.txt.zz are extracted from the Adobe Core 14 AFM files\n'
        '(mirrored in Apache PDFBox). Each AFM file carries this notice:\n\n'
    )
    (OUT / 'NOTICE-afm.txt').write_text(header + notices)
    (OUT / 'LICENSE-agl.md').write_text(fetch(AGL + 'LICENSE.md'))
    (OUT / 'LICENSE-pdfium').write_text(fetch_gitiles('LICENSE'))


if __name__ == '__main__':
    main()
