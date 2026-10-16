use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use pawdf_core::{Diagnostic, DiagnosticKind, Diagnostics, File, Location, Object, Ref};

use crate::error::{Error, ErrorKind};
use crate::font::{Font, load_font};
use crate::geom::{Matrix, Rect, Rotation};

const MAX_TREE_DEPTH: usize = 64;

const LETTER: Rect = Rect {
    x0: 0.,
    y0: 0.,
    x1: 612.,
    y1: 792.,
};

/// An open PDF document. Cheap to clone; clones share the parsed file and caches.
#[derive(Clone)]
pub struct Document(Arc<Inner>);

/// A parsed font and the diagnostics loading it gave.
type LoadedFont = (Arc<Font>, Vec<Diagnostic>);

pub(crate) struct Inner {
    pub file: File,
    pub pages: Vec<PageInfo>,
    pub tree_diags: Vec<Diagnostic>,
    /// Fonts by object, parsed once per document, with the diagnostics loading them gave.
    fonts: Mutex<HashMap<Ref, LoadedFont>>,
}

impl Inner {
    /// The font at `r`, loading it on first use, plus the diagnostics its loading gave,
    /// so every pass that uses it can report them.
    pub fn font(&self, r: Ref, at: Location) -> LoadedFont {
        let mut fonts = self.fonts.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((f, d)) = fonts.get(&r) {
            return (Arc::clone(f), d.clone());
        }
        let mut diags = Diagnostics::new();
        let font = match self.file.resolve(r).as_dict() {
            Some(dict) => load_font(&self.file, dict, at, &mut diags),
            None => {
                diags.push(DiagnosticKind::BadFont, at);
                Font::fallback()
            }
        };
        let entry = (Arc::new(font), diags.into_vec());
        fonts.insert(r, entry.clone());
        entry
    }
}

#[derive(Clone)]
pub(crate) struct PageInfo {
    pub obj: Ref,
    pub resources: Option<Arc<Object>>,
    pub media_box: Rect,
    pub crop_box: Rect,
    pub rotate: Rotation,
}

impl Document {
    /// Opens a document from its bytes and walks the page tree.
    ///
    /// Fails only when `pawdf_core` cannot open the file (encrypted, or no catalog).
    pub fn from_bytes(data: Vec<u8>) -> Result<Document, Error> {
        let file = File::from_bytes(data)?;
        let mut diags = Diagnostics::new();
        let pages = walk_pages(&file, &mut diags);
        Ok(Document(Arc::new(Inner {
            file,
            pages,
            tree_diags: diags.into_vec(),
            fonts: Mutex::new(HashMap::new()),
        })))
    }

    /// The number of pages.
    pub fn page_count(&self) -> usize {
        self.0.pages.len()
    }

    /// The page at `index` (0-based).
    pub fn page(&self, index: usize) -> Result<Page, Error> {
        match self.0.pages.get(index) {
            Some(info) => Ok(Page {
                index,
                info: info.clone(),
                doc: Arc::clone(&self.0),
            }),
            None => Err(Error {
                kind: ErrorKind::PageOutOfRange {
                    index,
                    count: self.0.pages.len(),
                },
            }),
        }
    }

    /// Structural diagnostics: file-level recoveries plus page-tree problems.
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        let mut out = self.0.file.diagnostics();
        out.extend_from_slice(&self.0.tree_diags);
        out
    }
}

/// One page of a [`Document`].
#[derive(Clone)]
pub struct Page {
    index: usize,
    pub(crate) info: PageInfo,
    pub(crate) doc: Arc<Inner>,
}

impl Page {
    /// The 0-based page index.
    pub fn index(&self) -> usize {
        self.index
    }

    /// `/MediaBox` (inherited), or US Letter when missing.
    pub fn media_box(&self) -> Rect {
        self.info.media_box
    }

    /// `/CropBox` (inherited), defaulting to the media box.
    pub fn crop_box(&self) -> Rect {
        self.info.crop_box
    }

    /// `/Rotate` (inherited), normalized to a multiple of 90°.
    pub fn rotate(&self) -> Rotation {
        self.info.rotate
    }

    /// Maps user space to display space: origin at the top-left of the rotated crop box,
    /// y pointing down, in points.
    pub fn display_matrix(&self) -> Matrix {
        let Rect { x0, y0, x1, y1 } = self.info.crop_box;
        let [a, b, c, d, e, f] = match self.info.rotate {
            Rotation::R0 => [1., 0., 0., -1., -x0, y1],
            Rotation::R90 => [0., 1., 1., 0., -y0, -x0],
            Rotation::R180 => [-1., 0., 0., 1., x1, -y0],
            Rotation::R270 => [0., -1., -1., 0., y1, x1],
        };
        Matrix { a, b, c, d, e, f }
    }
}

#[derive(Clone, Default)]
struct Inherited {
    resources: Option<Arc<Object>>,
    media_box: Option<Arc<Object>>,
    crop_box: Option<Arc<Object>>,
    rotate: Option<Arc<Object>>,
}

