from collections.abc import Sequence
from typing import Literal, TypeAlias, final

Matrix: TypeAlias = tuple[float, float, float, float, float, float]
Rect: TypeAlias = tuple[float, float, float, float]
Point: TypeAlias = tuple[float, float]
ImageKind: TypeAlias = Literal['samples', 'jpeg', 'jpx', 'jbig2', 'ccitt']
FieldKind: TypeAlias = Literal[
    'text', 'checkbox', 'radio', 'push_button', 'combo_box', 'list_box', 'signature', 'unknown'
]
SpaceFamily: TypeAlias = Literal['cal_gray', 'cal_rgb', 'lab', 'separation', 'device_n', 'indexed', 'unknown']

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
    @property
    def hidden(self) -> bool:
        """Whether optional content that is off in the default configuration encloses it.

        Hidden content is still returned; the caller decides whether to drop it.
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
    @property
    def hidden(self) -> bool:
        """Whether optional content that is off in the default configuration encloses it.

        Hidden content is still returned; the caller decides whether to drop it.
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
    @property
    def hidden(self) -> bool:
        """Whether optional content that is off in the default configuration encloses it.

        Hidden content is still returned; the caller decides whether to drop it.
        """

@final
class MarkedContent:
    """One marked-content sequence and the content it covers."""

    @property
    def tag(self) -> str:
        """The tag of ``BMC``/``BDC``, for example ``'Span'``, ``'P'``, ``'Artifact'``."""
    @property
    def mcid(self) -> int | None:
        """``/MCID``, linking the content to the structure tree. Numbered per content stream."""
    @property
    def stream(self) -> tuple[int, int] | None:
        """The form XObject holding the sequence as (object number, generation); None in the page's own content."""
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
class StructElement:
    """One structure element of a tagged PDF. Of attribute objects, only the table spans are read."""

    @property
    def role(self) -> str:
        """``/S`` as written, e.g. ``P``, ``H1``, or a custom name; empty when missing."""
    @property
    def standard(self) -> str:
        """``role`` after following ``/RoleMap``, which applies even to standard names."""
    @property
    def title(self) -> str | None:
        """``/T``, decoded."""
    @property
    def alt(self) -> str | None:
        """``/Alt``, decoded: alternate description, e.g. of a figure."""
    @property
    def actual_text(self) -> str | None:
        """``/ActualText``, decoded: replacement text."""
    @property
    def lang(self) -> str | None:
        """``/Lang``, decoded."""
    @property
    def expansion(self) -> str | None:
        """``/E``, decoded: expansion of an abbreviation."""
    @property
    def page(self) -> int | None:
        """0-based page index of its own ``/Pg``."""
    @property
    def row_span(self) -> int:
        """``/RowSpan`` of its Table attribute object; 1 when absent or malformed."""
    @property
    def col_span(self) -> int:
        """``/ColSpan`` of its Table attribute object; 1 when absent or malformed."""
    @property
    def children(self) -> list[StructElement | MarkedContentRef | ObjectRef]:
        """Kids, in ``/K`` order."""

@final
class MarkedContentRef:
    """A marked-content sequence of a structure element."""

    @property
    def page(self) -> int | None:
        """0-based page index the content is on."""
    @property
    def mcid(self) -> int:
        """Matches ``MarkedContent.mcid`` on that page, among sequences with the same ``stream``."""
    @property
    def stream(self) -> tuple[int, int] | None:
        """``/Stm`` as (object number, generation): the content stream holding the sequence, when not the page's."""

@final
class ObjectRef:
    """A PDF object of a structure element, such as an annotation."""

    @property
    def object(self) -> tuple[int, int]:
        """``/Obj`` as (object number, generation), e.g. an annotation or an image XObject."""
    @property
    def page(self) -> int | None:
        """0-based page index."""
    @property
    def annotation(self) -> int | None:
        """Index into that page's ``Annotations.items`` when the object is one of them."""

@final
class StructTree:
    """The logical structure tree of a tagged PDF. ``/ParentTree`` is not read."""

    @property
    def roots(self) -> list[StructElement]:
        """The children of ``/StructTreeRoot``, in order."""
    @property
    def diagnostics(self) -> list[Diagnostic]:
        """Recoveries made while reading the tree."""

@final
class Widget:
    """One widget annotation of a form field."""

    @property
    def page(self) -> int | None:
        """0-based page index from ``/Annots``, else the widget's ``/P``; ``None`` when neither resolves."""
    @property
    def rect(self) -> Rect:
        """``/Rect`` in default user space, normalized; all zero when missing or malformed."""

@final
class FormField:
    """One terminal form field. Its appearance is not read."""

    @property
    def name(self) -> str:
        """The fully qualified name: partial names joined with ``.``; empty when none has one."""
    @property
    def label(self) -> str | None:
        """``/TU``, decoded: the name shown to users."""
    @property
    def kind(self) -> FieldKind:
        """The field type."""
    @property
    def values(self) -> list[str]:
        """The value, decoded; empty when unfilled or off.

        More than one only for multi-select list boxes. Password fields are returned as
        written: check ``flags & 0x2000`` (Table 228 bit 14, 1-based) to filter them.
        """
    @property
    def flags(self) -> int:
        """``/Ff`` field flags, inherited; 0 when missing or malformed."""
    @property
    def widgets(self) -> list[Widget]:
        """Where the field appears, in ``/Kids`` order."""

