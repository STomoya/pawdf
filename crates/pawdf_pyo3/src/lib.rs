use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyIndexError, PyOverflowError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyList, PyRange, PyTuple};

use pawdf_reader as r;
use r::{Diagnostic as CoreDiagnostic, Location};

create_exception!(
    pawdf._core,
    PdfError,
    PyException,
    "The PDF could not be read."
);
create_exception!(
    pawdf._core,
    EncryptedError,
    PdfError,
    "The PDF is encrypted and could not be decrypted."
);
create_exception!(
    pawdf._core,
    PasswordError,
    EncryptedError,
    "No password (given or empty) opened the encrypted PDF."
);
create_exception!(
    pawdf._core,
    UnsupportedEncryptionError,
    EncryptedError,
    "The PDF uses an unsupported security handler or encryption method."
);

type Rect4 = (f64, f64, f64, f64);
type Matrix6 = (f64, f64, f64, f64, f64, f64);

fn rect(r: r::Rect) -> Rect4 {
    (r.x0, r.y0, r.x1, r.y1)
}

fn matrix(m: r::Matrix) -> Matrix6 {
    (m.a, m.b, m.c, m.d, m.e, m.f)
}

fn to_py_err(e: r::Error) -> PyErr {
    match e.kind {
        r::ErrorKind::Core(c) if matches!(c.kind, r::CoreErrorKind::IncorrectPassword { .. }) => {
            PasswordError::new_err(c.to_string())
        }
        r::ErrorKind::Core(c) if matches!(c.kind, r::CoreErrorKind::UnsupportedEncryption(_)) => {
            UnsupportedEncryptionError::new_err(c.to_string())
        }
        r::ErrorKind::PageOutOfRange { .. } => PyIndexError::new_err(e.to_string()),
        r::ErrorKind::Core(_) => PdfError::new_err(e.to_string()),
    }
}

fn wmode(w: r::WritingMode) -> &'static str {
    match w {
        r::WritingMode::Horizontal => "horizontal",
        r::WritingMode::Vertical => "vertical",
    }
}

fn family(f: r::ColorSpaceFamily) -> &'static str {
    match f {
        r::ColorSpaceFamily::CalGray => "cal_gray",
        r::ColorSpaceFamily::CalRgb => "cal_rgb",
        r::ColorSpaceFamily::Lab => "lab",
        r::ColorSpaceFamily::Separation => "separation",
        r::ColorSpaceFamily::DeviceN => "device_n",
        r::ColorSpaceFamily::Indexed => "indexed",
        r::ColorSpaceFamily::Unknown => "unknown",
    }
}

fn space_name(s: &r::ColorSpace) -> &'static str {
    match s {
        r::ColorSpace::Gray => "gray",
        r::ColorSpace::Rgb => "rgb",
        r::ColorSpace::Cmyk => "cmyk",
        r::ColorSpace::Indexed { .. } => "indexed",
        r::ColorSpace::Other { family: f, .. } => family(*f),
    }
}

fn image_kind(d: &r::ImageData) -> &'static str {
    match d {
        r::ImageData::Samples(_) => "samples",
        r::ImageData::Jpeg(_) => "jpeg",
        r::ImageData::Jpx(_) => "jpx",
        r::ImageData::Jbig2 { .. } => "jbig2",
        r::ImageData::Ccitt { .. } => "ccitt",
    }
}

fn image_bytes(d: &r::ImageData) -> &[u8] {
    match d {
        r::ImageData::Samples(b) | r::ImageData::Jpeg(b) | r::ImageData::Jpx(b) => b,
        r::ImageData::Jbig2 { data, .. } | r::ImageData::Ccitt { data, .. } => data,
    }
}

/// A recovery from malformed input.
#[pyclass(frozen, module = "pawdf._core")]
struct Diagnostic(CoreDiagnostic);

