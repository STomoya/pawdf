//! What one extraction pass over a page returns. All coordinates are in default user
//! space (bottom-left origin, y up, `/Rotate` not applied).

use std::ops::Range;

use pawdf_core::Diagnostic;

use crate::geom::{Matrix, Point, Rect};

/// Everything extracted from one page, in content-stream order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PageContent {
    /// One entry per character code shown.
    pub glyphs: Vec<Glyph>,
    /// Painted paths. Clip-only and `n` paths are dropped.
    pub paths: Vec<Path>,
    /// One entry per image placement.
    pub images: Vec<Image>,
    /// Fonts referenced by [`Glyph::font`].
    pub fonts: Vec<FontInfo>,
    /// Marked-content sequences (§14.6), ordered by where they open; a parent precedes its
    /// children. Sequences nested deeper than 256 or past 2^20 per page are omitted, their
    /// content counted in the enclosing sequence, and reported as a `RecursionLimit` diagnostic.
    pub marked_contents: Vec<MarkedContent>,
    /// Recoveries made during this pass only.
    pub diagnostics: Vec<Diagnostic>,
}

/// One marked-content sequence and the content it covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkedContent {
    /// The tag operand of `BMC`/`BDC`, e.g. `Span`, `P`, `Artifact`. Empty when malformed,
    /// and, like the other strings (left `None`), once the page's marked-content text exceeds
    /// a limit, which is reported as a `RecursionLimit` diagnostic.
    pub tag: String,
    /// `/MCID` (§14.7.5).
    pub mcid: Option<u32>,
    /// `/ActualText` (§14.9.4). `None` on sequences nested in one that has it, so
    /// replacement texts never overlap, and once the marked-content text limit is spent (see
    /// [`MarkedContent::tag`]).
    pub actual_text: Option<String>,
    /// `/Type` of an `Artifact` sequence (§14.8.2.2); `None` for other tags.
    pub artifact_type: Option<String>,
    /// `/Subtype` of an `Artifact` sequence (§14.8.2.2); `None` for other tags.
    pub artifact_subtype: Option<String>,
    /// Indices into [`PageContent::glyphs`]; may be empty.
    pub glyphs: Range<usize>,
    /// Indices into [`PageContent::paths`]; may be empty.
    pub paths: Range<usize>,
    /// Indices into [`PageContent::images`]; may be empty.
    pub images: Range<usize>,
}

/// One shown character code.
#[derive(Debug, Clone, PartialEq)]
pub struct Glyph {
    /// Unicode text; several characters for a ligature, U+FFFD when unmappable.
    pub text: String,
    /// The raw character code.
    pub code: u32,
    /// Index into [`PageContent::fonts`].
    pub font: usize,
    /// Font size (`Tfs`).
    pub size: f64,
    /// Text rendering matrix `Trm` (§9.4.4).
    pub matrix: Matrix,
    /// Glyph origin.
    pub origin: Point,
    /// Length of the displacement along the writing direction.
    pub advance: f64,
    /// Axis-aligned bound of the glyph box.
    pub bbox: Rect,
    /// Writing mode of the font.
    pub wmode: WritingMode,
    /// Text rendering mode (`Tr`).
    pub render_mode: RenderMode,
    /// Fill color.
    pub fill: Color,
    /// Stroke color.
    pub stroke: Color,
    /// Fill alpha (`ca`).
    pub fill_alpha: f64,
    /// Stroke alpha (`CA`).
    pub stroke_alpha: f64,
    /// Axis-aligned bound of the clipping region in effect, in default user space; `None`
    /// when nothing clips. A zero-area rectangle means fully clipped. Content outside it is
    /// certainly clipped; content inside may still be. Under a text clip (§9.3.6) the bound
    /// uses glyph boxes, which real outlines can exceed, so it is approximate.
    pub clip: Option<Rect>,
    /// Whether optional content (§8.11) that is off in the default configuration encloses
    /// it: a `BDC /OC` sequence or a form XObject with `/OC`. Such content is still returned;
    /// the caller decides whether to drop it.
    pub hidden: bool,
}

/// Writing mode (§9.7.4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WritingMode {
    /// Left to right.
    Horizontal,
    /// Top to bottom.
    Vertical,
}

