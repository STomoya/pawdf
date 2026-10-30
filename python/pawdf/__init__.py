"""Read PDF files: text with positions, vector paths, and images."""

from __future__ import annotations

from pathlib import Path as _Path
from typing import TYPE_CHECKING

from pawdf._core import (
    Annotation,
    Annotations,
    Color,
    Destination,
    Diagnostic,
    Document,
    EncryptedError,
    Encryption,
    FontInfo,
    FormField,
    FormFields,
    Glyph,
    Image,
    MarkedContent,
    MarkedContentRef,
    ObjectRef,
    Outline,
    OutlineItem,
    Page,
    PageContent,
    PasswordError,
    Path,
    PdfError,
    Permissions,
    SoftMask,
    StructElement,
    StructTree,
    UnsupportedEncryptionError,
    Widget,
)
from pawdf._version import __version__

if TYPE_CHECKING:
    import os

__all__ = [
    'Annotation',
    'Annotations',
    'Color',
    'Destination',
    'Diagnostic',
    'Document',
    'EncryptedError',
    'Encryption',
    'FontInfo',
    'FormField',
    'FormFields',
    'Glyph',
    'Image',
    'MarkedContent',
    'MarkedContentRef',
    'ObjectRef',
    'Outline',
    'OutlineItem',
    'Page',
    'PageContent',
    'PasswordError',
    'Path',
    'PdfError',
    'Permissions',
    'SoftMask',
    'StructElement',
    'StructTree',
    'UnsupportedEncryptionError',
    'Widget',
    '__version__',
    'open',
]


def open(path: str | os.PathLike[str], password: str | None = None) -> Document:  # noqa: A001 -- mirrors the builtin by design.
    """Read the PDF file at `path` and open it as a Document.

    Args:
        path: Path of the PDF file.
        password: Password of an encrypted file, tried as the user then the owner password.
            The empty user password is always tried too.

    Returns:
        The opened document.

    Raises:
        PdfError: If the file is not a readable PDF.
        PasswordError: If the file is encrypted and no password opened it.
        UnsupportedEncryptionError: If the file uses an unsupported security handler.
    """
    return Document(_Path(path).read_bytes(), password)