#[pymethods]
impl Diagnostic {
    #[getter]
    fn kind(&self) -> &'static str {
        self.0.kind.name()
    }

    #[getter]
    fn offset(&self) -> u64 {
        match self.0.location {
            Location::File(o) | Location::Stream { offset: o, .. } => o,
            Location::Object(_) => 0,
        }
    }

    #[getter]
    fn object(&self) -> Option<(u32, u16)> {
        match self.0.location {
            Location::Stream { obj, .. } | Location::Object(obj) => Some((obj.num, obj.generation)),
            Location::File(_) => None,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "Diagnostic(kind='{}', offset={}, object={})",
            self.kind(),
            self.offset(),
            match self.object() {
                Some((n, g)) => format!("({n}, {g})"),
                None => "None".to_owned(),
            }
        )
    }
}

fn diagnostics(py: Python<'_>, d: Vec<CoreDiagnostic>) -> PyResult<Py<PyList>> {
    let items = d
        .into_iter()
        .map(|x| Py::new(py, Diagnostic(x)))
        .collect::<PyResult<Vec<_>>>()?;
    Ok(PyList::new(py, items)?.unbind())
}

/// A color in its own color space.
#[pyclass(frozen, module = "pawdf._core")]
struct Color(r::Color);

#[pymethods]
impl Color {
    #[getter]
    fn family(&self) -> &'static str {
        match &self.0 {
            r::Color::Gray(_) => "gray",
            r::Color::Rgb(_) => "rgb",
            r::Color::Cmyk(_) => "cmyk",
            r::Color::Pattern(_) => "pattern",
            r::Color::Other { family: f, .. } => family(*f),
        }
    }

    #[getter]
    fn components<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        let c: Vec<f64> = match &self.0 {
            r::Color::Gray(g) => vec![f64::from(*g)],
            r::Color::Rgb(c) => c.iter().map(|&v| f64::from(v)).collect(),
            r::Color::Cmyk(c) => c.iter().map(|&v| f64::from(v)).collect(),
            r::Color::Other { components, .. } => {
                components.iter().map(|&v| f64::from(v)).collect()
            }
            r::Color::Pattern(_) => Vec::new(),
        };
        PyTuple::new(py, c)
    }

    #[getter]
    fn pattern(&self) -> Option<&str> {
        match &self.0 {
            r::Color::Pattern(p) => Some(p),
            _ => None,
        }
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!(
            "Color(family={:?}, components={})",
            self.family(),
            self.components(py)?.repr()?
        ))
    }
}

/// A font as seen from one page.
#[pyclass(frozen, module = "pawdf._core")]
struct FontInfo(r::FontInfo);

#[pymethods]
impl FontInfo {
    #[getter]
    fn base_font(&self) -> &str {
        &self.0.base_font
    }

    #[getter]
    fn kind(&self) -> &'static str {
        match self.0.kind {
            r::FontKind::Type1 => "type1",
            r::FontKind::MmType1 => "mmtype1",
            r::FontKind::TrueType => "truetype",
            r::FontKind::Type3 => "type3",
            r::FontKind::Type0 => "type0",
        }
    }

    #[getter]
    fn embedded(&self) -> bool {
        self.0.embedded
    }

    #[getter]
    fn wmode(&self) -> &'static str {
        wmode(self.0.wmode)
    }

    fn __repr__(&self) -> String {
        format!(
            "FontInfo(base_font={:?}, kind={:?}, embedded={})",
            self.0.base_font,
            self.kind(),
            if self.0.embedded { "True" } else { "False" }
        )
    }
}

/// One shown character code.
#[pyclass(frozen, module = "pawdf._core")]
struct Glyph(r::Glyph);

#[pymethods]
impl Glyph {
    #[getter]
    fn text(&self) -> &str {
        &self.0.text
    }

    #[getter]
    fn code(&self) -> u32 {
        self.0.code
    }

    #[getter]
    fn font(&self) -> usize {
        self.0.font
    }

    #[getter]
    fn size(&self) -> f64 {
        self.0.size
    }

    #[getter]
    fn matrix(&self) -> Matrix6 {
        matrix(self.0.matrix)
    }

    #[getter]
    fn origin(&self) -> (f64, f64) {
        (self.0.origin.x, self.0.origin.y)
    }

    #[getter]
    fn advance(&self) -> f64 {
        self.0.advance
    }

    #[getter]
    fn bbox(&self) -> Rect4 {
        rect(self.0.bbox)
    }