/// Text rendering mode `Tr` 0..=7 (§9.3.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    /// 0.
    Fill,
    /// 1.
    Stroke,
    /// 2.
    FillStroke,
    /// 3: neither filled nor stroked.
    Invisible,
    /// 4.
    FillClip,
    /// 5.
    StrokeClip,
    /// 6.
    FillStrokeClip,
    /// 7.
    Clip,
}

/// A font as seen from one page.
#[derive(Debug, Clone, PartialEq)]
pub struct FontInfo {
    /// `/BaseFont`, including any subset tag.
    pub base_font: String,
    /// The font type.
    pub kind: FontKind,
    /// Whether a font program is embedded.
    pub embedded: bool,
    /// Writing mode.
    pub wmode: WritingMode,
}

/// Font dictionary `/Subtype`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontKind {
    /// `Type1`.
    Type1,
    /// `MMType1`.
    MmType1,
    /// `TrueType`.
    TrueType,
    /// `Type3`.
    Type3,
    /// `Type0` (composite).
    Type0,
}

/// A painted path.
#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    /// Segments in user space.
    pub segments: Vec<Segment>,
    /// Axis-aligned bound of the clipping region in effect, in default user space; `None`
    /// when nothing clips. A zero-area rectangle means fully clipped. Content outside it is
    /// certainly clipped; content inside may still be. Under a text clip (§9.3.6) the bound
    /// uses glyph boxes, which real outlines can exceed, so it is approximate.
    pub clip: Option<Rect>,
    /// Whether it is stroked.
    pub stroke: bool,
    /// The fill rule, if filled.
    pub fill: Option<FillRule>,
    /// Line width in the path's own space; transform it by `ctm` for user space.
    pub line_width: f64,
    /// CTM at painting time.
    pub ctm: Matrix,
    /// Fill color.
    pub fill_color: Color,
    /// Stroke color.
    pub stroke_color: Color,
    /// Fill alpha (`ca`).
    pub fill_alpha: f64,
    /// Stroke alpha (`CA`).
    pub stroke_alpha: f64,
    /// Whether optional content (§8.11) that is off in the default configuration encloses
    /// it: a `BDC /OC` sequence or a form XObject with `/OC`. Such content is still returned;
    /// the caller decides whether to drop it.
    pub hidden: bool,
}

/// A path segment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Segment {
    /// Start a subpath.
    MoveTo(Point),
    /// Straight line.
    LineTo(Point),
    /// Cubic Bézier: two control points, then the end point.
    CurveTo(Point, Point, Point),
    /// Close the current subpath.
    Close,
}

/// Fill rule (§8.5.3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillRule {
    /// Nonzero winding number.
    NonZero,
    /// Even-odd.
    EvenOdd,
}

/// A color in its own color space; no conversion is done.
#[derive(Debug, Clone, PartialEq)]
pub enum Color {
    /// DeviceGray, or ICCBased with one component.
    Gray(f32),
    /// DeviceRGB, or ICCBased with three components.
    Rgb([f32; 3]),
    /// DeviceCMYK, or ICCBased with four components.
    Cmyk([f32; 4]),
    /// Any other color space, with its raw components.
    Other {
        /// The color space family.
        family: ColorSpaceFamily,
        /// The components as given.
        components: Vec<f32>,
    },
    /// A pattern, by its resource name.
    Pattern(String),
}

/// Color space families other than the device spaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSpaceFamily {
    /// `CalGray`.
    CalGray,
    /// `CalRGB`.
    CalRgb,
    /// `Lab`.
    Lab,
    /// `Separation`.
    Separation,
    /// `DeviceN`.
    DeviceN,
    /// `Indexed`.
    Indexed,
    /// Unrecognized, or `ICCBased` with an unusual component count.
    Unknown,
}

