mod common;

use common::{CATALOG, PAGES, build_pdf, build_pdf_eol};
use miniz_oxide::deflate::compress_to_vec_zlib;
use pawdf_core::{DiagnosticKind, ErrorKind, File, Location, Object, Ref};

fn r(num: u32) -> Ref {
    Ref { num, generation: 0 }
}

fn kinds(file: &File) -> Vec<DiagnosticKind> {
    file.diagnostics().iter().map(|d| d.kind).collect()
}

#[test]
fn resolves_objects_from_classic_xref() {
    let (pdf, _) = build_pdf(&[(1, CATALOG), (2, PAGES), (3, b"(hello)")], "/Root 1 0 R");
    let file = File::from_bytes(pdf).unwrap();
    assert_eq!(file.trailer().get(b"Root"), Some(&Object::Ref(r(1))));
    assert_eq!(file.resolve(r(3)), &Object::String(b"hello".to_vec()));
    let catalog = file.resolve(r(1)).as_dict().unwrap();
    assert_eq!(
        catalog.get(b"Type").unwrap().as_name(),
        Some(&b"Catalog"[..])
    );
    assert_eq!(file.objects(), vec![r(1), r(2), r(3)]);
    assert!(file.diagnostics().is_empty());
}

#[test]
fn missing_object_is_null_with_diagnostic() {
    let (pdf, _) = build_pdf(&[(1, CATALOG), (2, PAGES)], "/Root 1 0 R");
    let file = File::from_bytes(pdf).unwrap();
    assert_eq!(file.resolve(r(9)), &Object::Null);
    let d = file.diagnostics();
    assert_eq!(d[0].kind, DiagnosticKind::ObjectNotFound);
    assert_eq!(d[0].location, Location::Object(r(9)));
}

#[test]
fn reference_chains_are_followed_and_cycles_cut() {
    let (pdf, _) = build_pdf(
        &[
            (1, CATALOG),
            (2, PAGES),
            (3, b"4 0 R"),
            (4, b"42"),
            (5, b"5 0 R"),
        ],
        "/Root 1 0 R",
    );
    let file = File::from_bytes(pdf).unwrap();
    assert_eq!(file.resolve(r(3)), &Object::Int(42));
    assert_eq!(file.resolve(r(5)), &Object::Null);
    let d = file.diagnostics();
    assert_eq!(d[0].kind, DiagnosticKind::RecursionLimit);
    assert_eq!(d[0].location, Location::Object(r(5)));
}

#[test]
fn indirect_length_and_filter_are_resolved() {
    let body = compress_to_vec_zlib(b"BT ET", 6);
    let mut stream = b"<< /Length 4 0 R /Filter 5 0 R >>\nstream\n".to_vec();
    stream.extend_from_slice(&body);
    stream.extend_from_slice(b"\nendstream");
    let len = body.len().to_string();
    let (pdf, _) = build_pdf(
        &[
            (1, CATALOG),
            (2, PAGES),
            (3, &stream),
            (4, len.as_bytes()),
            (5, b"/FlateDecode"),
        ],
        "/Root 1 0 R",
    );
    let file = File::from_bytes(pdf).unwrap();
    let s = file.resolve(r(3)).as_stream().unwrap();
    assert_eq!(s.raw(), &body[..]);
    let mut d = pawdf_core::Diagnostics::new();
    assert_eq!(s.decode(Location::File(0), &mut d).data, b"BT ET");
    assert!(file.diagnostics().is_empty());
    assert!(d.is_empty());
}

#[test]
fn indirect_decode_parms_values_are_resolved() {
    // PNG Up rows of 2 columns: [1, 1] then [1, 1] + [1, 1].
    let body = compress_to_vec_zlib(&[2, 1, 1, 2, 1, 1], 6);
    let mut stream = format!(
        "<< /Length {} /Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 4 0 R >> >>\nstream\n",
        body.len()
    )
    .into_bytes();
    stream.extend_from_slice(&body);
    stream.extend_from_slice(b"\nendstream");
    let (pdf, _) = build_pdf(
        &[(1, CATALOG), (2, PAGES), (3, &stream), (4, b"2")],
        "/Root 1 0 R",
    );
    let file = File::from_bytes(pdf).unwrap();
    let s = file.resolve(r(3)).as_stream().unwrap();
    let mut d = pawdf_core::Diagnostics::new();
    assert_eq!(s.decode(Location::File(0), &mut d).data, [1, 1, 2, 2]);
    assert!(file.diagnostics().is_empty());
    assert!(d.is_empty());
}

#[test]
fn missing_header_is_reported() {
    let (mut pdf, _) = build_pdf(&[(1, CATALOG), (2, PAGES)], "/Root 1 0 R");
    pdf[..5].copy_from_slice(b"%XYZ-");
    let file = File::from_bytes(pdf).unwrap();
    assert!(kinds(&file).contains(&DiagnosticKind::MissingHeader));
}

#[test]
fn broken_startxref_triggers_repair() {
    let (mut pdf, _) = build_pdf(&[(1, CATALOG), (2, PAGES), (3, b"7")], "/Root 1 0 R");
    let pos = pdf.windows(9).rposition(|w| w == b"startxref").unwrap();
    pdf.truncate(pos);
    let file = File::from_bytes(pdf).unwrap();
    assert_eq!(file.resolve(r(3)), &Object::Int(7));
    assert!(kinds(&file).contains(&DiagnosticKind::XrefRepaired));
}

#[test]
fn wrong_entry_offset_falls_back_to_repair() {
    let (pdf, offsets) = build_pdf(&[(1, CATALOG), (2, PAGES), (3, b"7")], "/Root 1 0 R");
    // Point object 3's entry at object 2.
    let off3 = format!("{:010}", offsets[2].1);
    let off2 = format!("{:010}", offsets[1].1);
    let text = String::from_utf8(pdf).unwrap();
    let xref_at = text.rfind("xref\n0 4").unwrap();
    let (head, tail) = text.split_at(xref_at);
    let pdf = format!("{head}{}", tail.replacen(&off3, &off2, 1)).into_bytes();
    let file = File::from_bytes(pdf).unwrap();
    assert_eq!(file.resolve(r(3)), &Object::Int(7));
    let k = kinds(&file);
    assert!(k.contains(&DiagnosticKind::ObjectMismatch));
    assert!(k.contains(&DiagnosticKind::XrefRepaired));
}