    #[getter]
    fn wmode(&self) -> &'static str {
        wmode(self.0.wmode)
    }

    #[getter]
    fn render_mode(&self) -> u8 {
        self.0.render_mode as u8
    }

    #[getter]
    fn fill(&self) -> Color {
        Color(self.0.fill.clone())
    }

    #[getter]
    fn stroke(&self) -> Color {
        Color(self.0.stroke.clone())
    }

    #[getter]
    fn fill_alpha(&self) -> f64 {
        self.0.fill_alpha
    }

    #[getter]
    fn stroke_alpha(&self) -> f64 {
        self.0.stroke_alpha
    }

    #[getter]
    fn clip(&self) -> Option<Rect4> {
        self.0.clip.map(rect)
    }

    fn __repr__(&self) -> String {
        format!(
            "Glyph(text={:?}, origin={:?}, font={})",
            self.0.text,
            self.origin(),
            self.0.font
        )
    }
}

/// A painted path.
#[pyclass(frozen, module = "pawdf._core")]
struct Path(r::Path);

#[pymethods]
impl Path {
    #[getter]
    fn segments<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let items = self
            .0
            .segments
            .iter()
            .map(|s| {
                let (op, pts): (&str, Vec<f64>) = match *s {
                    r::Segment::MoveTo(p) => ("m", vec![p.x, p.y]),
                    r::Segment::LineTo(p) => ("l", vec![p.x, p.y]),
                    r::Segment::CurveTo(a, b, c) => ("c", vec![a.x, a.y, b.x, b.y, c.x, c.y]),
                    r::Segment::Close => ("h", Vec::new()),
                };
                (op, PyTuple::new(py, pts)?).into_pyobject(py)
            })
            .collect::<PyResult<Vec<_>>>()?;
        PyList::new(py, items)
    }

    #[getter]
    fn stroke(&self) -> bool {
        self.0.stroke
    }

    #[getter]
    fn fill(&self) -> Option<&'static str> {
        self.0.fill.map(|f| match f {
            r::FillRule::NonZero => "nonzero",
            r::FillRule::EvenOdd => "evenodd",
        })
    }

    #[getter]
    fn line_width(&self) -> f64 {
        self.0.line_width
    }

    #[getter]
    fn ctm(&self) -> Matrix6 {
        matrix(self.0.ctm)
    }

    #[getter]
    fn fill_color(&self) -> Color {
        Color(self.0.fill_color.clone())
    }

    #[getter]
    fn stroke_color(&self) -> Color {
        Color(self.0.stroke_color.clone())
    }

    #[getter]
    fn fill_alpha(&self) -> f64 {
        self.0.fill_alpha
    }

    #[getter]
    fn stroke_alpha(&self) -> f64 {
        self.0.stroke_alpha
    }

    #[getter]
    fn clip(&self) -> Option<Rect4> {
        self.0.clip.map(rect)
    }

    fn __repr__(&self) -> String {
        format!(
            "Path(segments={}, stroke={}, fill={:?})",
            self.0.segments.len(),
            if self.0.stroke { "True" } else { "False" },
            self.fill()
        )
    }
}

fn decode_tuple<'py>(
    py: Python<'py>,
    d: &Option<Vec<f32>>,
) -> PyResult<Option<Bound<'py, PyTuple>>> {
    d.as_ref()
        .map(|d| PyTuple::new(py, d.iter().map(|&v| f64::from(v))))
        .transpose()
}

/// A soft mask (`/SMask`) image.
#[pyclass(frozen, module = "pawdf._core")]
struct SoftMask(r::SoftMask);

#[pymethods]
impl SoftMask {
    #[getter]
    fn width(&self) -> u32 {
        self.0.width
    }

    #[getter]
    fn height(&self) -> u32 {
        self.0.height
    }

    #[getter]
    fn bpc(&self) -> u8 {
        self.0.bpc
    }

    #[getter]
    fn decode<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyTuple>>> {
        decode_tuple(py, &self.0.decode)
    }