/// A placed image.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    /// XObject resource name; `None` for inline images.
    pub name: Option<String>,
    /// Axis-aligned bound of the clipping region in effect, in default user space; `None`
    /// when nothing clips. A zero-area rectangle means fully clipped. Content outside it is
    /// certainly clipped; content inside may still be. Under a text clip (§9.3.6) the bound
    /// uses glyph boxes, which real outlines can exceed, so it is approximate.
    pub clip: Option<Rect>,
    /// Whether this is an inline image (`BI ID EI`).
    pub inline: bool,
    /// Maps the unit square to user space.
    pub ctm: Matrix,
    /// The unit square in user space.
    pub bbox: Rect,
    /// Width in samples.
    pub width: u32,
    /// Height in samples.
    pub height: u32,
    /// Bits per component.
    pub bpc: u8,
    /// Color space; `None` for image masks and images that do not declare one.
    pub color_space: Option<ColorSpace>,
    /// Whether this is a stencil mask (`/ImageMask true`).
    pub image_mask: bool,
    /// `/Decode` array, if present.
    pub decode: Option<Vec<f32>>,
    /// Sample data, or codec bytes.
    pub data: ImageData,
    /// `/SMask`, if present.
    pub smask: Option<SoftMask>,
    /// Whether optional content (§8.11) that is off in the default configuration encloses
    /// it: a `BDC /OC` sequence, a form XObject with `/OC`, or the image's own `/OC`. Such
    /// content is still returned; the caller decides whether to drop it.
    pub hidden: bool,
}

/// An image color space.
#[derive(Debug, Clone, PartialEq)]
pub enum ColorSpace {
    /// One component.
    Gray,
    /// Three components.
    Rgb,
    /// Four components.
    Cmyk,
    /// A palette over `base`.
    Indexed {
        /// The base color space.
        base: Box<ColorSpace>,
        /// The highest valid index.
        hival: u8,
        /// `(hival + 1) × components(base)` bytes.
        lookup: Vec<u8>,
    },
    /// Any other color space.
    Other {
        /// The family.
        family: ColorSpaceFamily,
        /// Number of components.
        components: u8,
    },
}

/// Image bytes after the non-codec filters.
#[derive(Debug, Clone, PartialEq)]
pub enum ImageData {
    /// Decoded samples, packed per row as in the PDF.
    Samples(Vec<u8>),
    /// JPEG (`DCTDecode`) file bytes.
    Jpeg(Vec<u8>),
    /// JPEG 2000 (`JPXDecode`) bytes.
    Jpx(Vec<u8>),
    /// JBIG2 embedded stream.
    Jbig2 {
        /// The page data.
        data: Vec<u8>,
        /// Decoded `/JBIG2Globals`, if any.
        globals: Option<Vec<u8>>,
    },
    /// CCITT fax data.
    Ccitt {
        /// The encoded data.
        data: Vec<u8>,
        /// The decode parameters.
        params: CcittParams,
    },
}

/// `CCITTFaxDecode` parameters (§7.4.6, Table 11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CcittParams {
    /// `/K`.
    pub k: i32,
    /// `/Columns`.
    pub columns: u32,
    /// `/Rows`.
    pub rows: u32,
    /// `/BlackIs1`.
    pub black_is_1: bool,
    /// `/EncodedByteAlign`.
    pub encoded_byte_align: bool,
    /// `/EndOfLine`.
    pub end_of_line: bool,
    /// `/EndOfBlock`.
    pub end_of_block: bool,
}

/// A soft mask image (`/SMask`), always DeviceGray.
#[derive(Debug, Clone, PartialEq)]
pub struct SoftMask {
    /// Width in samples.
    pub width: u32,
    /// Height in samples.
    pub height: u32,
    /// Bits per component.
    pub bpc: u8,
    /// `/Decode` array, if present.
    pub decode: Option<Vec<f32>>,
    /// The mask data.
    pub data: ImageData,
}

/// The document outline (bookmarks, §12.3.3).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outline {
    /// Top-level items, in `/First` → `/Next` order.
    pub items: Vec<OutlineItem>,
    /// Recoveries made while reading the outline.
    pub diagnostics: Vec<Diagnostic>,
}

/// One outline item.
#[derive(Debug, Clone, PartialEq)]
pub struct OutlineItem {
    /// `/Title`, decoded; empty when missing.
    pub title: String,
    /// Where the item points in this document, from `/Dest` or a GoTo action. `None` for
    /// other actions or when the destination cannot be resolved.
    pub dest: Option<Destination>,
    /// Child items, in order.
    pub children: Vec<OutlineItem>,
}