#[test]
fn repair_finds_catalog_without_trailer() {
    let pdf = b"%PDF-1.4\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n2 0 obj << /Type /Pages >> endobj\n";
    let file = File::from_bytes(pdf.to_vec()).unwrap();
    assert_eq!(file.trailer().get(b"Root"), Some(&Object::Ref(r(1))));
}

/// An incremental update that adds a new catalog object supersedes the original one.
#[test]
fn repair_uses_latest_catalog() {
    let pdf =
        b"%PDF-1.4\n1 0 obj << /Type /Catalog >> endobj\n5 0 obj << /Type /Catalog >> endobj\n";
    let file = File::from_bytes(pdf.to_vec()).unwrap();
    assert_eq!(file.trailer().get(b"Root"), Some(&Object::Ref(r(5))));
}

#[test]
fn no_catalog_is_unrecoverable() {
    let err = File::from_bytes(b"%PDF-1.4\nnothing here".to_vec())
        .err()
        .unwrap();
    assert_eq!(err.kind, ErrorKind::Unrecoverable);
    let err = File::from_bytes(Vec::new()).err().unwrap();
    assert_eq!(err.kind, ErrorKind::Unrecoverable);
}

#[test]
fn unsupported_handler_is_named() {
    let (pdf, _) = build_pdf(
        &[
            (1, CATALOG),
            (2, PAGES),
            (3, b"<< /Filter /Adobe.PubSec /V 4 /R 4 >>"),
        ],
        "/Root 1 0 R /Encrypt 3 0 R",
    );
    let xref = xref_offset(&pdf);
    let err = File::from_bytes(pdf).err().unwrap();
    assert_eq!(
        err.kind,
        ErrorKind::UnsupportedEncryption("Adobe.PubSec".into())
    );
    assert_eq!(
        err.to_string(),
        format!("unsupported encryption: Adobe.PubSec (at byte {xref})")
    );
}

#[test]
fn non_dict_encrypt_opens_unencrypted() {
    for trailer in ["/Root 1 0 R /Encrypt 99 0 R", "/Root 1 0 R /Encrypt null"] {
        let (pdf, _) = build_pdf(&[(1, CATALOG), (2, PAGES)], trailer);
        let file = File::from_bytes(pdf).unwrap();
        assert_eq!(count(&file, DiagnosticKind::BadEncryption), 1, "{trailer}");
    }
}

#[test]
fn incremental_update_overrides_older_objects() {
    let (mut pdf, _) = build_pdf(&[(1, CATALOG), (2, PAGES), (3, b"(old)")], "/Root 1 0 R");
    let prev = String::from_utf8_lossy(&pdf)
        .rsplit("startxref\n")
        .next()
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_string();
    let obj_at = pdf.len();
    pdf.extend_from_slice(b"3 0 obj\n(new)\nendobj\n");
    let xref_at = pdf.len();
    pdf.extend_from_slice(
        format!(
            "xref\n3 1\n{obj_at:010} 00000 n \ntrailer\n<< /Size 4 /Root 1 0 R /Prev {prev} >>\nstartxref\n{xref_at}\n%%EOF\n"
        )
        .as_bytes(),
    );
    let file = File::from_bytes(pdf).unwrap();
    assert_eq!(file.resolve(r(3)), &Object::String(b"new".to_vec()));
    assert!(file.resolve(r(1)).as_dict().is_some());
    assert!(file.diagnostics().is_empty());
}

/// Appends an incremental update defining `objects` (number, generation, body) and marking
/// `free` free, with a classic table chained to the previous one.
fn append_update(pdf: &mut Vec<u8>, objects: &[(u32, u16, &str)], free: &[u32]) {
    let prev = xref_offset(pdf);
    let mut rows = String::new();
    for &(num, generation, body) in objects {
        let at = pdf.len();
        pdf.extend_from_slice(format!("{num} {generation} obj\n{body}\nendobj\n").as_bytes());
        rows.push_str(&format!("{num} 1\n{at:010} {generation:05} n \n"));
    }
    for num in free {
        rows.push_str(&format!("{num} 1\n0000000000 00001 f \n"));
    }
    let xref_at = pdf.len();
    pdf.extend_from_slice(
        format!(
            "xref\n{rows}trailer\n<< /Size 9 /Root 1 0 R /Prev {prev} >>\nstartxref\n{xref_at}\n%%EOF\n"
        )
        .as_bytes(),
    );
}

/// #17: §7.5.4, an object freed in an update is deleted, not its older revision.
#[test]
fn object_freed_in_update_resolves_to_null() {
    let (mut pdf, _) = build_pdf(&[(1, CATALOG), (2, PAGES), (3, b"(old)")], "/Root 1 0 R");
    append_update(&mut pdf, &[], &[3]);
    let file = File::from_bytes(pdf).unwrap();
    assert_eq!(file.resolve(r(3)), &Object::Null);
    assert_eq!(file.objects(), vec![r(1), r(2)]);
    assert!(!kinds(&file).contains(&DiagnosticKind::XrefRepaired));
}

/// #17: an object freed and then reused keeps the newest revision and its generation.
#[test]
fn object_reused_after_free_has_new_generation() {
    let (mut pdf, _) = build_pdf(&[(1, CATALOG), (2, PAGES), (3, b"(old)")], "/Root 1 0 R");
    append_update(&mut pdf, &[], &[3]);
    append_update(&mut pdf, &[(3, 1, "(new)")], &[]);
    let file = File::from_bytes(pdf).unwrap();
    assert_eq!(file.resolve(r(3)), &Object::String(b"new".to_vec()));
    let three = Ref {
        num: 3,
        generation: 1,
    };
    assert_eq!(file.objects(), vec![r(1), r(2), three]);
    assert!(file.diagnostics().is_empty(), "{:?}", file.diagnostics());
}

