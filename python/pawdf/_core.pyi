from typing import Literal, TypeAlias, final

Matrix: TypeAlias = tuple[float, float, float, float, float, float]
Rect: TypeAlias = tuple[float, float, float, float]
Point: TypeAlias = tuple[float, float]
ImageKind: TypeAlias = Literal['samples', 'jpeg', 'jpx', 'jbig2', 'ccitt']
SpaceFamily: TypeAlias = Literal['cal_gray', 'cal_rgb', 'lab', 'separation', 'device_n', 'indexed', 'unknown']

def hello_from_bin() -> str: ...

class PdfError(Exception):
    """The PDF could not be read."""

class EncryptedError(PdfError):
    """The PDF is encrypted; decryption is not supported."""

@final
class Diagnostic:
    """A recovery from malformed input."""

    @property
    def kind(self) -> str:
        """The kind of recovery, in snake_case (for example ``'xref_repaired'``)."""
    @property
    def offset(self) -> int:
        """Byte offset in the file, or in the decoded stream when ``object`` is set."""
    @property
    def object(self) -> tuple[int, int] | None:
        """``(number, generation)`` of the stream the offset points into."""

@final
class Color:
    """A color in its own color space; no conversion is done."""

    @property
    def family(self) -> Literal['gray', 'rgb', 'cmyk', 'pattern'] | SpaceFamily:
        """Color space family."""
    @property
    def components(self) -> tuple[float, ...]:
        """Components as given; empty for patterns."""
    @property
    def pattern(self) -> str | None:
        """Pattern resource name, for pattern colors."""

@final
class FontInfo:
    """A font as seen from one page."""

    @property
    def base_font(self) -> str:
        """``/BaseFont``, including any subset tag."""
    @property
    def kind(self) -> Literal['type1', 'mmtype1', 'truetype', 'type3', 'type0']:
        """Font type."""
    @property
    def embedded(self) -> bool:
        """Whether a font program is embedded."""
    @property
    def wmode(self) -> Literal['horizontal', 'vertical']:
        """Writing mode."""

@final
class Glyph:
    """One shown character code, in PDF user space."""

    @property
    def text(self) -> str:
        """Unicode text; several characters for a ligature, U+FFFD when unmappable."""
    @property
    def code(self) -> int:
        """Raw character code."""
    @property
    def font(self) -> int:
        """Index into ``PageContent.fonts``."""
    @property
    def size(self) -> float:
        """Font size."""
    @property
    def matrix(self) -> Matrix:
        """Text rendering matrix."""
    @property
    def origin(self) -> Point:
        """Glyph origin."""
    @property
    def advance(self) -> float:
        """Length of the displacement along the writing direction."""
    @property
    def bbox(self) -> Rect:
        """Axis-aligned glyph box ``(x0, y0, x1, y1)``."""
    @property
    def wmode(self) -> Literal['horizontal', 'vertical']:
        """Writing mode."""
    @property
    def render_mode(self) -> int:
        """Text rendering mode 0..7; 3 is invisible."""
    @property
    def fill(self) -> Color:
        """Fill color."""
    @property
    def stroke(self) -> Color:
        """Stroke color."""
    @property
    def fill_alpha(self) -> float:
        """Fill alpha."""
    @property
    def stroke_alpha(self) -> float:
        """Stroke alpha."""

@final
class Path:
    """A painted path, in PDF user space."""

    @property
    def segments(self) -> list[tuple[Literal['m', 'l', 'c', 'h'], tuple[float, ...]]]:
        """Segments: move, line, curve (two control points then the end point), close."""
    @property
    def stroke(self) -> bool:
        """Whether the path is stroked."""
    @property
    def fill(self) -> Literal['nonzero', 'evenodd'] | None:
        """Fill rule, if filled."""
    @property
    def line_width(self) -> float:
        """Line width before the CTM."""
    @property
    def ctm(self) -> Matrix:
        """CTM at painting time."""
    @property
    def fill_color(self) -> Color:
        """Fill color."""
    @property
    def stroke_color(self) -> Color:
        """Stroke color."""
    @property
    def fill_alpha(self) -> float:
        """Fill alpha."""
    @property
    def stroke_alpha(self) -> float:
        """Stroke alpha."""

