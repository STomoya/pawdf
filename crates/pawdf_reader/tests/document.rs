mod common;

use common::{CATALOG, build_pdf};
use pawdf_core::DiagnosticKind;
use pawdf_reader::{Document, ErrorKind, Method, Page, Point, Rect, Rotation};

fn open(objects: &[(u32, &[u8])]) -> Document {
    let (pdf, _) = build_pdf(objects, "/Root 1 0 R");
    Document::from_bytes(pdf).expect("opens")
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Rect {
    Rect { x0, y0, x1, y1 }
}

fn count(doc: &Document, kind: DiagnosticKind) -> usize {
    doc.diagnostics().iter().filter(|d| d.kind == kind).count()
}

fn single_page(page_attrs: &str) -> Document {
    let page = format!("<< /Type /Page /Parent 2 0 R {page_attrs} >>");
    open(&[
        (1, CATALOG),
        (2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        (3, page.as_bytes()),
    ])
}

#[test]
fn inherited_attributes() {
    let doc = open(&[
        (1, CATALOG),
        (
            2,
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 100] /Rotate 90 /Resources << /Font << >> >> >>",
        ),
        (3, b"<< /Type /Page /Parent 2 0 R >>"),
    ]);
    assert_eq!(doc.page_count(), 1);
    let p = doc.page(0).unwrap();
    assert_eq!(p.media_box(), rect(0., 0., 200., 100.));
    assert_eq!(p.crop_box(), p.media_box());
    assert_eq!(p.rotate(), Rotation::R90);
    assert!(doc.diagnostics().is_empty());
}

#[test]
fn boxes_are_normalized_and_crop_box_is_read() {
    let doc = single_page("/MediaBox [612 792 0 0] /CropBox [10 10 100 100]");
    let p = doc.page(0).unwrap();
    assert_eq!(p.media_box(), rect(0., 0., 612., 792.));
    assert_eq!(p.crop_box(), rect(10., 10., 100., 100.));
}

#[test]
fn rotate_is_normalized() {
    let r = |attr: &str| single_page(&format!("/MediaBox [0 0 1 1] {attr}"));
    assert_eq!(r("/Rotate -90").page(0).unwrap().rotate(), Rotation::R270);
    assert_eq!(r("/Rotate 450").page(0).unwrap().rotate(), Rotation::R90);
    let bad = r("/Rotate 45");
    assert_eq!(bad.page(0).unwrap().rotate(), Rotation::R0);
    assert_eq!(count(&bad, DiagnosticKind::BadPageTree), 1);
}

#[test]
fn missing_media_box_defaults_to_letter() {
    let doc = single_page("");
    assert_eq!(doc.page(0).unwrap().media_box(), rect(0., 0., 612., 792.));
    assert_eq!(count(&doc, DiagnosticKind::BadPageTree), 1);
}

#[test]
fn display_matrix_maps_crop_box_corners() {
    let page = |rot: i32| -> Page {
        single_page(&format!("/MediaBox [0 0 200 100] /Rotate {rot}"))
            .page(0)
            .unwrap()
    };
    let check = |p: &Page, from: (f64, f64), to: (f64, f64)| {
        let got = p.display_matrix().apply(Point {
            x: from.0,
            y: from.1,
        });
        assert_eq!((got.x, got.y), to, "rotate {:?} {from:?}", p.rotate());
    };
    let p = page(0);
    check(&p, (0., 100.), (0., 0.));
    check(&p, (200., 0.), (200., 100.));
    let p = page(90);
    check(&p, (0., 0.), (0., 0.));
    check(&p, (200., 0.), (0., 200.));
    check(&p, (0., 100.), (100., 0.));
    let p = page(180);
    check(&p, (200., 0.), (0., 0.));
    check(&p, (0., 100.), (200., 100.));
    let p = page(270);
    check(&p, (200., 100.), (0., 0.));
    check(&p, (0., 0.), (100., 200.));
}

#[test]
fn page_tree_cycle_is_cut() {
    let doc = open(&[
        (1, CATALOG),
        (2, b"<< /Type /Pages /Kids [3 0 R 2 0 R] /Count 99 >>"),
        (3, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 1 1] >>"),
    ]);
    assert_eq!(doc.page_count(), 1);
    assert_eq!(count(&doc, DiagnosticKind::BadPageTree), 1);
}

#[test]
fn count_is_ignored_and_order_is_preserved() {
    let doc = open(&[
        (1, CATALOG),
        (2, b"<< /Type /Pages /Kids [3 0 R 6 0 R] /Count 1 >>"),
        (3, b"<< /Type /Pages /Kids [4 0 R 5 0 R] /Parent 2 0 R >>"),
        (4, b"<< /Type /Page /MediaBox [0 0 1 1] >>"),
        (5, b"<< /Type /Page /MediaBox [0 0 2 2] >>"),
        (6, b"<< /Type /Page /MediaBox [0 0 3 3] >>"),
    ]);
    let sizes: Vec<f64> = (0..doc.page_count())
        .map(|i| doc.page(i).unwrap().media_box().x1)
        .collect();
    assert_eq!(sizes, [1., 2., 3.]);
}

#[test]
fn non_reference_kids_are_dropped() {
    let doc = open(&[
        (1, CATALOG),
        (2, b"<< /Type /Pages /Kids [3 0 R 42 << /Type /Page >>] >>"),
        (3, b"<< /Type /Page /MediaBox [0 0 1 1] >>"),
    ]);
    assert_eq!(doc.page_count(), 1);
    assert_eq!(count(&doc, DiagnosticKind::BadPageTree), 2);
}

#[test]
fn pages_node_without_kids_is_diagnosed() {
    let doc = open(&[
        (1, CATALOG),
        (2, b"<< /Type /Pages /Kids [3 0 R 4 0 R] >>"),
        (3, b"<< /Type /Page /MediaBox [0 0 1 1] >>"),
        (4, b"<< /Type /Pages /MediaBox [0 0 1 1] >>"),
    ]);
    assert_eq!(doc.page_count(), 2);
    assert_eq!(count(&doc, DiagnosticKind::BadPageTree), 1);
}

#[test]
fn page_out_of_range() {
    let doc = single_page("/MediaBox [0 0 1 1]");
    let err = doc.page(5).unwrap_err();
    assert_eq!(err.kind, ErrorKind::PageOutOfRange { index: 5, count: 1 });
}

#[test]
fn encrypted_is_core_error() {
    let (pdf, _) = build_pdf(
        &[(1, CATALOG), (2, b"<< /Type /Pages /Kids [] >>")],
        "/Root 1 0 R /Encrypt << >>",
    );
    let err = Document::from_bytes(pdf).expect_err("fails");
    assert!(
        matches!(err.kind, ErrorKind::Core(ref e) if matches!(e.kind, pawdf_core::ErrorKind::UnsupportedEncryption(_)))
    );
}

#[test]
fn document_is_send_sync() {
    fn f<T: Send + Sync>() {}
    f::<Document>();
    f::<Page>();
}

#[test]
fn catalog_without_pages_has_no_pages() {
    let doc = open(&[(1, b"<< /Type /Catalog >>")]);
    assert_eq!(doc.page_count(), 0);
    assert_eq!(count(&doc, DiagnosticKind::BadPageTree), 1);
}

#[test]
fn kid_that_is_not_a_dictionary_is_dropped() {
    let doc = open(&[
        (1, CATALOG),
        (2, b"<< /Type /Pages /Kids [3 0 R] >>"),
        (3, b"42"),
    ]);
    assert_eq!(doc.page_count(), 0);
    assert_eq!(count(&doc, DiagnosticKind::BadPageTree), 1);
}

#[test]
fn bad_crop_box_falls_back_to_media_box() {
    let doc = single_page("/MediaBox [0 0 10 10] /CropBox [0 0 x 1]");
    assert_eq!(doc.page(0).unwrap().crop_box(), rect(0., 0., 10., 10.));
    assert_eq!(count(&doc, DiagnosticKind::BadPageTree), 1);
}

#[test]
fn crop_box_is_clipped_to_media_box() {
    let doc = single_page("/MediaBox [0 0 100 100] /CropBox [-10 50 50 200]");
    assert_eq!(doc.page(0).unwrap().crop_box(), rect(0., 50., 50., 100.));
    assert!(doc.diagnostics().is_empty());
    let doc = single_page("/MediaBox [0 0 10 10] /CropBox [20 20 30 30]");
    assert_eq!(doc.page(0).unwrap().crop_box(), rect(0., 0., 10., 10.));
    assert_eq!(count(&doc, DiagnosticKind::BadPageTree), 1);
}

#[test]
fn errors_display() {
    let doc = single_page("/MediaBox [0 0 1 1]");
    assert_eq!(
        doc.page(3).unwrap_err().to_string(),
        "page index 3 out of range (1 pages)"
    );
    let (pdf, _) = build_pdf(&[(1, CATALOG)], "/Root 1 0 R /Encrypt << >>");
    let err = Document::from_bytes(pdf).expect_err("encrypted");
    assert!(
        err.to_string()
            .starts_with("unsupported encryption: missing /Filter")
    );
    assert!(std::error::Error::source(&err).is_some());
    assert!(format!("{doc:?}").starts_with("Document"));
}

/// SHA-256(b"" ‖ 8 × 0x01) ‖ 8 × 0x01 ‖ 8 × 0x02: an R5 `/U` for the empty user password.
const R5_EMPTY_U: &str = "04abc8821a06e5a30937967d11ad10221cb5ac3b5273e434f1284ee87129a06101010101010101010202020202020202";

#[test]
fn opens_encrypted_document() {
    let dict = format!(
        "<< /Filter /Standard /V 5 /R 5 /Length 256 /P -3904 /O <{0}> /U <{0}> /OE <{1}> /UE <{1}> \
         /CF << /StdCF << /CFM /AESV3 >> >> /StmF /Identity /StrF /Identity >>",
        R5_EMPTY_U,
        "00".repeat(32)
    );
    let (pdf, _) = build_pdf(
        &[
            (1, CATALOG),
            (2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
            (3, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 1 1] >>"),
            (4, dict.as_bytes()),
        ],
        "/Root 1 0 R /Encrypt 4 0 R",
    );
    let doc = Document::from_bytes(pdf.clone()).unwrap();
    assert_eq!(doc.page_count(), 1);
    let enc = doc.encryption().unwrap();
    assert_eq!((enc.method, enc.revision), (Method::Identity, 5));
    assert!(Document::from_bytes_with_password(pdf, "anything").is_ok());
}