/// #17: with a damaged xref, `objects()` lists what the repair scan can resolve.
#[test]
fn objects_include_repaired_entries_when_xref_damaged() {
    let body = b"(x)\nendobj\n4 2 obj\n(y)";
    let (pdf, _) = build_pdf(&[(1, CATALOG), (2, PAGES), (3, body)], "/Root 1 0 R");
    let text = String::from_utf8(pdf)
        .unwrap()
        .replacen("00000 n\ntrailer", "00000 q\ntrailer", 1);
    let file = File::from_bytes(text.into_bytes()).unwrap();
    assert!(kinds(&file).contains(&DiagnosticKind::BadXrefEntry));
    let four = Ref {
        num: 4,
        generation: 2,
    };
    assert!(file.objects().contains(&four), "{:?}", file.objects());
}

#[test]
fn xref_off_by_one_subsection_is_fixed() {
    let (pdf, _) = build_pdf(&[(1, CATALOG), (2, PAGES)], "/Root 1 0 R");
    let text = String::from_utf8(pdf)
        .unwrap()
        .replacen("xref\n0 3\n", "xref\n1 3\n", 1);
    let file = File::from_bytes(text.into_bytes()).unwrap();
    assert!(file.resolve(r(1)).as_dict().is_some());
    assert!(kinds(&file).contains(&DiagnosticKind::BadXrefEntry));
    assert!(!kinds(&file).contains(&DiagnosticKind::XrefRepaired));
}

/// Objects 1 and 2 live in object stream 3; the xref is a stream (object 4) with
/// `/W [1 2 1]` and the given extra dictionary entries.
fn objstm_pdf(xref_extra: &str) -> Vec<u8> {
    let (cat, pages) = (
        "<< /Type /Catalog /Pages 2 0 R >> ",
        "<< /Type /Pages /Kids [] /Count 0 >>",
    );
    let header = format!("1 0 2 {} ", cat.len());
    let first = header.len();
    let members = format!("{header}{cat}{pages}");
    let packed = compress_to_vec_zlib(members.as_bytes(), 6);
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let objstm_at = pdf.len();
    pdf.extend_from_slice(
        format!(
            "3 0 obj\n<< /Type /ObjStm /N 2 /First {first} /Length {} /Filter /FlateDecode >>\nstream\n",
            packed.len()
        )
        .as_bytes(),
    );
    pdf.extend_from_slice(&packed);
    pdf.extend_from_slice(b"\nendstream\nendobj\n");
    let xref_at = pdf.len();
    let rows: Vec<[u8; 4]> = vec![
        [0, 0, 0, 0],
        [2, 0, 3, 0],
        [2, 0, 3, 1],
        [1, (objstm_at >> 8) as u8, objstm_at as u8, 0],
        [1, (xref_at >> 8) as u8, xref_at as u8, 0],
    ];
    let data: Vec<u8> = rows.concat();
    pdf.extend_from_slice(
        format!(
            "4 0 obj\n<< /Type /XRef /Size 5 /W [1 2 1] {xref_extra} /Length {} >>\nstream\n",
            data.len()
        )
        .as_bytes(),
    );
    pdf.extend_from_slice(&data);
    pdf.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());
    pdf
}

#[test]
fn xref_stream_and_object_stream() {
    let file = File::from_bytes(objstm_pdf("/Root 1 0 R")).unwrap();
    let catalog = file.resolve(r(1)).as_dict().unwrap();
    assert_eq!(catalog.get(b"Pages"), Some(&Object::Ref(r(2))));
    let pages = file.resolve(r(2)).as_dict().unwrap();
    assert_eq!(pages.get(b"Count"), Some(&Object::Int(0)));
    assert!(file.diagnostics().is_empty(), "{:?}", file.diagnostics());
}

#[test]
fn repair_finds_catalog_inside_object_stream() {
    let mut pdf = objstm_pdf("");
    let pos = pdf.windows(9).rposition(|w| w == b"startxref").unwrap();
    pdf[pos + 8] = b'X';
    let file = File::from_bytes(pdf).unwrap();
    assert_eq!(file.trailer().get(b"Root"), Some(&Object::Ref(r(1))));
    assert!(file.resolve(r(2)).as_dict().is_some());
    assert!(kinds(&file).contains(&DiagnosticKind::XrefRepaired));
}

#[test]
fn object_stream_referencing_itself_terminates() {
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let cat_at = pdf.len();
    pdf.extend_from_slice(b"1 0 obj << /Type /Catalog >> endobj\n");
    let xref_at = pdf.len();
    // Object 5 claims to live in object stream 5.
    let rows: Vec<[u8; 4]> = vec![
        [0, 0, 0, 0],
        [1, 0, cat_at as u8, 0],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [2, 0, 5, 0],
    ];
    let data: Vec<u8> = rows.concat();
    pdf.extend_from_slice(
        format!(
            "9 0 obj\n<< /Type /XRef /Size 6 /W [1 2 1] /Root 1 0 R /Length {} >>\nstream\n",
            data.len()
        )
        .as_bytes(),
    );
    pdf.extend_from_slice(&data);
    pdf.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());
    let file = File::from_bytes(pdf).unwrap();
    assert_eq!(file.resolve(r(5)), &Object::Null);
}

/// A `/Root` the xref places in a missing object stream is checked at open, and the file
/// is repaired to the catalog that does exist.
#[test]
fn root_in_missing_object_stream_is_repaired() {
    let mut pdf = b"%PDF-1.7\n".to_vec();
    pdf.extend_from_slice(b"5 0 obj << /Type /Catalog >> endobj\n");
    let xref_at = pdf.len();
    // Object 1 claims to live in object stream 9, which does not exist.
    let data: Vec<u8> = [[0u8, 0, 0, 0], [2, 0, 9, 0]].concat();
    pdf.extend_from_slice(
        format!(
            "8 0 obj\n<< /Type /XRef /Size 2 /W [1 2 1] /Root 1 0 R /Length {} >>\nstream\n",
            data.len()
        )
        .as_bytes(),
    );
    pdf.extend_from_slice(&data);
    pdf.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());
    let file = File::from_bytes(pdf).unwrap();
    assert_eq!(file.trailer().get(b"Root"), Some(&Object::Ref(r(5))));
    assert!(kinds(&file).contains(&DiagnosticKind::XrefRepaired));
}