    #[getter]
    fn kind(&self) -> &'static str {
        image_kind(&self.0.data)
    }

    #[getter]
    fn data<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, image_bytes(&self.0.data))
    }

    fn __repr__(&self) -> String {
        format!(
            "SoftMask(width={}, height={}, kind={:?})",
            self.0.width,
            self.0.height,
            self.kind()
        )
    }
}

/// A placed image.
#[pyclass(frozen, module = "pawdf._core")]
struct Image(r::Image);

#[pymethods]
impl Image {
    #[getter]
    fn name(&self) -> Option<&str> {
        self.0.name.as_deref()
    }

    #[getter]
    fn inline(&self) -> bool {
        self.0.inline
    }

    #[getter]
    fn ctm(&self) -> Matrix6 {
        matrix(self.0.ctm)
    }

    #[getter]
    fn bbox(&self) -> Rect4 {
        rect(self.0.bbox)
    }

    #[getter]
    fn width(&self) -> u32 {
        self.0.width
    }

    #[getter]
    fn height(&self) -> u32 {
        self.0.height
    }

    #[getter]
    fn bpc(&self) -> u8 {
        self.0.bpc
    }

    #[getter]
    fn color_space(&self) -> Option<&'static str> {
        self.0.color_space.as_ref().map(space_name)
    }

    #[getter]
    fn indexed<'py>(&self, py: Python<'py>) -> Option<(&'static str, u8, Bound<'py, PyBytes>)> {
        match &self.0.color_space {
            Some(r::ColorSpace::Indexed {
                base,
                hival,
                lookup,
            }) => Some((space_name(base), *hival, PyBytes::new(py, lookup))),
            _ => None,
        }
    }

    #[getter]
    fn image_mask(&self) -> bool {
        self.0.image_mask
    }

    #[getter]
    fn decode<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyTuple>>> {
        decode_tuple(py, &self.0.decode)
    }

    #[getter]
    fn kind(&self) -> &'static str {
        image_kind(&self.0.data)
    }

    #[getter]
    fn data<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, image_bytes(&self.0.data))
    }

    #[getter]
    fn jbig2_globals<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        match &self.0.data {
            r::ImageData::Jbig2 {
                globals: Some(g), ..
            } => Some(PyBytes::new(py, g)),
            _ => None,
        }
    }

    #[getter]
    fn ccitt<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, pyo3::types::PyDict>>> {
        let r::ImageData::Ccitt { params: p, .. } = &self.0.data else {
            return Ok(None);
        };
        let d = pyo3::types::PyDict::new(py);
        d.set_item("k", p.k)?;
        d.set_item("columns", p.columns)?;
        d.set_item("rows", p.rows)?;
        d.set_item("black_is_1", p.black_is_1)?;
        d.set_item("encoded_byte_align", p.encoded_byte_align)?;
        d.set_item("end_of_line", p.end_of_line)?;
        d.set_item("end_of_block", p.end_of_block)?;
        Ok(Some(d))
    }

    #[getter]
    fn smask(&self) -> Option<SoftMask> {
        self.0.smask.clone().map(SoftMask)
    }

    #[getter]
    fn clip(&self) -> Option<Rect4> {
        self.0.clip.map(rect)
    }

    fn __repr__(&self) -> String {
        format!(
            "Image(name={:?}, width={}, height={}, kind={:?})",
            self.0.name,
            self.0.width,
            self.0.height,
            self.kind()
        )
    }
}

/// Everything extracted from one page, in content-stream order.
#[pyclass(frozen, module = "pawdf._core")]
struct PageContent {
    #[pyo3(get)]
    glyphs: Py<PyList>,
    #[pyo3(get)]
    paths: Py<PyList>,
    #[pyo3(get)]
    images: Py<PyList>,
    #[pyo3(get)]
    fonts: Py<PyList>,
    #[pyo3(get)]
    marked_contents: Py<PyList>,
    #[pyo3(get)]
    diagnostics: Py<PyList>,
}

/// One marked-content sequence and the content it covers.
#[pyclass(frozen, module = "pawdf._core")]
struct MarkedContent(r::MarkedContent);

fn range<'py>(py: Python<'py>, r: &std::ops::Range<usize>) -> PyResult<Bound<'py, PyRange>> {
    let n = |v: usize| isize::try_from(v).unwrap_or(isize::MAX);
    PyRange::new(py, n(r.start), n(r.end))
}

