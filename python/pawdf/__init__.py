"""Read PDF files: text with positions, vector paths, and images."""

from __future__ import annotations

from pathlib import Path as _Path
from typing import TYPE_CHECKING

from pawdf._core import (
    Color,
    Diagnostic,
    Document,
    EncryptedError,
    FontInfo,
    Glyph,
    Image,
    MarkedContent,
    Page,
    PageContent,
    Path,
    PdfError,
    SoftMask,
    hello_from_bin,
)
from pawdf._version import __version__

if TYPE_CHECKING:
    import os

__all__ = [
    'Color',
    'Diagnostic',
    'Document',
    'EncryptedError',
    'FontInfo',
    'Glyph',
    'Image',
    'MarkedContent',
    'Page',
    'PageContent',
    'Path',
    'PdfError',
    'SoftMask',
    '__version__',
    'open',
]


def open(path: str | os.PathLike[str]) -> Document:  # noqa: A001 -- mirrors the builtin by design.
    """Read the PDF file at `path` and open it as a Document.

    Args:
        path: Path of the PDF file.

    Returns:
        The opened document.

    Raises:
        PdfError: If the file is not a readable PDF.
        EncryptedError: If the file is encrypted.
    """
    return Document(_Path(path).read_bytes())


def hello() -> str:
    return hello_from_bin()
