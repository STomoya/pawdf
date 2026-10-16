# pawdf

## Third-party data

`crates/pawdf_reader/data/` bundles data derived from these sources. Regenerate it with
`uv run --no-project python scripts/gen_reader_data.py`.

| File | Source | License |
|---|---|---|
| `glyphlist.txt.zz` | Adobe Glyph List and ITC Zapf Dingbats glyph list (`adobe-type-tools/agl-aglfn`) | BSD-3-Clause, `LICENSE-agl.md` |
| `encodings.txt.zz` | Base encoding glyph-name tables from PDFium (`core/fpdfapi/font/cpdf_fontencoding.cpp`) | BSD-3-Clause, `LICENSE-pdfium` |
| `std14.txt.zz` | Glyph widths from the Adobe Core 14 AFM files (via Apache PDFBox) | Adobe AFM notice, `NOTICE-afm.txt` |
| `cmaps.bin` | Predefined CJK CMaps (`adobe-type-tools/cmap-resources`) and `Adobe-*-UCS2` CID-to-Unicode tables (`adobe-type-tools/mapping-resources-pdf`), unmodified, each entry zlib-compressed | BSD-3-Clause, `LICENSE-cmap-resources.md`, `LICENSE-mapping-resources-pdf.txt` |