@final
class SoftMask:
    """A soft mask image."""

    @property
    def width(self) -> int:
        """Width in samples."""
    @property
    def height(self) -> int:
        """Height in samples."""
    @property
    def bpc(self) -> int:
        """Bits per component."""
    @property
    def decode(self) -> tuple[float, ...] | None:
        """``/Decode`` array."""
    @property
    def kind(self) -> ImageKind:
        """How ``data`` is encoded."""
    @property
    def data(self) -> bytes:
        """Samples, or codec bytes."""

@final
class Image:
    """A placed image."""

    @property
    def name(self) -> str | None:
        """XObject resource name; None for inline images."""
    @property
    def inline(self) -> bool:
        """Whether this is an inline image."""
    @property
    def ctm(self) -> Matrix:
        """Maps the unit square to user space."""
    @property
    def bbox(self) -> Rect:
        """The unit square in user space."""
    @property
    def width(self) -> int:
        """Width in samples."""
    @property
    def height(self) -> int:
        """Height in samples."""
    @property
    def bpc(self) -> int:
        """Bits per component."""
    @property
    def color_space(self) -> Literal['gray', 'rgb', 'cmyk'] | SpaceFamily | None:
        """Color space; None for image masks and undeclared spaces."""
    @property
    def indexed(self) -> tuple[str, int, bytes] | None:
        """``(base family, hival, lookup)`` for indexed images."""
    @property
    def image_mask(self) -> bool:
        """Whether this is a stencil mask."""
    @property
    def decode(self) -> tuple[float, ...] | None:
        """``/Decode`` array."""
    @property
    def kind(self) -> ImageKind:
        """How ``data`` is encoded."""
    @property
    def data(self) -> bytes:
        """Samples after non-codec filters, or codec bytes."""
    @property
    def jbig2_globals(self) -> bytes | None:
        """Decoded ``/JBIG2Globals`` for JBIG2 images."""
    @property
    def ccitt(self) -> dict[str, int | bool] | None:
        """CCITT parameters: k, columns, rows, black_is_1, encoded_byte_align, end_of_line, end_of_block."""
    @property
    def smask(self) -> SoftMask | None:
        """``/SMask``, if present."""

@final
class PageContent:
    """Everything extracted from one page, in content-stream order."""

    @property
    def glyphs(self) -> list[Glyph]:
        """One entry per character code shown."""
    @property
    def paths(self) -> list[Path]:
        """Painted paths."""
    @property
    def images(self) -> list[Image]:
        """One entry per image placement."""
    @property
    def fonts(self) -> list[FontInfo]:
        """Fonts referenced by ``Glyph.font``."""
    @property
    def diagnostics(self) -> list[Diagnostic]:
        """Recoveries made during this extraction."""

@final
class Page:
    """One page of a document."""

    @property
    def index(self) -> int:
        """0-based page index."""
    @property
    def media_box(self) -> Rect:
        """``/MediaBox``."""
    @property
    def crop_box(self) -> Rect:
        """``/CropBox``, defaulting to the media box."""
    @property
    def rotate(self) -> Literal[0, 90, 180, 270]:
        """``/Rotate``, normalized."""
    @property
    def display_matrix(self) -> Matrix:
        """Maps user space to top-left, y-down display space with rotation applied."""
    def content(self) -> PageContent:
        """Extract glyphs, painted paths, and images. Releases the GIL while parsing."""

@final
class Document:
    """An open PDF document."""

    def __init__(self, data: bytes) -> None:
        """Open a document from its bytes.

        Raises:
            PdfError: If the data is not a readable PDF.
            EncryptedError: If the PDF is encrypted.
        """
    def __len__(self) -> int: ...
    def __getitem__(self, index: int) -> Page:
        """The page at ``index``; negative indices count from the end."""
    @property
    def diagnostics(self) -> list[Diagnostic]:
        """Structural recoveries made while opening the file."""