/// The failed /Root check must not leave object 9 marked missing for the rebuilt table.
#[test]
fn root_in_unlisted_object_stream_is_repaired() {
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let members = b"1 0 << /Type /Catalog >>";
    pdf.extend_from_slice(
        format!(
            "9 0 obj << /Type /ObjStm /N 1 /First 4 /Length {} >>\nstream\n",
            members.len()
        )
        .as_bytes(),
    );
    pdf.extend_from_slice(members);
    pdf.extend_from_slice(b"\nendstream\nendobj\n");
    let xref_at = pdf.len();
    // Object 1 lives in object stream 9, which the xref does not list.
    let data: Vec<u8> = [[0u8, 0, 0, 0], [2, 0, 9, 0]].concat();
    pdf.extend_from_slice(
        format!(
            "8 0 obj\n<< /Type /XRef /Size 2 /W [1 2 1] /Root 1 0 R /Length {} >>\nstream\n",
            data.len()
        )
        .as_bytes(),
    );
    pdf.extend_from_slice(&data);
    pdf.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());
    let file = File::from_bytes(pdf).unwrap();
    let root = file
        .trailer()
        .get(b"Root")
        .and_then(Object::as_ref)
        .unwrap();
    assert_eq!(
        file.resolve(root)
            .as_dict()
            .and_then(|d| d.get(b"Type"))
            .and_then(Object::as_name),
        Some(&b"Catalog"[..])
    );
    let repairs = kinds(&file)
        .iter()
        .filter(|&&k| k == DiagnosticKind::XrefRepaired)
        .count();
    assert_eq!(repairs, 1);
}

#[test]
fn junk_before_header_is_repaired() {
    let (pdf, _) = build_pdf(&[(1, CATALOG), (2, PAGES), (3, b"7")], "/Root 1 0 R");
    let mut shifted = vec![b'\n'; 100];
    shifted.extend_from_slice(&pdf);
    let file = File::from_bytes(shifted).unwrap();
    assert_eq!(file.resolve(r(3)), &Object::Int(7));
    assert!(kinds(&file).contains(&DiagnosticKind::XrefRepaired));
}

#[test]
fn cr_only_line_endings() {
    let stream = b"<< /Length 5 >>\rstream\rhello\rendstream";
    let (pdf, _) = build_pdf_eol(
        &[(1, CATALOG), (2, PAGES), (3, stream)],
        "/Root 1 0 R",
        "\r",
    );
    let file = File::from_bytes(pdf).unwrap();
    assert_eq!(file.resolve(r(3)).as_stream().unwrap().raw(), b"hello");
    assert!(file.diagnostics().is_empty(), "{:?}", file.diagnostics());
}

#[test]
fn wrong_generation_in_reference_is_tolerated() {
    let (pdf, _) = build_pdf(&[(1, CATALOG), (2, PAGES), (3, b"7")], "/Root 1 0 R");
    let file = File::from_bytes(pdf).unwrap();
    assert_eq!(
        file.resolve(Ref {
            num: 3,
            generation: 5
        }),
        &Object::Int(7)
    );
}

/// A classic-table file updated by an xref-stream revision.
#[test]
fn xref_stream_update_over_classic_table() {
    let (mut pdf, _) = build_pdf(&[(1, CATALOG), (2, PAGES), (3, b"(old)")], "/Root 1 0 R");
    let text = String::from_utf8(pdf.clone()).unwrap();
    let prev = text
        .rsplit("startxref\n")
        .next()
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_string();
    let obj_at = pdf.len();
    pdf.extend_from_slice(b"3 0 obj\n(new)\nendobj\n");
    let xref_at = pdf.len();
    let data = [
        [1, (obj_at >> 8) as u8, obj_at as u8, 0],
        [1, (xref_at >> 8) as u8, xref_at as u8, 0],
    ]
    .concat();
    pdf.extend_from_slice(
        format!(
            "4 0 obj\n<< /Type /XRef /Size 5 /Index [3 2] /W [1 2 1] /Root 1 0 R /Prev {prev} /Length {} >>\nstream\n",
            data.len()
        )
        .as_bytes(),
    );
    pdf.extend_from_slice(&data);
    pdf.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());
    let file = File::from_bytes(pdf).unwrap();
    assert_eq!(file.resolve(r(3)), &Object::String(b"new".to_vec()));
    assert!(file.resolve(r(2)).as_dict().is_some());
    assert!(file.diagnostics().is_empty(), "{:?}", file.diagnostics());
}

/// Hybrid file (§7.5.8.4): the classic table marks object 3 free, and the `/XRefStm`
/// stream places it in object stream 4.
#[test]
fn hybrid_file_uses_xref_stream_entries() {
    let objstm = b"<< /Type /ObjStm /N 1 /First 4 /Length 8 >>\nstream\n3 0 (hi)\nendstream";
    let mut xrefstm =
        b"<< /Type /XRef /Size 6 /Index [3 1] /W [1 2 1] /Length 4 >>\nstream\n".to_vec();
    xrefstm.extend_from_slice(&[2, 0, 4, 0]);
    xrefstm.extend_from_slice(b"\nendstream");
    let objects: [(u32, &[u8]); 4] = [(1, CATALOG), (2, PAGES), (4, objstm), (5, &xrefstm)];
    let (_, offsets) = build_pdf(&objects, "/Root 1 0 R");
    let (pdf, _) = build_pdf(&objects, &format!("/Root 1 0 R /XRefStm {}", offsets[3].1));
    let file = File::from_bytes(pdf).unwrap();
    assert_eq!(file.resolve(r(3)), &Object::String(b"hi".to_vec()));
    assert!(file.diagnostics().is_empty(), "{:?}", file.diagnostics());
}

fn xref_offset(pdf: &[u8]) -> usize {
    pdf.windows(6).rposition(|w| w == b"\nxref\n").unwrap() + 1
}

#[test]
fn prev_cycle_terminates() {
    let objects: [(u32, &[u8]); 2] = [(1, CATALOG), (2, PAGES)];
    let (pdf, _) = build_pdf(&objects, "/Root 1 0 R");
    let own = xref_offset(&pdf);
    let (pdf, _) = build_pdf(&objects, &format!("/Root 1 0 R /Prev {own}"));
    let file = File::from_bytes(pdf).unwrap();
    assert!(file.resolve(r(2)).as_dict().is_some());
    assert!(kinds(&file).contains(&DiagnosticKind::BadXrefEntry));
}