#[pymethods]
impl MarkedContent {
    #[getter]
    fn tag(&self) -> &str {
        &self.0.tag
    }

    #[getter]
    fn mcid(&self) -> Option<u32> {
        self.0.mcid
    }

    #[getter]
    fn actual_text(&self) -> Option<&str> {
        self.0.actual_text.as_deref()
    }

    #[getter]
    fn artifact_type(&self) -> Option<&str> {
        self.0.artifact_type.as_deref()
    }

    #[getter]
    fn artifact_subtype(&self) -> Option<&str> {
        self.0.artifact_subtype.as_deref()
    }

    #[getter]
    fn glyphs<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyRange>> {
        range(py, &self.0.glyphs)
    }

    #[getter]
    fn paths<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyRange>> {
        range(py, &self.0.paths)
    }

    #[getter]
    fn images<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyRange>> {
        range(py, &self.0.images)
    }

    fn __repr__(&self) -> String {
        format!(
            "MarkedContent(tag={:?}, mcid={:?}, glyphs=range({}, {}))",
            self.0.tag, self.0.mcid, self.0.glyphs.start, self.0.glyphs.end
        )
    }
}

fn list<T, W: pyo3::PyClass + Into<pyo3::PyClassInitializer<W>>>(
    py: Python<'_>,
    items: Vec<T>,
    wrap: fn(T) -> W,
) -> PyResult<Py<PyList>> {
    let objs = items
        .into_iter()
        .map(|x| Py::new(py, wrap(x)))
        .collect::<PyResult<Vec<_>>>()?;
    Ok(PyList::new(py, objs)?.unbind())
}

#[pymethods]
impl PageContent {
    fn __repr__(&self, py: Python<'_>) -> String {
        format!(
            "PageContent(glyphs={}, paths={}, images={}, fonts={}, marked_contents={}, diagnostics={})",
            self.glyphs.bind(py).len(),
            self.paths.bind(py).len(),
            self.images.bind(py).len(),
            self.fonts.bind(py).len(),
            self.marked_contents.bind(py).len(),
            self.diagnostics.bind(py).len()
        )
    }
}

/// A position in this document.
#[pyclass(frozen, module = "pawdf._core")]
struct Destination(r::Destination);

fn opt_float(v: Option<f64>) -> String {
    v.map_or("None".to_owned(), |v| format!("{v:?}"))
}

#[pymethods]
impl Destination {
    #[getter]
    fn page(&self) -> usize {
        self.0.page
    }

    #[getter]
    fn left(&self) -> Option<f64> {
        self.0.left
    }

    #[getter]
    fn top(&self) -> Option<f64> {
        self.0.top
    }

    fn __repr__(&self) -> String {
        format!(
            "Destination(page={}, left={}, top={})",
            self.0.page,
            opt_float(self.0.left),
            opt_float(self.0.top)
        )
    }
}

/// One outline item.
#[pyclass(frozen, module = "pawdf._core")]
struct OutlineItem {
    #[pyo3(get)]
    title: String,
    #[pyo3(get)]
    dest: Option<Py<Destination>>,
    #[pyo3(get)]
    children: Py<PyList>,
}

/// Recurses once per nesting level, which the reader caps.
fn outline_items(py: Python<'_>, items: Vec<r::OutlineItem>) -> PyResult<Py<PyList>> {
    let objs = items
        .into_iter()
        .map(|i| {
            Py::new(
                py,
                OutlineItem {
                    title: i.title,
                    dest: i.dest.map(|d| Py::new(py, Destination(d))).transpose()?,
                    children: outline_items(py, i.children)?,
                },
            )
        })
        .collect::<PyResult<Vec<_>>>()?;
    Ok(PyList::new(py, objs)?.unbind())
}

#[pymethods]
impl OutlineItem {
    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!(
            "OutlineItem(title={}, dest={}, children={})",
            self.title.clone().into_pyobject(py)?.repr()?,
            match &self.dest {
                Some(d) => d.get().__repr__(),
                None => "None".to_owned(),
            },
            self.children.bind(py).len()
        ))
    }
}

