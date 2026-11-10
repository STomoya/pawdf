"""Layout analysis: a page's glyphs grouped into blocks, lines and words, in reading order.

Each algorithm is a function named after it. ``pdfminer`` ports pdfminer.six's layout analysis.
"""

from __future__ import annotations

from pawdf._core import Block, Layout, Line, Word
from pawdf._core import layout_pdfminer as pdfminer

__all__ = ['Block', 'Layout', 'Line', 'Word', 'pdfminer']