#[test]
fn unreadable_prev_section_keeps_newer_entries() {
    let (pdf, _) = build_pdf(&[(1, CATALOG), (2, PAGES)], "/Root 1 0 R /Prev 3");
    let file = File::from_bytes(pdf).unwrap();
    assert!(file.resolve(r(2)).as_dict().is_some());
    let k = kinds(&file);
    assert!(k.contains(&DiagnosticKind::BadXrefEntry));
    assert!(!k.contains(&DiagnosticKind::XrefRepaired));
}

#[test]
fn malformed_table_entry_ends_table_but_keeps_trailer() {
    let (pdf, offsets) = build_pdf(&[(1, CATALOG), (2, PAGES)], "/Root 1 0 R");
    let entry = format!("{:010} 00000 n", offsets[1].1);
    let text = String::from_utf8(pdf)
        .unwrap()
        .replacen(&entry, "zzzzzzzzzz 00000 n", 1);
    let file = File::from_bytes(text.into_bytes()).unwrap();
    assert!(file.resolve(r(1)).as_dict().is_some());
    assert!(file.resolve(r(2)).as_dict().is_some());
    let k = kinds(&file);
    assert!(k.contains(&DiagnosticKind::BadXrefEntry));
    assert!(k.contains(&DiagnosticKind::XrefRepaired));
}

fn count(file: &File, kind: DiagnosticKind) -> usize {
    file.diagnostics().iter().filter(|d| d.kind == kind).count()
}

fn assert_fast(what: &str, start: std::time::Instant) {
    let took = start.elapsed();
    assert!(
        took < std::time::Duration::from_secs(1),
        "{what} took {took:?}"
    );
}

/// Review C1: repair must not re-parse from every `trailer` hit inside an earlier parse.
#[test]
fn repair_scan_is_linear_for_trailer_spam() {
    let mut pdf = b"%PDF-1.4\n".to_vec();
    pdf.extend_from_slice(&b"trailer(".repeat(20_000));
    let start = std::time::Instant::now();
    assert!(File::from_bytes(pdf).is_err());
    assert_fast("trailer spam", start);
}

/// Review C1: same for `N G obj` hits that sit inside an unterminated earlier object.
#[test]
fn repair_scan_is_linear_for_unterminated_objects() {
    let mut pdf = b"%PDF-1.4\n".to_vec();
    for i in 0..10_000 {
        pdf.extend_from_slice(format!("{i} 0 obj(").as_bytes());
    }
    let start = std::time::Instant::now();
    assert!(File::from_bytes(pdf).is_err());
    assert_fast("object spam", start);
}

/// An unterminated array must not swallow the objects and trailer after it during repair.
#[test]
fn repair_recovers_after_unterminated_array() {
    let pdf = b"%PDF-1.4\n3 0 obj\n[ 1 2\nendobj\n1 0 obj\n<< /Type /Catalog >>\nendobj\ntrailer\n<< /Root 1 0 R >>\n";
    let file = File::from_bytes(pdf.to_vec()).unwrap();
    assert_eq!(file.trailer().get(b"Root"), Some(&Object::Ref(r(1))));
    assert_eq!(
        file.resolve(r(3)),
        &Object::Array(vec![Object::Int(1), Object::Int(2)])
    );
}

/// Same for an `endobj` where a dictionary value should be.
#[test]
fn repair_recovers_after_missing_dict_value() {
    let pdf = b"%PDF-1.4\n3 0 obj\n<< /A 1 /B\nendobj\n1 0 obj\n<< /Type /Catalog >>\nendobj\ntrailer\n<< /Root 1 0 R >>\n";
    let file = File::from_bytes(pdf.to_vec()).unwrap();
    assert_eq!(file.trailer().get(b"Root"), Some(&Object::Ref(r(1))));
    let d = file.resolve(r(3)).as_dict().unwrap();
    assert_eq!(d.get(b"A"), Some(&Object::Int(1)));
    assert_eq!(d.get(b"B"), Some(&Object::Null));
}

fn many_keys(n: usize) -> String {
    (0..n).map(|i| format!("/k{i} 1 ")).collect()
}

/// Merging a trailer with many keys must be linear, on both the xref and repair paths.
#[test]
fn trailer_with_many_keys_is_linear() {
    let keys = many_keys(50_000);
    let (pdf, _) = build_pdf(&[(1, CATALOG), (2, PAGES)], &format!("/Root 1 0 R {keys}"));
    let start = std::time::Instant::now();
    let file = File::from_bytes(pdf).unwrap();
    assert_fast("xref trailer", start);
    assert_eq!(file.trailer().len(), 50_002);

    let pdf = format!("%PDF-1.4\n1 0 obj << /Type /Catalog >> endobj\ntrailer << {keys} >>\n");
    let start = std::time::Instant::now();
    let file = File::from_bytes(pdf.into_bytes()).unwrap();
    assert_fast("repair trailer", start);
    assert_eq!(file.trailer().len(), 50_001);
}

/// Review I1: two sections naming the same `/XRefStm` must read it only once.
#[test]
fn shared_xrefstm_is_read_once() {
    let objstm = b"<< /Type /ObjStm /N 1 /First 4 /Length 8 >>\nstream\n3 0 (hi)\nendstream";
    let mut xrefstm =
        b"<< /Type /XRef /Size 6 /Index [3 1] /W [1 2 1] /Length 4 >>\nstream\n".to_vec();
    xrefstm.extend_from_slice(&[2, 0, 4, 0]);
    xrefstm.extend_from_slice(b"\nendstream");
    let objects: [(u32, &[u8]); 4] = [(1, CATALOG), (2, PAGES), (4, objstm), (5, &xrefstm)];
    let (_, offsets) = build_pdf(&objects, "/Root 1 0 R");
    let stm = offsets[3].1;
    let (mut pdf, _) = build_pdf(&objects, &format!("/Root 1 0 R /XRefStm {stm}"));
    let prev = xref_offset(&pdf);
    let xref_at = pdf.len();
    pdf.extend_from_slice(
        format!(
            "xref\n0 1\n0000000000 65535 f \ntrailer\n<< /Size 6 /Root 1 0 R /XRefStm {stm} /Prev {prev} >>\nstartxref\n{xref_at}\n%%EOF\n"
        )
        .as_bytes(),
    );
    let file = File::from_bytes(pdf).unwrap();
    assert_eq!(file.resolve(r(3)), &Object::String(b"hi".to_vec()));
    assert_eq!(count(&file, DiagnosticKind::BadXrefEntry), 1);
}

