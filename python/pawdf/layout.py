"""Layout analysis: a page's glyphs grouped into blocks, lines and words, in reading order.

Each algorithm is a function named after it. ``pdfminer`` ports pdfminer.six's layout analysis.
``text_in_regions`` applies it to caller-supplied boxes, such as table cells from a detector.
"""

from __future__ import annotations

from pawdf._core import Block, Layout, Line, Word
from pawdf._core import layout_pdfminer as pdfminer
from pawdf._core import layout_text_in_regions as text_in_regions

__all__ = ['Block', 'Layout', 'Line', 'Word', 'pdfminer', 'text_in_regions']