/// The document outline (bookmarks).
#[pyclass(frozen, module = "pawdf._core")]
struct Outline {
    #[pyo3(get)]
    items: Py<PyList>,
    #[pyo3(get)]
    diagnostics: Py<PyList>,
}

#[pymethods]
impl Outline {
    fn __repr__(&self, py: Python<'_>) -> String {
        format!(
            "Outline(items={}, diagnostics={})",
            self.items.bind(py).len(),
            self.diagnostics.bind(py).len()
        )
    }
}

/// One annotation. Its appearance is not read.
#[pyclass(frozen, module = "pawdf._core")]
struct Annotation(r::Annotation);

type Point2 = (f64, f64);
type Quad = (Point2, Point2, Point2, Point2);

#[pymethods]
impl Annotation {
    #[getter]
    fn subtype(&self) -> &str {
        &self.0.subtype
    }

    #[getter]
    fn rect(&self) -> Rect4 {
        rect(self.0.rect)
    }

    #[getter]
    fn contents(&self) -> Option<&str> {
        self.0.contents.as_deref()
    }

    #[getter]
    fn author(&self) -> Option<&str> {
        self.0.author.as_deref()
    }

    #[getter]
    fn name(&self) -> Option<&str> {
        self.0.name.as_deref()
    }

    #[getter]
    fn flags(&self) -> u32 {
        self.0.flags
    }

    #[getter]
    fn quad_points(&self) -> Vec<Quad> {
        self.0
            .quad_points
            .iter()
            .map(|q| {
                let [a, b, c, d] = q.map(|p| (p.x, p.y));
                (a, b, c, d)
            })
            .collect()
    }

    #[getter]
    fn dest(&self) -> Option<Destination> {
        self.0.dest.clone().map(Destination)
    }

    #[getter]
    fn uri(&self) -> Option<&str> {
        self.0.uri.as_deref()
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!(
            "Annotation(subtype={}, rect={:?})",
            self.subtype().into_pyobject(py)?.repr()?,
            self.rect()
        ))
    }
}

/// The annotations of one page.
#[pyclass(frozen, module = "pawdf._core")]
struct Annotations {
    #[pyo3(get)]
    items: Py<PyList>,
    #[pyo3(get)]
    diagnostics: Py<PyList>,
}

#[pymethods]
impl Annotations {
    fn __repr__(&self, py: Python<'_>) -> String {
        format!(
            "Annotations(items={}, diagnostics={})",
            self.items.bind(py).len(),
            self.diagnostics.bind(py).len()
        )
    }
}

/// One widget annotation of a form field.
#[pyclass(frozen, module = "pawdf._core")]
struct Widget(r::Widget);

#[pymethods]
impl Widget {
    #[getter]
    fn page(&self) -> Option<usize> {
        self.0.page
    }

    #[getter]
    fn rect(&self) -> Rect4 {
        rect(self.0.rect)
    }

    fn __repr__(&self) -> String {
        let page = self.0.page.map_or("None".to_owned(), |p| p.to_string());
        format!("Widget(page={page}, rect={:?})", self.rect())
    }
}

/// One terminal form field. Its appearance is not read.
#[pyclass(frozen, module = "pawdf._core")]
struct FormField(r::FormField);

#[pymethods]
impl FormField {
    #[getter]
    fn name(&self) -> &str {
        &self.0.name
    }

    #[getter]
    fn label(&self) -> Option<&str> {
        self.0.label.as_deref()
    }

    #[getter]
    fn kind(&self) -> &'static str {
        match self.0.kind {
            r::FieldKind::Text => "text",
            r::FieldKind::CheckBox => "checkbox",
            r::FieldKind::RadioButton => "radio",
            r::FieldKind::PushButton => "push_button",
            r::FieldKind::ComboBox => "combo_box",
            r::FieldKind::ListBox => "list_box",
            r::FieldKind::Signature => "signature",
            r::FieldKind::Unknown => "unknown",
        }
    }

    #[getter]
    fn values(&self) -> Vec<String> {
        self.0.values.clone()
    }

    #[getter]
    fn flags(&self) -> u32 {
        self.0.flags
    }

    #[getter]
    fn widgets(&self) -> Vec<Widget> {
        self.0.widgets.iter().cloned().map(Widget).collect()
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!(
            "FormField(name={}, kind={}, values={})",
            self.name().into_pyobject(py)?.repr()?,
            self.kind().into_pyobject(py)?.repr()?,
            self.values().into_pyobject(py)?.repr()?
        ))
    }
}