/// An xref stream row pointing past EOF is diagnosed, so the object is found by repair.
#[test]
fn xref_stream_offset_past_eof_falls_back_to_repair() {
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let cat_at = pdf.len();
    pdf.extend_from_slice(b"1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n");
    pdf.extend_from_slice(b"2 0 obj << /Type /Pages >> endobj\n");
    let xref_at = pdf.len();
    let rows: Vec<[u8; 4]> = vec![[0, 0, 0, 0], [1, 0, cat_at as u8, 0], [1, 0xff, 0xff, 0]];
    let data: Vec<u8> = rows.concat();
    pdf.extend_from_slice(
        format!(
            "9 0 obj\n<< /Type /XRef /Size 3 /W [1 2 1] /Root 1 0 R /Length {} >>\nstream\n",
            data.len()
        )
        .as_bytes(),
    );
    pdf.extend_from_slice(&data);
    pdf.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());
    let file = File::from_bytes(pdf).unwrap();
    assert!(file.resolve(r(2)).as_dict().is_some());
    assert!(kinds(&file).contains(&DiagnosticKind::BadXrefEntry));
}

/// A trailer `/Root` naming no object falls back to the catalog the repair scan found.
#[test]
fn repair_replaces_dangling_root_with_catalog() {
    let pdf = b"%PDF-1.7\n1 0 obj<</Type /Catalog>>endobj\ntrailer<</Root 99 0 R>>";
    let file = File::from_bytes(pdf.to_vec()).unwrap();
    assert_eq!(file.trailer().get(b"Root"), Some(&Object::Ref(r(1))));

    let pdf = b"%PDF-1.7\n1 0 obj (x) endobj\ntrailer<</Root 99 0 R>>";
    let err = File::from_bytes(pdf.to_vec()).err().unwrap();
    assert_eq!(err.kind, ErrorKind::Unrecoverable);
}

/// Review I4 + I6: a dangling reference is reported once and does not trigger repair.
#[test]
fn missing_object_is_reported_once_without_repair() {
    let (pdf, _) = build_pdf(&[(1, CATALOG), (2, PAGES)], "/Root 1 0 R");
    let file = File::from_bytes(pdf).unwrap();
    assert_eq!(file.resolve(r(9)), &Object::Null);
    assert_eq!(file.resolve(r(9)), &Object::Null);
    assert_eq!(count(&file, DiagnosticKind::ObjectNotFound), 1);
    assert_eq!(count(&file, DiagnosticKind::XrefRepaired), 0);
}

/// Review I4: a wrong entry is diagnosed once; later resolves reuse the repaired table.
#[test]
fn mismatched_entry_is_reported_once() {
    let (pdf, offsets) = build_pdf(&[(1, CATALOG), (2, PAGES), (3, b"7")], "/Root 1 0 R");
    let off3 = format!("{:010}", offsets[2].1);
    let off2 = format!("{:010}", offsets[1].1);
    let text = String::from_utf8(pdf).unwrap();
    let xref_at = text.rfind("xref\n0 4").unwrap();
    let (head, tail) = text.split_at(xref_at);
    let pdf = format!("{head}{}", tail.replacen(&off3, &off2, 1)).into_bytes();
    let file = File::from_bytes(pdf).unwrap();
    assert_eq!(file.resolve(r(3)), &Object::Int(7));
    assert_eq!(file.resolve(r(3)), &Object::Int(7));
    assert_eq!(count(&file, DiagnosticKind::ObjectMismatch), 1);
    assert_eq!(count(&file, DiagnosticKind::XrefRepaired), 1);
}

/// Review I4: members the xref places in an object stream but its header lacks must not
/// re-decode the stream on every lookup. The hex data ends in `x`, so each decode adds
/// one FilterError.
#[test]
fn object_stream_is_decoded_once_for_missing_members() {
    let members = b"2 0 << /Type /Pages /Kids [] /Count 0 >>";
    let hex: String = members.iter().map(|b| format!("{b:02x}")).collect();
    let objstm = format!(
        "<< /Type /ObjStm /N 1 /First 4 /Filter /ASCIIHexDecode /Length {} >>\nstream\n{hex}x\nendstream",
        hex.len() + 1
    );
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let cat_at = pdf.len();
    pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    let stm_at = pdf.len();
    pdf.extend_from_slice(format!("5 0 obj\n{objstm}\nendobj\n").as_bytes());
    let xref_at = pdf.len();
    let rows: Vec<[u8; 4]> = vec![
        [0, 0, 0, 0],
        [1, 0, cat_at as u8, 0],
        [2, 0, 5, 0],
        [2, 0, 5, 1],
        [2, 0, 5, 2],
        [1, (stm_at >> 8) as u8, stm_at as u8, 0],
    ];
    let data: Vec<u8> = rows.concat();
    pdf.extend_from_slice(
        format!(
            "6 0 obj\n<< /Type /XRef /Size 6 /W [1 2 1] /Root 1 0 R /Length {} >>\nstream\n",
            data.len()
        )
        .as_bytes(),
    );
    pdf.extend_from_slice(&data);
    pdf.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());
    let file = File::from_bytes(pdf).unwrap();
    assert!(file.resolve(r(2)).as_dict().is_some());
    assert_eq!(file.resolve(r(3)), &Object::Null);
    assert_eq!(file.resolve(r(4)), &Object::Null);
    assert_eq!(count(&file, DiagnosticKind::FilterError), 1);
}

/// Review (PR #3): a thread that finds another mid-way through loading an object stream
/// must not resolve the stream's members to null.
#[test]
fn concurrent_object_stream_lookups_all_resolve() {
    for _ in 0..200 {
        let file = File::from_bytes(objstm_pdf("/Root 1 0 R")).unwrap();
        let barrier = std::sync::Barrier::new(8);
        std::thread::scope(|s| {
            for i in 0..8 {
                let (file, barrier) = (&file, &barrier);
                s.spawn(move || {
                    barrier.wait();
                    assert!(file.resolve(r(1 + i % 2)).as_dict().is_some());
                });
            }
        });
        assert!(file.diagnostics().is_empty(), "{:?}", file.diagnostics());
    }
}