/// Collects leaf pages in document order. `/Count` is ignored because writers get it wrong;
/// the `/Kids` walk is authoritative.
fn walk_pages(file: &File, diags: &mut Diagnostics) -> Vec<PageInfo> {
    let mut pages = Vec::new();
    let bad = |diags: &mut Diagnostics| diags.push(DiagnosticKind::BadPageTree, Location::File(0));
    let root = file
        .trailer()
        .get(b"Root")
        .map(|o| file.deref(o))
        .and_then(Object::as_dict)
        .and_then(|c| c.get(b"Pages"))
        .and_then(Object::as_ref);
    let Some(root) = root else {
        bad(diags);
        return pages;
    };
    let mut visited = HashSet::new();
    let mut stack = vec![(root, Arc::new(Inherited::default()), 0usize)];
    while let Some((r, parent, depth)) = stack.pop() {
        if depth > MAX_TREE_DEPTH || !visited.insert(r.num) {
            bad(diags);
            continue;
        }
        let Some(node) = file.resolve(r).as_dict() else {
            bad(diags);
            continue;
        };
        let mut inherited = Inherited::clone(&parent);
        // §7.7.3.4: these four attributes are inheritable.
        for (key, slot) in [
            (b"Resources".as_slice(), &mut inherited.resources),
            (b"MediaBox", &mut inherited.media_box),
            (b"CropBox", &mut inherited.crop_box),
            (b"Rotate", &mut inherited.rotate),
        ] {
            if let Some(v) = node.get(key) {
                // Shared, so a node with many kids does not copy a large /Resources per kid.
                *slot = Some(Arc::new(file.deref(v).clone()));
            }
        }
        let kids = node.get(b"Kids").map(|k| file.deref(k));
        let is_page = node.get(b"Type").and_then(Object::as_name) == Some(b"Page");
        match kids.and_then(Object::as_array) {
            Some(kids) if !is_page => {
                let inherited = Arc::new(inherited);
                for kid in kids.iter().rev() {
                    match kid.as_ref() {
                        Some(k) => stack.push((k, Arc::clone(&inherited), depth + 1)),
                        None => bad(diags),
                    }
                }
            }
            _ => pages.push(page_info(file, r, inherited, diags)),
        }
    }
    pages
}

fn page_info(file: &File, obj: Ref, inh: Inherited, diags: &mut Diagnostics) -> PageInfo {
    let mut bad = || diags.push(DiagnosticKind::BadPageTree, Location::File(0));
    let media_box = match inh.media_box.map(|b| rect(file, &b)) {
        Some(Some(r)) => r,
        _ => {
            bad();
            LETTER
        }
    };
    let crop_box = match inh.crop_box.map(|b| rect(file, &b)) {
        Some(Some(r)) => r,
        Some(None) => {
            bad();
            media_box
        }
        None => media_box,
    };
    let rotate = match inh.rotate.as_ref().map(|o| o.as_int()) {
        None => Rotation::R0,
        Some(Some(n)) if n % 90 == 0 => match n.rem_euclid(360) {
            90 => Rotation::R90,
            180 => Rotation::R180,
            270 => Rotation::R270,
            _ => Rotation::R0,
        },
        Some(_) => {
            bad();
            Rotation::R0
        }
    };
    PageInfo {
        obj,
        resources: inh.resources,
        media_box,
        crop_box,
        rotate,
    }
}

fn rect(file: &File, obj: &Object) -> Option<Rect> {
    let [x0, y0, x1, y1] = numbers::<4>(file, obj)?;
    Some(Rect::from_corners(x0, y0, x1, y1))
}

/// Exactly `N` finite numbers from an array, resolving indirect elements.
pub(crate) fn numbers<const N: usize>(file: &File, obj: &Object) -> Option<[f64; N]> {
    let items = file.deref(obj).as_array()?;
    if items.len() != N {
        return None;
    }
    let mut out = [0.; N];
    for (slot, item) in out.iter_mut().zip(items) {
        *slot = file.deref(item).as_number().filter(|n| n.is_finite())?;
    }
    Some(out)
}

impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document")
            .field("pages", &self.page_count())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for Page {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Page")
            .field("index", &self.index)
            .field("media_box", &self.info.media_box)
            .field("rotate", &self.info.rotate)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::build_pdf;

    #[test]
    fn inherited_resources_are_shared_not_copied() {
        let pdf = build_pdf(&[
            (1, b"<< /Type /Catalog /Pages 2 0 R >>"),
            (2, b"<< /Type /Pages /Kids [3 0 R 4 0 R 3 0 R 3 0 R] /Resources << /Font << /F1 << /Type /Font >> >> >> >>"),
            (3, b"<< /Type /Page >>"),
            (4, b"<< /Type /Page >>"),
        ]);
        let doc = Document::from_bytes(pdf).unwrap();
        assert_eq!(doc.page_count(), 2);
        let (a, b) = (doc.page(0).unwrap(), doc.page(1).unwrap());
        let (Some(ra), Some(rb)) = (&a.info.resources, &b.info.resources) else {
            panic!("resources inherited");
        };
        assert!(Arc::ptr_eq(ra, rb));
    }

    #[test]
    fn font_cache_reuses_by_ref() {
        let pdf = build_pdf(&[
            (1, b"<< /Type /Catalog /Pages 2 0 R >>"),
            (2, b"<< /Type /Pages /Kids [3 0 R 4 0 R] >>"),
            (
                3,
                b"<< /Type /Page /Resources << /Font << /F1 7 0 R >> >> >>",
            ),
            (
                4,
                b"<< /Type /Page /Resources << /Font << /F1 7 0 R >> >> >>",
            ),
            (7, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"),
        ]);
        let doc = Document::from_bytes(pdf).unwrap();
        let r = Ref {
            num: 7,
            generation: 0,
        };
        let (a, _) = doc.0.font(r, Location::File(0));
        let (b, _) = doc.0.font(r, Location::File(0));
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(a.info.base_font, "Helvetica");
    }
}