/// A position in this document (§12.3.2).
#[derive(Debug, Clone, PartialEq)]
pub struct Destination {
    /// 0-based page index.
    pub page: usize,
    /// Left edge in default user space, when the destination gives one.
    pub left: Option<f64>,
    /// Top edge in default user space, when the destination gives one.
    pub top: Option<f64>,
}

/// The annotations of one page (§12.5).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Annotations {
    /// Annotations in `/Annots` order, of every subtype.
    pub items: Vec<Annotation>,
    /// Recoveries made while reading the annotations.
    pub diagnostics: Vec<Diagnostic>,
}

/// One annotation. Its appearance is not read.
#[derive(Debug, Clone, PartialEq)]
pub struct Annotation {
    /// `/Subtype`, e.g. `Link`, `Text`, `Highlight`, `Widget`; empty when missing.
    pub subtype: String,
    /// `/Rect` in default user space, normalized; all zero when missing or malformed.
    pub rect: Rect,
    /// `/Contents`, decoded.
    pub contents: Option<String>,
    /// `/T`, decoded: the author of a markup annotation (§12.5.6.2). Always `None` for a
    /// `Widget`, whose `/T` is a form field name.
    pub author: Option<String>,
    /// `/NM`, decoded: the annotation's name, unique on its page.
    pub name: Option<String>,
    /// `/F` annotation flags (§12.5.3); 0 when missing or malformed.
    pub flags: u32,
    /// `/QuadPoints` in default user space, four points per quadrilateral, in the order
    /// written.
    pub quad_points: Vec<[Point; 4]>,
    /// Where `/Dest` or a GoTo action points in this document, as for outline items.
    pub dest: Option<Destination>,
    /// The URI of a URI action.
    pub uri: Option<String>,
    /// Whether its `/OC` (§8.11.3.4) is off in the default configuration. Such annotations are
    /// still returned.
    pub hidden: bool,
}

/// The interactive form fields of a document (AcroForm, §12.7).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FormFields {
    /// Terminal fields, in depth-first `/Fields` → `/Kids` order.
    pub items: Vec<FormField>,
    /// Recoveries made while reading the form.
    pub diagnostics: Vec<Diagnostic>,
}

/// One terminal form field. Its appearance is not read.
#[derive(Debug, Clone, PartialEq)]
pub struct FormField {
    /// The fully qualified name (§12.7.4.2): the partial names `/T` of the field and its
    /// ancestors joined with `.`, skipping levels without one. Empty when no level has one.
    pub name: String,
    /// `/TU`, decoded: the name shown to users.
    pub label: Option<String>,
    /// The field type, from the inherited `/FT` and `/Ff`.
    pub kind: FieldKind,
    /// The value `/V`, decoded, without empty strings; empty when unfilled, off, or not a
    /// value-bearing kind. More
    /// than one only for multi-select list boxes. Password fields are returned as written:
    /// check `flags & 0x2000` (Table 228 bit 14, 1-based) to filter them.
    pub values: Vec<String>,
    /// `/Ff` field flags, inherited (Table 226); 0 when missing or malformed.
    pub flags: u32,
    /// Where the field appears, in `/Kids` order.
    pub widgets: Vec<Widget>,
}

/// A form field type (§12.7.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    /// `/Tx`.
    Text,
    /// `/Btn` without the Radio or Pushbutton flag.
    CheckBox,
    /// `/Btn` with the Radio flag.
    RadioButton,
    /// `/Btn` with the Pushbutton flag.
    PushButton,
    /// `/Ch` with the Combo flag.
    ComboBox,
    /// `/Ch` without the Combo flag.
    ListBox,
    /// `/Sig`.
    Signature,
    /// `/FT` missing or unrecognized.
    Unknown,
}

/// One widget annotation of a form field.
#[derive(Debug, Clone, PartialEq)]
pub struct Widget {
    /// 0-based page index from `/Annots`, else the widget's `/P`; `None` when neither resolves.
    pub page: Option<usize>,
    /// `/Rect` in default user space, normalized; all zero when missing or malformed.
    pub rect: Rect,
}