/// Review (PR #3): an indirect `/Length` whose xref entry points at a different object is
/// not trusted, even when that object's integer happens to land on a later `endstream`.
#[test]
fn indirect_length_with_mismatched_header_is_ignored() {
    let swallowed = "\nendstream\nendobj\n4 0 obj\n<< /Length 3 >>\nstream\nxyz";
    let wrong = ("hello".len() + swallowed.len()).to_string();
    let objects: [(u32, &[u8]); 6] = [
        (1, CATALOG),
        (2, PAGES),
        (3, b"<< /Length 5 0 R >>\nstream\nhello\nendstream"),
        (4, b"<< /Length 3 >>\nstream\nxyz\nendstream"),
        (5, b"5"),
        (6, wrong.as_bytes()),
    ];
    let (pdf, offsets) = build_pdf(&objects, "/Root 1 0 R");
    let row = |n: usize| format!("{:010} 00000 n", offsets[n].1);
    let pdf = String::from_utf8(pdf)
        .unwrap()
        .replacen(&row(4), &row(5), 1);
    let file = File::from_bytes(pdf.into_bytes()).unwrap();
    assert_eq!(file.resolve(r(3)).as_stream().unwrap().raw(), b"hello");
    assert!(kinds(&file).contains(&DiagnosticKind::BadStreamLength));
}

#[test]
fn object_stream_without_first_is_diagnosed() {
    let pdf = objstm_pdf("/Root 1 0 R");
    let at = pdf.windows(6).position(|w| w == b"/First").unwrap();
    let mut pdf = pdf;
    pdf[at..at + 6].copy_from_slice(b"/Fxxxx");
    let file = File::from_bytes(pdf).unwrap();
    file.resolve(r(2));
    assert!(
        kinds(&file).contains(&DiagnosticKind::BadObject),
        "{:?}",
        file.diagnostics()
    );
}

/// #32 F5: a too-large count or a stray token must not consume the `trailer` keyword.
#[test]
fn table_overrun_keeps_trailer_without_repair() {
    let (pdf, _) = build_pdf(&[(1, CATALOG), (2, PAGES)], "/Root 1 0 R");
    let text = String::from_utf8(pdf).unwrap();
    for bad in [
        text.replacen("xref\n0 3\n", "xref\n0 4\n", 1),
        text.replacen("trailer\n", "7\ntrailer\n", 1),
    ] {
        let file = File::from_bytes(bad.into_bytes()).unwrap();
        assert!(file.resolve(r(2)).as_dict().is_some());
        assert_eq!(kinds(&file), vec![DiagnosticKind::BadXrefEntry]);
    }
}

/// #32 F5: a subsection whose object numbers do not fit in `u32` is diagnosed.
#[test]
fn table_subsection_past_u32_is_diagnosed() {
    let (pdf, _) = build_pdf(&[(1, CATALOG), (2, PAGES)], "/Root 1 0 R");
    let text = String::from_utf8(pdf).unwrap().replacen(
        "trailer\n",
        "4294967296 1\n0000000009 00000 n\ntrailer\n",
        1,
    );
    let file = File::from_bytes(text.into_bytes()).unwrap();
    assert_eq!(kinds(&file), vec![DiagnosticKind::BadXrefEntry]);
}

/// #32 F6: `trailer` inside a comment must not swallow the object after it.
#[test]
fn repair_ignores_trailer_in_comment() {
    let pdf = b"%PDF-1.4\n% trailer\n1 0 obj << /Type /Catalog >> endobj\n";
    let file = File::from_bytes(pdf.to_vec()).unwrap();
    assert_eq!(file.trailer().get(b"Root"), Some(&Object::Ref(r(1))));
}

/// Object 3 is listed in object stream 4, and the xref stream's rows are `rows_after_3`
/// for object 4 onwards. Object 4 is the object stream holding `3 0 (hi)` unless `objstm`
/// replaces it.
fn objstm_member_pdf(objstm: Option<&str>, rows_after_3: usize) -> Vec<u8> {
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let cat_at = pdf.len();
    pdf.extend_from_slice(b"1 0 obj << /Type /Catalog >> endobj\n");
    let stm_at = pdf.len();
    let body = objstm
        .unwrap_or("<< /Type /ObjStm /N 1 /First 4 /Length 8 >>\nstream\n3 0 (hi)\nendstream");
    pdf.extend_from_slice(format!("4 0 obj\n{body}\nendobj\n").as_bytes());
    let xref_at = pdf.len();
    let mut rows: Vec<[u8; 4]> = vec![
        [0, 0, 0, 0],
        [1, 0, cat_at as u8, 0],
        [0, 0, 0, 0],
        [2, 0, 4, 0],
    ];
    rows.extend([[1, 0, stm_at as u8, 0]].repeat(rows_after_3));
    let data: Vec<u8> = rows.concat();
    pdf.extend_from_slice(
        format!(
            "9 0 obj\n<< /Type /XRef /Size {} /W [1 2 1] /Root 1 0 R /Length {} >>\nstream\n",
            rows.len(),
            data.len()
        )
        .as_bytes(),
    );
    pdf.extend_from_slice(&data);
    pdf.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());
    pdf
}

/// #32 F7: a member's object stream missing from the xref is found by the repair scan.
#[test]
fn member_of_unlisted_object_stream_is_repaired() {
    let file = File::from_bytes(objstm_member_pdf(None, 0)).unwrap();
    assert_eq!(file.resolve(r(3)), &Object::String(b"hi".to_vec()));
}

/// #32 F7: a container that is not a stream is a bad object, not a missing one.
#[test]
fn non_stream_object_stream_is_bad_object() {
    let file = File::from_bytes(objstm_member_pdf(Some("5"), 1)).unwrap();
    assert_eq!(file.resolve(r(3)), &Object::Null);
    let d = file.diagnostics();
    assert!(
        d.iter()
            .any(|d| d.kind == DiagnosticKind::BadObject && d.location == Location::Object(r(4))),
        "{d:?}"
    );
    assert!(
        !d.iter()
            .any(|d| d.location == Location::Object(r(4))
                && d.kind == DiagnosticKind::ObjectNotFound)
    );
}

