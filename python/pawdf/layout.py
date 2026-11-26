"""Layout analysis: a page's glyphs grouped into blocks, lines and words, in reading order.

Each algorithm is a function named after it. ``pdfminer`` ports pdfminer.six's layout analysis.
``text_in_regions`` applies it to caller-supplied boxes, such as table cells from a detector.
``tables_tagged`` reads the tables of a tagged PDF from its structure tree; ``tables_ruled`` finds
tables drawn with ruling lines.
"""

from __future__ import annotations

from pawdf._core import Block, Cell, Layout, Line, Table, Tables, Word
from pawdf._core import layout_pdfminer as pdfminer
from pawdf._core import layout_tables_ruled as tables_ruled
from pawdf._core import layout_tables_tagged as tables_tagged
from pawdf._core import layout_text_in_regions as text_in_regions

__all__ = [
    'Block',
    'Cell',
    'Layout',
    'Line',
    'Table',
    'Tables',
    'Word',
    'pdfminer',
    'tables_ruled',
    'tables_tagged',
    'text_in_regions',
]