@final
class FormFields:
    """The form fields of a document."""

    @property
    def items(self) -> list[FormField]:
        """Terminal fields, in depth-first order."""
    @property
    def diagnostics(self) -> list[Diagnostic]:
        """Recoveries made while reading the form."""

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
        """``/T``, decoded: the author of a markup annotation.

        Always ``None`` for a ``Widget``, whose ``/T`` is a form field name.
        """
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
    @property
    def hidden(self) -> bool:
        """Whether its ``/OC`` is off in the default configuration. Hidden annotations are still returned."""

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
    def structure_tree(self) -> StructTree:
        """The logical structure tree of a tagged PDF; empty when untagged. Read on every call.

        Releases the GIL while parsing.
        """
    def form_fields(self) -> FormFields:
        """The interactive form fields. Read on every call. Releases the GIL while parsing."""

@final
class Word:
    """A run of glyphs between spaces."""

    @property
    def bbox(self) -> Rect:
        """Union of its glyphs' boxes ``(x0, y0, x1, y1)``, in the algorithm's page space."""
    @property
    def text(self) -> str:
        """Its glyphs' text, concatenated."""
    @property
    def glyphs(self) -> list[int]:
        """Indices into ``PageContent.glyphs``."""

@final
class Line:
    """One line of text."""

    @property
    def bbox(self) -> Rect:
        """Union of its glyphs' boxes ``(x0, y0, x1, y1)``, in the algorithm's page space."""
    @property
    def orientation(self) -> Literal['horizontal', 'vertical']:
        """Writing direction."""
    @property
    def text(self) -> str:
        """Its glyphs' text, with a space wherever the algorithm inserted one."""
    @property
    def words(self) -> list[Word]:
        """Words in order."""
    @property
    def glyphs(self) -> list[int]:
        """Indices into ``PageContent.glyphs`` of all its glyphs, including whitespace glyphs in no word."""

@final
class Block:
    """A group of neighbouring lines."""

    @property
    def bbox(self) -> Rect:
        """Union of its lines' boxes ``(x0, y0, x1, y1)``, in the algorithm's page space."""
    @property
    def orientation(self) -> Literal['horizontal', 'vertical']:
        """Direction of its lines."""
    @property
    def text(self) -> str:
        """Its lines' text joined with newlines."""
    @property
    def lines(self) -> list[Line]:
        """Lines in reading order."""

@final
class Layout:
    """The blocks of one page, in reading order."""

    @property
    def blocks(self) -> list[Block]:
        """Text blocks in reading order."""
    @property
    def diagnostics(self) -> list[Diagnostic]:
        """Recoveries made while reading the page, then those made by layout analysis."""

def layout_pdfminer(
    page: Page,
    *,
    line_overlap: float = 0.5,
    char_margin: float = 2.0,
    line_margin: float = 0.5,
    word_margin: float = 0.1,
    boxes_flow: float | None = 0.5,
    detect_vertical: bool = False,
) -> Layout:
    """Group a page's glyphs into blocks, lines and words with pdfminer.six's layout algorithm.

    Given the same glyph boxes, the result matches pdfminer.six's ``LTPage.analyze``. Results on a
    whole PDF can still differ from running pdfminer on it: glyph boxes are measured differently,
    text in form XObjects is grouped with the rest of the page, ties between equally distant
    blocks are broken by creation order rather than memory address, and a ``/MediaBox`` written
    with its corners reversed is normalised where pdfminer keeps it as written. A page that hits a
    work or block limit gets a simpler grouping or order and a ``layout_limit`` diagnostic; that
    result does not match pdfminer.

    Boxes are in pdfminer's upright page space: the media box's lower-left corner is the origin,
    ``/Rotate`` is applied, and y points up. They differ from ``Glyph.bbox`` on rotated or shifted
    pages. Hidden optional content is left out; invisible text is kept. ``/ActualText`` is not
    applied; use the glyph indices with ``PageContent.marked_contents`` for that.

    Reading the page again with ``Page.content()`` gives the glyphs the indices refer to.
    Releases the GIL while reading and analyzing.

    Args:
        page: The page to analyze.
        line_overlap: Two glyphs whose overlap exceeds this fraction of the smaller height can
            share a line.
        char_margin: Two glyphs closer than this multiple of the wider glyph's width can share
            a line.
        line_margin: Two lines closer than this multiple of the line height can share a block.
        word_margin: A gap wider than this multiple of a glyph's larger dimension inserts a space.
        boxes_flow: How much horizontal (-1.0) versus vertical (+1.0) position decides block
            order. ``None`` sorts blocks by their lower-left corner instead.
        detect_vertical: Whether vertical lines are detected.

    Returns:
        The page's blocks in reading order.

    Raises:
        ValueError: If a setting is not finite, or ``boxes_flow`` is outside [-1, 1].
    """

