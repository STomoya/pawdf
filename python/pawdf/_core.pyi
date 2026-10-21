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
    """The PDF is encrypted and could not be decrypted."""

class PasswordError(EncryptedError):
    """No password (given or empty) opened the encrypted PDF."""

class UnsupportedEncryptionError(EncryptedError):
    """The PDF uses an unsupported security handler or encryption method."""

@final
class Permissions:
    """The permission flags ``/P`` of an encrypted PDF. Reported, not enforced."""

    @property
    def bits(self) -> int:
        """The raw ``/P`` value as an unsigned 32-bit integer."""
    @property
    def print(self) -> bool:
        """Printing is allowed (at low quality unless ``print_high_quality``)."""
    @property
    def modify(self) -> bool:
        """Modifying the contents is allowed."""
    @property
    def copy(self) -> bool:
        """Copying or extracting text and graphics is allowed."""
    @property
    def annotate(self) -> bool:
        """Adding or modifying annotations and filling forms is allowed."""
    @property
    def fill_forms(self) -> bool:
        """Filling in form fields is allowed."""
    @property
    def extract_for_accessibility(self) -> bool:
        """Extracting text and graphics for accessibility is allowed."""
    @property
    def assemble(self) -> bool:
        """Assembling the document (inserting, rotating, deleting pages) is allowed."""
    @property
    def print_high_quality(self) -> bool:
        """Printing at full quality is allowed."""

@final
class Encryption:
    """How an encrypted PDF was opened."""

    @property
    def revision(self) -> int:
        """The standard security handler revision ``/R`` (2 to 6)."""
    @property
    def method(self) -> Literal['identity', 'rc4', 'aes-128', 'aes-256']:
        """The encryption method of strings and streams."""
    @property
    def key_bits(self) -> int:
        """The file key length in bits."""
    @property
    def permissions(self) -> Permissions:
        """The permission flags."""
    @property
    def authenticated_as(self) -> Literal['user', 'owner']:
        """Which password opened the file."""

@final
class Diagnostic:
    """A recovery from malformed input."""

    @property
    def kind(self) -> str:
        """The kind of recovery, in snake_case (for example ``'xref_repaired'``)."""
    @property
    def offset(self) -> int:
        """Byte offset in the file, or in the decoded stream when ``object`` is a stream; 0
        when the diagnostic is about ``object`` as a whole.
        """
    @property
    def object(self) -> tuple[int, int] | None:
        """``(number, generation)`` of the stream the offset points into, or of the object the
        diagnostic is about.
        """

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
    @property
    def clip(self) -> Rect | None:
        """Bound of the clipping region in effect, or None when nothing clips.

        A zero-area rect means fully clipped. Content outside it is certainly clipped; content
        inside may still be. Under a text clip the bound uses glyph boxes and is approximate.
        """

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
    @property
    def clip(self) -> Rect | None:
        """Bound of the clipping region in effect, or None when nothing clips.

        A zero-area rect means fully clipped. Content outside it is certainly clipped; content
        inside may still be. Under a text clip the bound uses glyph boxes and is approximate.
        """

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
    @property
    def clip(self) -> Rect | None:
        """Bound of the clipping region in effect, or None when nothing clips.

        A zero-area rect means fully clipped. Content outside it is certainly clipped; content
        inside may still be. Under a text clip the bound uses glyph boxes and is approximate.
        """