/// The form fields of a document.
#[pyclass(frozen, module = "pawdf._core")]
struct FormFields {
    #[pyo3(get)]
    items: Py<PyList>,
    #[pyo3(get)]
    diagnostics: Py<PyList>,
}

#[pymethods]
impl FormFields {
    fn __repr__(&self, py: Python<'_>) -> String {
        format!(
            "FormFields(items={}, diagnostics={})",
            self.items.bind(py).len(),
            self.diagnostics.bind(py).len()
        )
    }
}

/// One page of a document.
#[pyclass(frozen, module = "pawdf._core")]
struct Page(r::Page);

#[pymethods]
impl Page {
    #[getter]
    fn index(&self) -> usize {
        self.0.index()
    }

    #[getter]
    fn media_box(&self) -> Rect4 {
        rect(self.0.media_box())
    }

    #[getter]
    fn crop_box(&self) -> Rect4 {
        rect(self.0.crop_box())
    }

    #[getter]
    fn rotate(&self) -> u16 {
        self.0.rotate().degrees()
    }

    #[getter]
    fn display_matrix(&self) -> Matrix6 {
        matrix(self.0.display_matrix())
    }

    /// Extracts glyphs, painted paths, and images. Releases the GIL while parsing.
    fn content(&self, py: Python<'_>) -> PyResult<PageContent> {
        let c = py.detach(|| self.0.content());
        Ok(PageContent {
            glyphs: list(py, c.glyphs, Glyph)?,
            paths: list(py, c.paths, Path)?,
            images: list(py, c.images, Image)?,
            fonts: list(py, c.fonts, FontInfo)?,
            marked_contents: list(py, c.marked_contents, MarkedContent)?,
            diagnostics: diagnostics(py, c.diagnostics)?,
        })
    }

    /// The page's annotations, read on every call. Releases the GIL while parsing.
    fn annotations(&self, py: Python<'_>) -> PyResult<Annotations> {
        let a = py.detach(|| self.0.annotations());
        Ok(Annotations {
            items: list(py, a.items, Annotation)?,
            diagnostics: diagnostics(py, a.diagnostics)?,
        })
    }

    fn __repr__(&self) -> String {
        format!(
            "Page(index={}, media_box={:?}, rotate={})",
            self.index(),
            self.media_box(),
            self.rotate()
        )
    }
}

/// The permission flags `/P` of an encrypted PDF. Reported, not enforced.
#[pyclass(frozen, module = "pawdf._core")]
struct Permissions(r::Permissions);

#[pymethods]
impl Permissions {
    #[getter]
    fn bits(&self) -> u32 {
        self.0.bits()
    }

    #[getter]
    fn print(&self) -> bool {
        self.0.print()
    }

    #[getter]
    fn modify(&self) -> bool {
        self.0.modify()
    }

    #[getter]
    fn copy(&self) -> bool {
        self.0.copy()
    }

    #[getter]
    fn annotate(&self) -> bool {
        self.0.annotate()
    }

    #[getter]
    fn fill_forms(&self) -> bool {
        self.0.fill_forms()
    }

    #[getter]
    fn extract_for_accessibility(&self) -> bool {
        self.0.extract_for_accessibility()
    }

    #[getter]
    fn assemble(&self) -> bool {
        self.0.assemble()
    }

    #[getter]
    fn print_high_quality(&self) -> bool {
        self.0.print_high_quality()
    }

    fn __repr__(&self) -> String {
        format!("Permissions(bits={:#010x})", self.0.bits())
    }
}

/// How an encrypted PDF was opened.
#[pyclass(frozen, module = "pawdf._core")]
struct Encryption(r::Encryption);