def layout_text_in_regions(
    page: Page,
    regions: Sequence[Rect],
    *,
    line_overlap: float = 0.5,
    char_margin: float = 2.0,
    line_margin: float = 0.5,
    word_margin: float = 0.1,
    boxes_flow: float | None = 0.5,
    detect_vertical: bool = False,
) -> list[Layout]:
    """Group the glyphs in each region on their own, with pdfminer.six's layout algorithm.

    Meant for boxes from an external detector, such as table cells: text from neighbouring
    regions never shares a line, as it can when the whole page is grouped at once.

    Regions are ``(x0, y0, x1, y1)`` in display space (see ``Page.display_matrix``): the origin
    is the top-left of the rotated crop box and y points down, in points. A box from an image
    rendered at ``dpi`` maps to it by dividing by ``dpi / 72``. Corners may come in either order.
    A glyph belongs to a region when its box's centre is inside it; the right and bottom edges
    are excluded, so regions that share an edge never share a glyph.

    Result boxes are in pdfminer's upright page space, as for ``layout_pdfminer``, not in display
    space. Each layout's diagnostics hold the recoveries made while reading the page, then those
    made grouping that region. Settings are as for ``layout_pdfminer``. Releases the GIL while
    reading and analyzing.

    Args:
        page: The page to read.
        regions: Boxes in display space.
        line_overlap: As for ``layout_pdfminer``.
        char_margin: As for ``layout_pdfminer``.
        line_margin: As for ``layout_pdfminer``.
        word_margin: As for ``layout_pdfminer``.
        boxes_flow: As for ``layout_pdfminer``.
        detect_vertical: As for ``layout_pdfminer``.

    Returns:
        One layout per region, in the order given.

    Raises:
        ValueError: If a region is not finite, a setting is not finite, or ``boxes_flow`` is
            outside [-1, 1].
    """

@final
class Table:
    """A table on one page."""

    @property
    def bbox(self) -> Rect:
        """Union of its cells' boxes, in display space."""
    @property
    def rows(self) -> int:
        """Rows of the whole table, including rows on other pages."""
    @property
    def cols(self) -> int:
        """Columns of the whole table: the widest row, counting spans."""
    @property
    def cells(self) -> list[Cell]:
        """Cells of the rows on this page, row by row."""

@final
class Cell:
    """One table cell."""

    @property
    def row(self) -> int:
        """0-based row in the whole table, so parts of a table split across pages line up."""
    @property
    def col(self) -> int:
        """0-based column."""
    @property
    def row_span(self) -> int:
        """Rows it spans, clipped to the table's last row."""
    @property
    def col_span(self) -> int:
        """Columns it spans."""
    @property
    def header(self) -> bool:
        """Whether it is a header cell (``TH``)."""
    @property
    def bbox(self) -> Rect | None:
        """Union of its glyphs' boxes on this page, in display space; ``None`` when it has none."""
    @property
    def layout(self) -> Layout:
        """Its text, grouped with pdfminer.six's algorithm, with boxes in pdfminer's page space."""

def layout_tables_tagged(
    page: Page,
    tree: StructTree,
    region: Rect | None = None,
    *,
    line_overlap: float = 0.5,
    char_margin: float = 2.0,
    line_margin: float = 0.5,
    word_margin: float = 0.1,
    boxes_flow: float | None = 0.5,
    detect_vertical: bool = False,
) -> list[Table]:
    """Read the tables of a page in a tagged PDF from its structure tree.

    Tables are the ``Table`` elements of ``tree``, in tree order; a table nested in a cell is
    returned on its own as well. Rows are ``TR`` elements, directly or in ``THead``, ``TBody`` or
    ``TFoot``; cells are their ``TH`` and ``TD`` elements, placed on the grid as in HTML. A cell's
    text is the marked content under it on this page.

    A table is returned when it has glyphs on this page. Rows with content only on other pages
    are left out, and row indices count every row, so the parts of a table split across pages
    can be joined.

    Boxes are in display space, as for ``layout_text_in_regions``, so a cell box can be passed to
    it. With a ``region``, also in display space with corners in either order, only tables whose
    box overlaps it are returned. Each cell layout's diagnostics hold the recoveries made while
    reading the page and the tree, then those made grouping that cell. Settings are as for
    ``layout_pdfminer``. Releases the GIL while reading and analyzing.

    Args:
        page: The page to read.
        tree: The document's structure tree, from ``Document.structure_tree``.
        region: The extent of a table, in display space.
        line_overlap: As for ``layout_pdfminer``.
        char_margin: As for ``layout_pdfminer``.
        line_margin: As for ``layout_pdfminer``.
        word_margin: As for ``layout_pdfminer``.
        boxes_flow: As for ``layout_pdfminer``.
        detect_vertical: As for ``layout_pdfminer``.

    Returns:
        The page's tables, in tree order.

    Raises:
        ValueError: If the region is not finite, a setting is not finite, or ``boxes_flow`` is
            outside [-1, 1].
    """