@final
class MarkedContent:
    """One marked-content sequence and the content it covers."""

    @property
    def tag(self) -> str:
        """The tag of ``BMC``/``BDC``, for example ``'Span'``, ``'P'``, ``'Artifact'``."""
    @property
    def mcid(self) -> int | None:
        """``/MCID``, linking the content to the structure tree."""
    @property
    def actual_text(self) -> str | None:
        """``/ActualText``; None on sequences nested in one that has it.

        Also None once the page's marked-content text limit is spent.
        """
    @property
    def artifact_type(self) -> str | None:
        """``/Type`` of an ``Artifact`` sequence, for example ``'Pagination'``."""
    @property
    def artifact_subtype(self) -> str | None:
        """``/Subtype`` of an ``Artifact`` sequence, for example ``'Footer'``."""
    @property
    def glyphs(self) -> range:
        """Indices into ``PageContent.glyphs``."""
    @property
    def paths(self) -> range:
        """Indices into ``PageContent.paths``."""
    @property
    def images(self) -> range:
        """Indices into ``PageContent.images``."""

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
    def marked_contents(self) -> list[MarkedContent]:
        """Marked-content sequences, ordered by where they open; a parent precedes its children.

        Sequences nested deeper than 256 or past 2**20 per page are omitted, their content
        counted in the enclosing sequence, and reported as a ``recursion_limit`` diagnostic.
        """
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
        """``/CropBox`` clipped to the media box, defaulting to the media box."""
    @property
    def rotate(self) -> Literal[0, 90, 180, 270]:
        """``/Rotate``, normalized."""
    @property
    def display_matrix(self) -> Matrix:
        """Maps user space to top-left, y-down display space with rotation applied."""
    def content(self) -> PageContent:
        """Extract glyphs, painted paths, and images. Releases the GIL while parsing."""
    def annotations(self) -> Annotations:
        """The page's annotations. Read on every call. Releases the GIL while parsing."""

@final
class Destination:
    """A position in this document."""

    @property
    def page(self) -> int:
        """0-based page index."""
    @property
    def left(self) -> float | None:
        """Left edge in default user space, when the destination gives one."""
    @property
    def top(self) -> float | None:
        """Top edge in default user space, when the destination gives one."""

@final
class OutlineItem:
    """One outline item."""

    @property
    def title(self) -> str:
        """The decoded title; empty when missing."""
    @property
    def dest(self) -> Destination | None:
        """Where the item points, or ``None`` for other actions or unresolvable destinations."""
    @property
    def children(self) -> list[OutlineItem]:
        """Child items, in order."""

@final
class Outline:
    """The document outline (bookmarks)."""

    @property
    def items(self) -> list[OutlineItem]:
        """Top-level items, in order."""
    @property
    def diagnostics(self) -> list[Diagnostic]:
        """Recoveries made while reading the outline."""

@final
class Annotation:
    """One annotation. Its appearance is not read."""

    @property
    def subtype(self) -> str:
        """``/Subtype``, e.g. ``'Link'``, ``'Text'``, ``'Highlight'``, ``'Widget'``; empty when missing."""
    @property
    def rect(self) -> Rect:
        """``/Rect`` in default user space, normalized; all zero when missing or malformed."""
    @property
    def contents(self) -> str | None:
        """``/Contents``, decoded."""
    @property
    def author(self) -> str | None:
        """``/T``, decoded: the author of a markup annotation."""
    @property
    def name(self) -> str | None:
        """``/NM``, decoded: the annotation's name, unique on its page."""
    @property
    def flags(self) -> int:
        """``/F`` annotation flags; 0 when missing or malformed."""
    @property
    def quad_points(self) -> list[tuple[Point, Point, Point, Point]]:
        """``/QuadPoints`` in default user space, four points per quadrilateral, as written."""
    @property
    def dest(self) -> Destination | None:
        """Where ``/Dest`` or a GoTo action points in this document."""
    @property
    def uri(self) -> str | None:
        """The URI of a URI action."""

@final
class Annotations:
    """The annotations of one page."""

    @property
    def items(self) -> list[Annotation]:
        """Annotations in ``/Annots`` order, of every subtype."""
    @property
    def diagnostics(self) -> list[Diagnostic]:
        """Recoveries made while reading the annotations."""

@final
class Document:
    """An open PDF document."""

    def __init__(self, data: bytes, password: str | None = None) -> None:
        """Open a document from its bytes.

        Args:
            data: The PDF file contents.
            password: Password of an encrypted file, tried as the user then the owner
                password. The empty user password is always tried too.

        Raises:
            PdfError: If the data is not a readable PDF.
            PasswordError: If the PDF is encrypted and no password opened it.
            UnsupportedEncryptionError: If the PDF uses an unsupported security handler.
        """
    def __len__(self) -> int: ...
    def __getitem__(self, index: int) -> Page:
        """The page at ``index``; negative indices count from the end."""
    @property
    def diagnostics(self) -> list[Diagnostic]:
        """Structural recoveries made while opening the file."""
    @property
    def encryption(self) -> Encryption | None:
        """How the file was decrypted, or ``None`` if it is not encrypted."""
    def outline(self) -> Outline:
        """The document outline (bookmarks). Read on every call. Releases the GIL while parsing."""