#[pymethods]
impl Encryption {
    #[getter]
    fn revision(&self) -> u8 {
        self.0.revision
    }

    #[getter]
    fn method(&self) -> &'static str {
        match self.0.method {
            r::Method::Identity => "identity",
            r::Method::Rc4 => "rc4",
            r::Method::Aes128 => "aes-128",
            r::Method::Aes256 => "aes-256",
        }
    }

    #[getter]
    fn key_bits(&self) -> u16 {
        self.0.key_bits
    }

    #[getter]
    fn permissions(&self) -> Permissions {
        Permissions(self.0.permissions)
    }

    #[getter]
    fn authenticated_as(&self) -> &'static str {
        match self.0.authenticated_as {
            r::PasswordKind::User => "user",
            r::PasswordKind::Owner => "owner",
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "Encryption(revision={}, method='{}', key_bits={}, authenticated_as='{}')",
            self.0.revision,
            self.method(),
            self.0.key_bits,
            self.authenticated_as()
        )
    }
}

/// An open PDF document.
#[pyclass(frozen, module = "pawdf._core")]
struct Document(r::Document);

#[pymethods]
impl Document {
    #[new]
    #[pyo3(signature = (data, password=None))]
    fn new(py: Python<'_>, data: Vec<u8>, password: Option<String>) -> PyResult<Self> {
        py.detach(|| match password {
            Some(pw) => r::Document::from_bytes_with_password(data, &pw),
            None => r::Document::from_bytes(data),
        })
        .map(Document)
        .map_err(to_py_err)
    }

    #[getter]
    fn encryption(&self) -> Option<Encryption> {
        self.0.encryption().map(Encryption)
    }

    fn __len__(&self) -> usize {
        self.0.page_count()
    }

    fn __getitem__(&self, index: &Bound<'_, PyAny>) -> PyResult<Page> {
        let count = self.0.page_count();
        let out_of_range =
            || PyIndexError::new_err(format!("page index {index} out of range ({count} pages)"));
        let index: isize = match index.extract() {
            Ok(i) => i,
            Err(e) if e.is_instance_of::<PyOverflowError>(index.py()) => {
                return Err(out_of_range());
            }
            Err(e) => return Err(e),
        };
        let resolved = if index < 0 {
            count.checked_sub(index.unsigned_abs())
        } else {
            Some(index.unsigned_abs())
        };
        let Some(i) = resolved else {
            return Err(out_of_range());
        };
        self.0.page(i).map(Page).map_err(to_py_err)
    }

    #[getter]
    fn diagnostics(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        diagnostics(py, self.0.diagnostics())
    }

    /// The document outline (bookmarks). Read on every call. Releases the GIL while parsing.
    fn outline(&self, py: Python<'_>) -> PyResult<Outline> {
        let o = py.detach(|| self.0.outline());
        Ok(Outline {
            items: outline_items(py, o.items)?,
            diagnostics: diagnostics(py, o.diagnostics)?,
        })
    }

    /// The interactive form fields. Read on every call. Releases the GIL while parsing.
    fn form_fields(&self, py: Python<'_>) -> PyResult<FormFields> {
        let f = py.detach(|| self.0.form_fields());
        Ok(FormFields {
            items: list(py, f.items, FormField)?,
            diagnostics: diagnostics(py, f.diagnostics)?,
        })
    }

    fn __repr__(&self) -> String {
        format!("Document(pages={})", self.0.page_count())
    }
}

/// A Python module implemented in Rust. The name of this module must match
/// the `lib.name` setting in the `Cargo.toml`, else Python will not be able to
/// import the module.
#[pymodule]
mod _core {
    use pyo3::prelude::*;

    #[pymodule_export]
    use super::{
        Annotation, Annotations, Color, Destination, Diagnostic, Document, EncryptedError,
        Encryption, FontInfo, FormField, FormFields, Glyph, Image, MarkedContent, Outline,
        OutlineItem, Page, PageContent, PasswordError, Path, PdfError, Permissions, SoftMask,
        UnsupportedEncryptionError, Widget,
    };

    #[pyfunction]
    fn hello_from_bin() -> String {
        "Hello from pawdf!".to_string()
    }
}
