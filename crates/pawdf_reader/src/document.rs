use std::collections::HashSet;
use std::sync::Arc;

use pawdf_core::{Diagnostic, DiagnosticKind, Diagnostics, File, Location, Object, Ref};

use crate::error::{Error, ErrorKind};
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

pub(crate) struct Inner {
    pub file: File,
    pub pages: Vec<PageInfo>,
    pub tree_diags: Vec<Diagnostic>,
}

#[derive(Clone)]
pub(crate) struct PageInfo {
    pub obj: Ref,
    pub resources: Option<Object>,
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
    resources: Option<Object>,
    media_box: Option<Object>,
    crop_box: Option<Object>,
    rotate: Option<Object>,
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
    let mut stack = vec![(root, Inherited::default(), 0usize)];
    while let Some((r, mut inherited, depth)) = stack.pop() {
        if depth > MAX_TREE_DEPTH || !visited.insert(r.num) {
            bad(diags);
            continue;
        }
        let Some(node) = file.resolve(r).as_dict() else {
            bad(diags);
            continue;
        };
        // §7.7.3.4: these four attributes are inheritable.
        for (key, slot) in [
            (b"Resources".as_slice(), &mut inherited.resources),
            (b"MediaBox", &mut inherited.media_box),
            (b"CropBox", &mut inherited.crop_box),
            (b"Rotate", &mut inherited.rotate),
        ] {
            if let Some(v) = node.get(key) {
                *slot = Some(file.deref(v).clone());
            }
        }
        let kids = node.get(b"Kids").map(|k| file.deref(k));
        let is_page = node.get(b"Type").and_then(Object::as_name) == Some(b"Page");
        match kids.and_then(Object::as_array) {
            Some(kids) if !is_page => {
                for kid in kids.iter().rev() {
                    match kid.as_ref() {
                        Some(k) => stack.push((k, inherited.clone(), depth + 1)),
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