/// #32 F8: an unusable `/Prev` or `/XRefStm` is diagnosed, so absent objects are looked
/// for by repair.
#[test]
fn bad_prev_and_xrefstm_are_diagnosed() {
    for extra in ["/Prev /bad", "/XRefStm -1"] {
        let (pdf, _) = build_pdf(&[(1, CATALOG), (2, PAGES)], &format!("/Root 1 0 R {extra}"));
        let mut pdf = pdf;
        pdf.extend_from_slice(b"3 0 obj (lost) endobj\n");
        let file = File::from_bytes(pdf).unwrap();
        assert_eq!(count(&file, DiagnosticKind::BadXrefEntry), 1, "{extra}");
        assert_eq!(file.resolve(r(3)), &Object::String(b"lost".to_vec()));
    }
}

/// #32 F8: an object stream header with fewer than `/N` pairs is diagnosed.
#[test]
fn truncated_object_stream_header_is_diagnosed() {
    let pdf = objstm_pdf("/Root 1 0 R");
    let at = pdf.windows(4).position(|w| w == b"/N 2").unwrap();
    let mut pdf = pdf;
    pdf[at + 3] = b'3';
    let file = File::from_bytes(pdf).unwrap();
    assert!(file.resolve(r(2)).as_dict().is_some());
    assert_eq!(kinds(&file), vec![DiagnosticKind::BadObject]);
}

/// #32 F9: a repaired trailer's `/Root` that is not a dictionary is replaced by the
/// catalog the scan found.
#[test]
fn repair_replaces_non_dict_root_with_catalog() {
    let pdf = b"%PDF-1.7\n1 0 obj << /Type /Catalog >> endobj\n3 0 obj 5 endobj\ntrailer << /Root 3 0 R >>\n";
    let file = File::from_bytes(pdf.to_vec()).unwrap();
    assert_eq!(file.trailer().get(b"Root"), Some(&Object::Ref(r(1))));
}

/// #32 F37: in repair, a later object stream's member beats an earlier one's.
#[test]
fn repair_prefers_later_object_stream_member() {
    let mut pdf = b"%PDF-1.7\n1 0 obj << /Type /Catalog >> endobj\n".to_vec();
    for (num, member) in [(4, "(old)"), (5, "(new)")] {
        let members = format!("3 0 {member}");
        pdf.extend_from_slice(
            format!(
                "{num} 0 obj << /Type /ObjStm /N 1 /First 4 /Length {} >>\nstream\n{members}\nendstream\nendobj\n",
                members.len()
            )
            .as_bytes(),
        );
    }
    let file = File::from_bytes(pdf).unwrap();
    assert_eq!(file.resolve(r(3)), &Object::String(b"new".to_vec()));
}

/// #32 F10a: a stream loaded from the repaired table reads its indirect `/Length` there.
#[test]
fn repaired_stream_reads_length_from_repaired_table() {
    let objects: [(u32, &[u8]); 4] = [
        (1, CATALOG),
        (2, PAGES),
        (3, b"<< /Length 4 0 R >>\nstream\nhello\nendstream"),
        (4, b"5"),
    ];
    let (pdf, offsets) = build_pdf(&objects, "/Root 1 0 R");
    let row = |n: usize| format!("{:010} 00000 n", offsets[n].1);
    let pdf = String::from_utf8(pdf)
        .unwrap()
        .replacen(&row(2), &row(1), 1)
        .replacen(&row(3), "0000000000 00001 f", 1);
    let file = File::from_bytes(pdf.into_bytes()).unwrap();
    assert_eq!(file.resolve(r(3)).as_stream().unwrap().raw(), b"hello");
    assert_eq!(count(&file, DiagnosticKind::BadStreamLength), 0);
}

/// #32 F44: a damaged xref does not resurrect an object it marks free.
#[test]
fn free_object_stays_null_in_damaged_xref() {
    let objects: [(u32, &[u8]); 3] = [(1, CATALOG), (2, PAGES), (5, b"(five)")];
    let (pdf, offsets) = build_pdf(&objects, "/Root 1 0 R /Prev 99999");
    let row = format!("{:010} 00000 n", offsets[2].1);
    let pdf = String::from_utf8(pdf)
        .unwrap()
        .replacen(&row, "0000000000 00001 f", 1);
    let file = File::from_bytes(pdf.into_bytes()).unwrap();
    assert_eq!(file.resolve(r(5)), &Object::Null);
    assert_eq!(file.objects(), vec![r(1), r(2)]);
}

/// #41 review: free rows of a hybrid table whose `/XRefStm` is unreadable do not block
/// the repair scan from finding the compressed objects.
#[test]
fn damaged_hybrid_free_rows_do_not_block_repair() {
    let objstm = b"<< /Type /ObjStm /N 1 /First 4 /Length 8 >>\nstream\n3 0 (hi)\nendstream";
    let objects: [(u32, &[u8]); 3] = [(1, CATALOG), (2, PAGES), (4, objstm)];
    let (pdf, _) = build_pdf(&objects, "/Root 1 0 R /XRefStm 99999");
    let file = File::from_bytes(pdf).unwrap();
    assert_eq!(file.resolve(r(3)), &Object::String(b"hi".to_vec()));
    assert!(file.objects().contains(&r(3)));
}

/// #41 review: an older revision's free row does not hide an object a damaged newer table
/// lost.
#[test]
fn older_free_row_does_not_block_repair_of_damaged_update() {
    let (mut pdf, _) = build_pdf(&[(1, CATALOG), (2, PAGES), (4, b"null")], "/Root 1 0 R");
    let prev = xref_offset(&pdf);
    let at = pdf.len();
    pdf.extend_from_slice(b"3 0 obj (new) endobj\n");
    let xref_at = pdf.len();
    pdf.extend_from_slice(
        format!(
            "xref\n0 1\n0000000000 65535 f \n2 2\nGARBAGEROW\n{at:010} 00000 n \ntrailer\n<< /Size 5 /Root 1 0 R /Prev {prev} >>\nstartxref\n{xref_at}\n%%EOF\n"
        )
        .as_bytes(),
    );
    let file = File::from_bytes(pdf).unwrap();
    assert_eq!(file.resolve(r(3)), &Object::String(b"new".to_vec()));
    assert!(file.objects().contains(&r(3)));
}
