"""Regenerate the bundled data in crates/pawdf_reader/data from upstream sources.

Run with: uv run --no-project python scripts/gen_reader_data.py
"""

from __future__ import annotations

import base64
import io
import re
import tarfile
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

CMAP_RESOURCES = (
    'https://codeload.github.com/adobe-type-tools/cmap-resources/tar.gz/f5cf3bca7fdfeaceb77aa82847e974f2306c20b4'
)
MAPPING_RESOURCES = (
    'https://codeload.github.com/adobe-type-tools/mapping-resources-pdf/tar.gz/2dd5e53fb74a01718b9dfd448a0d1cce6fff2aa5'
)
# ISO 32000-2 Table 116, without Identity-H and Identity-V, which the reader builds in code.
PREDEFINED_CMAPS = [
    '83pv-RKSJ-H',
    '90ms-RKSJ-H',
    '90ms-RKSJ-V',
    '90msp-RKSJ-H',
    '90msp-RKSJ-V',
    '90pv-RKSJ-H',
    'Add-RKSJ-H',
    'Add-RKSJ-V',
    'B5pc-H',
    'B5pc-V',
    'CNS-EUC-H',
    'CNS-EUC-V',
    'ETen-B5-H',
    'ETen-B5-V',
    'ETenms-B5-H',
    'ETenms-B5-V',
    'EUC-H',
    'EUC-V',
    'Ext-RKSJ-H',
    'Ext-RKSJ-V',
    'GB-EUC-H',
    'GB-EUC-V',
    'GBK-EUC-H',
    'GBK-EUC-V',
    'GBK2K-H',
    'GBK2K-V',
    'GBKp-EUC-H',
    'GBKp-EUC-V',
    'GBpc-EUC-H',
    'GBpc-EUC-V',
    'H',
    'HKscs-B5-H',
    'HKscs-B5-V',
    'KSC-EUC-H',
    'KSC-EUC-V',
    'KSCms-UHC-H',
    'KSCms-UHC-HW-H',
    'KSCms-UHC-HW-V',
    'KSCms-UHC-V',
    'KSCpc-EUC-H',
    'UniCNS-UCS2-H',
    'UniCNS-UCS2-V',
    'UniCNS-UTF16-H',
    'UniCNS-UTF16-V',
    'UniGB-UCS2-H',
    'UniGB-UCS2-V',
    'UniGB-UTF16-H',
    'UniGB-UTF16-V',
    'UniJIS-UCS2-H',
    'UniJIS-UCS2-HW-H',
    'UniJIS-UCS2-HW-V',
    'UniJIS-UCS2-V',
    'UniJIS-UTF16-H',
    'UniJIS-UTF16-V',
    'UniKS-UCS2-H',
    'UniKS-UCS2-V',
    'UniKS-UTF16-H',
    'UniKS-UTF16-V',
    'V',
]
UCS2_ORDERINGS = ['Japan1', 'GB1', 'CNS1', 'Korea1', 'KR']

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


def fetch_tar(url: str) -> dict[str, bytes]:
    """Download a .tar.gz and return its regular files by path without the top directory."""
    with urllib.request.urlopen(url) as r, tarfile.open(fileobj=io.BytesIO(r.read()), mode='r:gz') as tar:
        out = {}
        for m in tar.getmembers():
            f = tar.extractfile(m) if m.isfile() else None
            if f is not None:
                out[m.name.split('/', 1)[1]] = f.read()
        return out


def cmaps() -> tuple[bytes, bytes, bytes]:
    """The CMap archive, then the cmap-resources and mapping-resources-pdf licenses.

    The archive is an ASCII index of `name offset length` lines, a blank line, then each
    file zlib-compressed on its own, offsets counted from the end of the index.
    """
    cmap_src = fetch_tar(CMAP_RESOURCES)
    mapping_src = fetch_tar(MAPPING_RESOURCES)
    by_name = {p.rsplit('/', 1)[1]: data for p, data in cmap_src.items() if re.match(r'Adobe-[^/]+/CMap/', p)}
    files: dict[str, bytes] = {}
    todo = list(PREDEFINED_CMAPS)
    while todo:
        name = todo.pop()
        if name in files:
            continue
        files[name] = by_name[name]
        todo += [m.decode() for m in re.findall(rb'/([\w-]+)\s+usecmap', files[name])]
    for ordering in UCS2_ORDERINGS:
        name = f'Adobe-{ordering}-UCS2'
        files[name] = mapping_src[f'pdf2unicode/{name}']
    index, blobs, offset = [], [], 0
    for name in sorted(files):
        blob = zlib.compress(files[name], 9)
        index.append(f'{name} {offset} {len(blob)}\n')
        blobs.append(blob)
        offset += len(blob)
    archive = (''.join(index) + '\n').encode('ascii') + b''.join(blobs)
    return archive, cmap_src['LICENSE.md'], mapping_src['LICENSE.txt']


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
    archive, cmap_license, mapping_license = cmaps()
    (OUT / 'cmaps.bin').write_bytes(archive)
    (OUT / 'LICENSE-cmap-resources.md').write_bytes(cmap_license)
    (OUT / 'LICENSE-mapping-resources-pdf.txt').write_bytes(mapping_license)


if __name__ == '__main__':
    main()
