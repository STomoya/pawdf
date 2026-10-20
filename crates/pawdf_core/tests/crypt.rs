mod common;

use common::crypt::{Cfm, Enc, raw_stream};
use common::{CATALOG, PAGES, build_pdf};
use miniz_oxide::deflate::compress_to_vec_zlib;
use pawdf_core::{
    DiagnosticKind, Diagnostics, ErrorKind, File, Location, Method, Object, PasswordKind, Ref,
};

fn r(num: u32) -> Ref {
    Ref { num, generation: 0 }
}

fn count(file: &File, kind: DiagnosticKind) -> usize {
    file.diagnostics().iter().filter(|d| d.kind == kind).count()
}

fn xref_offset(pdf: &[u8]) -> usize {
    let s = String::from_utf8_lossy(pdf);
    let tail = &s[s.rfind("startxref").unwrap() + 9..];
    tail.split_whitespace().next().unwrap().parse().unwrap()
}

/// Objects 1–2 catalog and pages, 3 a dict with an encrypted string nested in an array,
/// 4 an encrypted stream, 5 the encryption dictionary.
fn encrypted(enc: &Enc) -> Vec<u8> {
    let obj3 = format!(
        "<< /S {} /A [ {} ] >>",
        enc.string(3, b"secret"),
        enc.string(3, b"nested")
    );
    let obj4 = enc.stream(4, "", b"BT (x) Tj ET");
    let dict = enc.dict();
    build_pdf(
        &[
            (1, CATALOG),
            (2, PAGES),
            (3, obj3.as_bytes()),
            (4, &obj4),
            (5, dict.as_bytes()),
        ],
        &format!("/Root 1 0 R {}", enc.trailer(5)),
    )
    .0
}

fn assert_plaintext(file: &File) {
    let d = file.resolve(r(3)).as_dict().unwrap();
    assert_eq!(d.get(b"S").unwrap().as_string(), Some(&b"secret"[..]));
    let a = d.get(b"A").unwrap().as_array().unwrap();
    assert_eq!(a[0].as_string(), Some(&b"nested"[..]));
    assert_eq!(
        file.resolve(r(4)).as_stream().unwrap().raw(),
        b"BT (x) Tj ET"
    );
}

#[test]
fn every_revision_and_password() {
    for rev in [2, 3, 4, 5, 6] {
        let pdf = encrypted(&Enc::new(rev, "u", "o"));
        let xref = xref_offset(&pdf);
        let err = File::from_bytes(pdf.clone()).err().unwrap();
        assert_eq!(
            err.kind,
            ErrorKind::IncorrectPassword {
                password_given: false
            },
            "R{rev}"
        );
        assert_eq!(
            err.to_string(),
            format!("a password is required (at byte {xref})")
        );
        for pw in ["u", "o"] {
            let file = File::from_bytes_with_password(pdf.clone(), pw).unwrap();
            assert_plaintext(&file);
            assert!(
                file.diagnostics().is_empty(),
                "R{rev} {pw}: {:?}",
                file.diagnostics()
            );
        }
        let err = File::from_bytes_with_password(pdf, "x").err().unwrap();
        assert_eq!(
            err.kind,
            ErrorKind::IncorrectPassword {
                password_given: true
            },
            "R{rev}"
        );

        let pdf = encrypted(&Enc::new(rev, "", "o"));
        let file = File::from_bytes(pdf.clone()).unwrap();
        assert_plaintext(&file);
        assert!(file.diagnostics().is_empty(), "R{rev}");
        let file = File::from_bytes_with_password(pdf, "x").unwrap();
        assert_plaintext(&file);
        assert_eq!(count(&file, DiagnosticKind::WrongPassword), 1, "R{rev}");
    }
}

/// pikepdf output (see `src/crypt/key.rs`): the info string as object 2's `/T` and the
/// content stream as object 5, keeping their object numbers since R4 keys depend on them.
#[test]
fn known_answer_file_opens() {
    let r4 = (
        "/V 4 /R 4 /Length 128 /P -1028 /CF << /StdCF << /CFM /AESV2 /Length 16 >> >> /StmF /StdCF /StrF /StdCF",
        "2a2f0a1990192c60114730bdcd39f37828a53c89a340dd473c85299dc5258e1c",
        "a09e2aa09050292242ca44e3ea530b0e0021446990b9e4114071a4d9104984c1",
        "",
        "91129a4b973c0fc9429654415580045b9a29fc9d544693e52cccbc0a98c3abda",
        "63be1f01a45e6bb7142f23396cebb6ea221f51bff0fb8cd7d3effeed5f378ff3737a6de833e7ff78145e6cd7ff4a59b3",
    );
    let r6 = (
        "/V 5 /R 6 /Length 256 /P -1028 /CF << /StdCF << /CFM /AESV3 /Length 32 >> >> /StmF /StdCF /StrF /StdCF",
        "032d448327f132a5de8700ae8fc5dcdce728f71bcf5e006f58369a833ef654c5c236fb4847c6c457f62d17dd48160adf",
        "aa7e624e63c80251c24affcc1c68f17dd89a77d6219a2b2728abe0c6c010345c1a1ccbc623e8097c103c4072d8c70094",
        "/OE <a15024c8210101a9b6ac57bab9ed8c68725052ff912b02c7aa4ea8a0f8cd40a8> /UE <32dbec132621ec00d7cb6682b41f78ae402db7ba0a438457cf6b9ef543fd1a62> /Perms <06aa5f81d15e9c3295fb1e203ef2140e>",
        "d5f7506bb5f859c4051dc1b7f0d6d7314b3e016991e403b36cabc4870bb91f08",
        "070967b33263f268b72c85dc1bf8740676b5d57a1f72e9a6bdfa24e6f7bf42f4d45abbcbc281b3b7add93944b1571122",
    );
    for (entries, o, u, extra, string, stream) in [r4, r6] {
        let pages = format!("<< /Type /Pages /Kids [] /Count 0 /T <{string}> >>");
        let content = raw_stream("", &unhex(stream));
        let dict = format!("<< /Filter /Standard {entries} /O <{o}> /U <{u}> {extra} >>");
        let (pdf, _) = build_pdf(
            &[
                (1, CATALOG),
                (2, pages.as_bytes()),
                (5, &content),
                (6, dict.as_bytes()),
            ],
            "/Root 1 0 R /Encrypt 6 0 R /ID [<31415926535897932384626433832795> <00>]",
        );
        for pw in ["u", "o"] {
            let file = File::from_bytes_with_password(pdf.clone(), pw).unwrap();
            let pages = file.resolve(r(2)).as_dict().unwrap();
            assert_eq!(
                pages.get(b"T").unwrap().as_string(),
                Some(&b"Hello KAT"[..])
            );
            let s = file.resolve(r(5)).as_stream().unwrap();
            assert_eq!(s.raw(), b"BT /F1 12 Tf (KAT) Tj ET");
            assert!(file.diagnostics().is_empty(), "{:?}", file.diagnostics());
        }
    }
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn encrypt_dict_and_trailer_id_left_alone() {
    let enc = Enc::new(4, "u", "o");
    let file = File::from_bytes_with_password(encrypted(&enc), "u").unwrap();
    let d = file.resolve(r(5)).as_dict().unwrap();
    assert_eq!(d.get(b"O").unwrap().as_string(), Some(&enc.o()[..]));
    let id = file.trailer().get(b"ID").unwrap().as_array().unwrap();
    assert_eq!(id[0].as_string(), Some(&enc.id[..]));
}

#[test]
fn missing_id_is_diagnostic() {
    for (rev, diags) in [(4, 1), (6, 0)] {
        let mut enc = Enc::new(rev, "", "o");
        enc.id = Vec::new();
        let file = File::from_bytes(encrypted(&enc)).unwrap();
        assert_plaintext(&file);
        assert_eq!(count(&file, DiagnosticKind::BadEncryption), diags, "R{rev}");
    }
}

/// A file with an xref stream: `direct` objects in the file, `compressed` as
/// (number, object stream, index), and the trailer entries in the xref stream dictionary.
fn xref_stream_pdf(direct: &[(u32, &[u8])], compressed: &[(u32, u32)], trailer: &str) -> Vec<u8> {
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut rows: Vec<(u32, [u8; 7])> = Vec::new();
    for (num, body) in direct {
        let off = pdf.len() as u32;
        let o = off.to_be_bytes();
        rows.push((*num, [1, o[0], o[1], o[2], o[3], 0, 0]));
        pdf.extend_from_slice(format!("{num} 0 obj\n").as_bytes());
        pdf.extend_from_slice(body);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    for (index, (num, stream)) in compressed.iter().enumerate() {
        let s = stream.to_be_bytes();
        rows.push((*num, [2, s[0], s[1], s[2], s[3], 0, index as u8]));
    }
    let xref_num = rows.iter().map(|(n, _)| n + 1).max().unwrap_or(1);
    let xref_at = pdf.len() as u32;
    let o = xref_at.to_be_bytes();
    rows.push((xref_num, [1, o[0], o[1], o[2], o[3], 0, 0]));
    let size = xref_num + 1;
    let mut data = Vec::new();
    for n in 0..size {
        match rows.iter().find(|(m, _)| *m == n) {
            Some((_, row)) => data.extend_from_slice(row),
            None => data.extend_from_slice(&[0; 7]),
        }
    }
    pdf.extend_from_slice(
        format!(
            "{xref_num} 0 obj\n<< /Type /XRef /Size {size} /W [1 4 2] {trailer} /Length {} >>\nstream\n",
            data.len()
        )
        .as_bytes(),
    );
    pdf.extend_from_slice(&data);
    pdf.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());
    pdf
}

#[test]
fn object_stream_members_decrypted_once() {
    let enc = Enc::new(4, "u", "o");
    let member = "<< /S (secret) >>";
    let header = "3 0 ";
    let packed = compress_to_vec_zlib(format!("{header}{member}").as_bytes(), 6);
    let objstm = enc.stream(
        6,
        &format!(
            "/Type /ObjStm /N 1 /First {} /Filter /FlateDecode",
            header.len()
        ),
        &packed,
    );
    let dict = enc.dict();
    let pdf = xref_stream_pdf(
        &[(1, CATALOG), (2, PAGES), (5, dict.as_bytes()), (6, &objstm)],
        &[(3, 6)],
        &format!("/Root 1 0 R {}", enc.trailer(5)),
    );
    let file = File::from_bytes_with_password(pdf, "u").unwrap();
    let d = file.resolve(r(3)).as_dict().unwrap();
    assert_eq!(d.get(b"S").unwrap().as_string(), Some(&b"secret"[..]));
    assert!(file.diagnostics().is_empty(), "{:?}", file.diagnostics());
}

#[test]
fn bad_aes_padding_keeps_partial_data() {
    let enc = Enc::new(4, "u", "o");
    let mut ct = enc.encrypt(4, b"BT (x) Tj ET");
    *ct.last_mut().unwrap() ^= 0xFF;
    let obj4 = raw_stream("", &ct);
    let dict = enc.dict();
    let (pdf, offsets) = build_pdf(
        &[(1, CATALOG), (2, PAGES), (4, &obj4), (5, dict.as_bytes())],
        &format!("/Root 1 0 R {}", enc.trailer(5)),
    );
    let file = File::from_bytes_with_password(pdf, "u").unwrap();
    let raw = file.resolve(r(4)).as_stream().unwrap().raw().to_vec();
    assert_eq!(raw.len(), 16);
    let at = offsets.iter().find(|(n, _)| *n == 4).unwrap().1 as u64;
    let bad: Vec<_> = file
        .diagnostics()
        .into_iter()
        .filter(|d| d.kind == DiagnosticKind::BadEncryption)
        .collect();
    assert_eq!(bad.len(), 1);
    assert_eq!(bad[0].location, Location::File(at));
}

#[test]
fn sasl_prep_password() {
    let mut enc = Enc::new(6, "", "o");
    enc.user = b"password".to_vec();
    let file = File::from_bytes_with_password(encrypted(&enc), "pass\u{AD}word").unwrap();
    assert_plaintext(&file);
}

#[test]
fn raw_utf8_password_on_r4() {
    let enc = Enc::new(4, "日本", "o");
    let file = File::from_bytes_with_password(encrypted(&enc), "日本").unwrap();
    assert_plaintext(&file);
}

#[test]
fn crypt_filter_length_bits_or_bytes() {
    let enc = Enc::new(4, "u", "o");
    for len in ["/Length 16", "/Length 128"] {
        let dict = enc
            .dict()
            .replace("/CFM /AESV2 /Length 16", &format!("/CFM /AESV2 {len}"));
        let obj4 = enc.stream(4, "", b"BT (x) Tj ET");
        let (pdf, _) = build_pdf(
            &[(1, CATALOG), (2, PAGES), (4, &obj4), (5, dict.as_bytes())],
            &format!("/Root 1 0 R {}", enc.trailer(5)),
        );
        let file = File::from_bytes_with_password(pdf, "u").unwrap();
        assert_eq!(
            file.resolve(r(4)).as_stream().unwrap().raw(),
            b"BT (x) Tj ET",
            "{len}"
        );
    }
}

/// Catalog, pages, the given objects, and `dict` as object 5.
fn with_dict(enc: &Enc, dict: &str, objects: &[(u32, &[u8])]) -> File {
    let mut all: Vec<(u32, &[u8])> = vec![(1, CATALOG), (2, PAGES), (5, dict.as_bytes())];
    all.extend_from_slice(objects);
    all.sort_by_key(|(n, _)| *n);
    let (pdf, _) = build_pdf(&all, &format!("/Root 1 0 R {}", enc.trailer(5)));
    File::from_bytes_with_password(pdf, "u").unwrap()
}

fn stream(file: &File, num: u32) -> &pawdf_core::Stream {
    file.resolve(r(num)).as_stream().unwrap()
}

const PLAIN: &[u8] = b"BT (x) Tj ET";

#[test]
fn per_stream_crypt_identity() {
    let enc = Enc::new(4, "u", "o");
    let obj4 = raw_stream(
        "/Filter [/Crypt] /DecodeParms [<< /Name /Identity >>]",
        PLAIN,
    );
    let file = with_dict(&enc, &enc.dict(), &[(4, &obj4)]);
    let s = stream(&file, 4);
    assert_eq!(s.raw(), PLAIN);
    assert_eq!(s.dict.get(b"Filter"), None);
    assert_eq!(s.dict.get(b"DecodeParms"), None);
    assert!(file.diagnostics().is_empty(), "{:?}", file.diagnostics());
}

#[test]
fn per_stream_crypt_named_filter() {
    let enc = Enc::new(4, "u", "o");
    let rc4 = Enc {
        cfm: Cfm::Rc4,
        ..Enc::new(4, "u", "o")
    };
    let dict = enc
        .dict()
        .replace("/Length 16 >> >>", "/Length 16 >> /Other << /CFM /V2 >> >>");
    let obj4 = rc4.stream(4, "/Filter /Crypt /DecodeParms << /Name /Other >>", PLAIN);
    let file = with_dict(&enc, &dict, &[(4, &obj4)]);
    assert_eq!(stream(&file, 4).raw(), PLAIN);
    assert!(file.diagnostics().is_empty(), "{:?}", file.diagnostics());
}

#[test]
fn crypt_filter_with_indirect_decodeparms() {
    let enc = Enc::new(4, "u", "o");
    let packed = compress_to_vec_zlib(PLAIN, 6);
    let obj4 = raw_stream("/Filter [/Crypt /FlateDecode] /DecodeParms 7 0 R", &packed);
    let file = with_dict(
        &enc,
        &enc.dict(),
        &[(4, &obj4), (7, b"[<< /Name /Identity >> null]")],
    );
    let s = stream(&file, 4);
    assert_eq!(
        s.dict.get(b"Filter").unwrap().as_name(),
        Some(&b"FlateDecode"[..])
    );
    let mut d = Diagnostics::new();
    assert_eq!(s.decode(Location::File(0), &mut d).data, PLAIN);
    assert!(d.is_empty());
}

#[test]
fn unknown_crypt_filter_name_is_diagnostic() {
    let enc = Enc::new(4, "u", "o");
    let ct = enc.encrypt(4, PLAIN);
    let obj4 = raw_stream("/Filter /Crypt /DecodeParms << /Name /Nope >>", &ct);
    let file = with_dict(&enc, &enc.dict(), &[(4, &obj4)]);
    assert_eq!(stream(&file, 4).raw(), &ct[..]);
    assert_eq!(count(&file, DiagnosticKind::BadEncryption), 1);
}

#[test]
fn encrypt_metadata_false() {
    let mut enc = Enc::new(4, "u", "o");
    enc.encrypt_metadata = false;
    let meta = raw_stream("/Type /Metadata /Subtype /XML", b"<x:xmpmeta/>");
    let obj4 = enc.stream(4, "", PLAIN);
    let file = with_dict(&enc, &enc.dict(), &[(4, &obj4), (6, &meta)]);
    assert_eq!(stream(&file, 6).raw(), b"<x:xmpmeta/>");
    assert_eq!(stream(&file, 4).raw(), PLAIN);
}

#[test]
fn xref_stream_left_alone() {
    let enc = Enc::new(4, "u", "o");
    let dict = enc.dict();
    let pdf = xref_stream_pdf(
        &[(1, CATALOG), (2, PAGES), (5, dict.as_bytes())],
        &[],
        &format!("/Root 1 0 R {}", enc.trailer(5)),
    );
    let file = File::from_bytes_with_password(pdf, "u").unwrap();
    let xref = stream(&file, 6);
    assert_eq!(xref.raw().len(), 7 * 7);
    assert_eq!(&xref.raw()[..8], &[0, 0, 0, 0, 0, 0, 0, 1]);
    let id = xref.dict.get(b"ID").unwrap().as_array().unwrap();
    assert_eq!(id[0].as_string(), Some(&enc.id[..]));
    assert_eq!(count(&file, DiagnosticKind::BadEncryption), 0);
}

#[test]
fn signature_contents_left_alone() {
    let enc = Enc::new(4, "u", "o");
    let sig = format!(
        "<< /Type /Sig /Contents <0011> /M {} >>",
        enc.string(6, b"D:2026")
    );
    let by_range = "<< /ByteRange [0 1 2 3] /Contents <0011> >>";
    let nested = "<< /V << /Type /DocTimeStamp /Contents <0011> >> >>";
    let file = with_dict(
        &enc,
        &enc.dict(),
        &[
            (6, sig.as_bytes()),
            (7, by_range.as_bytes()),
            (8, nested.as_bytes()),
        ],
    );
    let contents = |d: &pawdf_core::Dict| d.get(b"Contents").unwrap().as_string().unwrap().to_vec();
    let six = file.resolve(r(6)).as_dict().unwrap();
    assert_eq!(contents(six), [0, 0x11]);
    assert_eq!(six.get(b"M").unwrap().as_string(), Some(&b"D:2026"[..]));
    assert_eq!(contents(file.resolve(r(7)).as_dict().unwrap()), [0, 0x11]);
    let v = file.resolve(r(8)).as_dict().unwrap().get(b"V").unwrap();
    assert_eq!(contents(v.as_dict().unwrap()), [0, 0x11]);
    assert!(matches!(six.get(b"Type"), Some(Object::Name(_))));
}

#[test]
fn embedded_file_uses_eff() {
    let enc = Enc::new(4, "u", "o");
    let dict = enc.dict().replace(
        "/StmF /StdCF /StrF /StdCF",
        "/StmF /Identity /StrF /Identity /EFF /StdCF",
    );
    let ef = enc.stream(6, "/Type /EmbeddedFile", b"attachment");
    let obj4 = raw_stream("", PLAIN);
    let file = with_dict(&enc, &dict, &[(4, &obj4), (6, &ef)]);
    assert_eq!(stream(&file, 6).raw(), b"attachment");
    assert_eq!(stream(&file, 4).raw(), PLAIN);
}

#[test]
fn broken_xref_with_encrypted_object_streams() {
    let enc = Enc::new(4, "u", "o");
    let cat = "<< /Type /Catalog /Pages 2 0 R >> ";
    let header = format!("1 0 2 {} ", cat.len());
    let members = format!("{header}{cat}<< /Type /Pages /Kids [] /Count 0 >>");
    let packed = compress_to_vec_zlib(members.as_bytes(), 6);
    let objstm = enc.stream(
        3,
        &format!(
            "/Type /ObjStm /N 2 /First {} /Filter /FlateDecode",
            header.len()
        ),
        &packed,
    );
    let dict = enc.dict();
    let (pdf, _) = build_pdf(
        &[(3, &objstm), (5, dict.as_bytes())],
        &format!("/Root 1 0 R {}", enc.trailer(5)),
    );
    let xref = xref_offset(&pdf).to_string();
    let at = pdf.len() - "\n%%EOF\n".len() - xref.len();
    let mut pdf = pdf;
    pdf.splice(at..at + xref.len(), "9".repeat(xref.len()).into_bytes());

    let file = File::from_bytes_with_password(pdf, "u").unwrap();
    let catalog = file.resolve(r(1)).as_dict().unwrap();
    assert_eq!(catalog.get(b"Pages"), Some(&Object::Ref(r(2))));
    assert!(count(&file, DiagnosticKind::XrefRepaired) > 0);
}

#[test]
fn encryption_info() {
    let enc = Enc::new(4, "u", "o");
    let pdf = encrypted(&enc);
    let info = File::from_bytes_with_password(pdf.clone(), "u")
        .unwrap()
        .encryption()
        .unwrap();
    assert_eq!(info.revision, 4);
    assert_eq!(info.method, Method::Aes128);
    assert_eq!(info.key_bits, 128);
    assert_eq!(info.authenticated_as, PasswordKind::User);
    let p = info.permissions;
    assert_eq!(p.bits(), 0xFFFFF0C0);
    assert!(!p.print() && !p.modify() && !p.copy() && !p.annotate());
    assert!(!p.fill_forms() && !p.extract_for_accessibility());
    assert!(!p.assemble() && !p.print_high_quality());

    let owner = File::from_bytes_with_password(pdf, "o")
        .unwrap()
        .encryption()
        .unwrap();
    assert_eq!(owner.authenticated_as, PasswordKind::Owner);
    assert_eq!(owner.permissions.bits(), 0xFFFFF0C0);

    let mut all = Enc::new(4, "u", "o");
    all.p = -4;
    let p = File::from_bytes_with_password(encrypted(&all), "u")
        .unwrap()
        .encryption()
        .unwrap()
        .permissions;
    assert!(p.print() && p.modify() && p.copy() && p.annotate());
    assert!(p.fill_forms() && p.extract_for_accessibility());
    assert!(p.assemble() && p.print_high_quality());

    for (rev, method, bits) in [(2, Method::Rc4, 40), (6, Method::Aes256, 256)] {
        let info = File::from_bytes_with_password(encrypted(&Enc::new(rev, "u", "o")), "u")
            .unwrap()
            .encryption()
            .unwrap();
        assert_eq!(
            (info.revision, info.method, info.key_bits),
            (rev, method, bits)
        );
    }

    let (plain, _) = build_pdf(&[(1, CATALOG), (2, PAGES)], "/Root 1 0 R");
    assert_eq!(File::from_bytes(plain).unwrap().encryption(), None);
    let (null, _) = build_pdf(&[(1, CATALOG), (2, PAGES)], "/Root 1 0 R /Encrypt null");
    assert_eq!(File::from_bytes(null).unwrap().encryption(), None);
}

#[test]
fn bad_string_padding_is_diagnostic() {
    let enc = Enc::new(4, "u", "o");
    let mut ct = enc.encrypt(6, b"title");
    *ct.last_mut().unwrap() ^= 0xFF;
    let obj6 = format!("<< /T <{}> >>", common::crypt::hex(&ct));
    let file = with_dict(&enc, &enc.dict(), &[(6, obj6.as_bytes())]);
    let t = file.resolve(r(6)).as_dict().unwrap().get(b"T").unwrap();
    assert_eq!(t.as_string().unwrap().len(), 16);
    assert_eq!(count(&file, DiagnosticKind::BadEncryption), 1);
}

#[test]
fn crypt_filter_removed_from_longer_chain() {
    let enc = Enc::new(4, "u", "o");
    let packed = compress_to_vec_zlib(PLAIN, 6);
    let hexed: String = packed
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
        + ">";
    let obj4 = raw_stream(
        "/Filter [/Crypt /AHx /FlateDecode] /DecodeParms [<< /Name /Identity >> null null]",
        hexed.as_bytes(),
    );
    let file = with_dict(&enc, &enc.dict(), &[(4, &obj4)]);
    let s = stream(&file, 4);
    let names: Vec<_> = s
        .dict
        .get(b"Filter")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o.as_name().unwrap().to_vec())
        .collect();
    assert_eq!(names, [b"AHx".to_vec(), b"FlateDecode".to_vec()]);
    assert_eq!(
        s.dict.get(b"DecodeParms"),
        Some(&Object::Array(vec![Object::Null, Object::Null]))
    );
    assert_eq!(
        s.decode(Location::File(0), &mut Diagnostics::new()).data,
        PLAIN
    );
}

/// pdfium and pdf.js follow references inside the encryption dictionary and `/ID`.
#[test]
fn indirect_values_in_encrypt_dict() {
    let enc = Enc::new(4, "u", "o");
    let dict = enc.dict();
    let cf_at = dict.find(" /CF << ").unwrap();
    let cf_end = dict[cf_at..].find(">> >>").unwrap() + cf_at + 5;
    let cf = dict[cf_at + 5..cf_end].to_string();
    let o = format!("<{}>", common::crypt::hex(&enc.o()));
    let dict = format!("{} /CF 7 0 R{}", &dict[..cf_at], &dict[cf_end..])
        .replace(&format!("/O {o}"), "/O 8 0 R");
    let id = format!("[<{0}> <{0}>]", common::crypt::hex(&enc.id));
    let obj3 = format!(
        "<< /S {} /A [ {} ] >>",
        enc.string(3, b"secret"),
        enc.string(3, b"nested")
    );
    let obj4 = enc.stream(4, "", b"BT (x) Tj ET");
    let (pdf, _) = build_pdf(
        &[
            (1, CATALOG),
            (2, PAGES),
            (3, obj3.as_bytes()),
            (4, &obj4),
            (5, dict.as_bytes()),
            (7, cf.as_bytes()),
            (8, o.as_bytes()),
            (9, id.as_bytes()),
        ],
        "/Root 1 0 R /Encrypt 5 0 R /ID 9 0 R",
    );
    let file = File::from_bytes_with_password(pdf, "u").unwrap();
    assert_plaintext(&file);
    assert_eq!(count(&file, DiagnosticKind::BadEncryption), 0);
}

/// The repair scan resolves an indirect `/DecodeParms` naming the object stream's crypt
/// filter, instead of falling back to Identity.
#[test]
fn broken_xref_object_stream_with_indirect_crypt_filter() {
    let enc = Enc::new(4, "u", "o");
    let rc4 = Enc {
        cfm: Cfm::Rc4,
        ..Enc::new(4, "u", "o")
    };
    let dict = enc
        .dict()
        .replace("/Length 16 >> >>", "/Length 16 >> /Other << /CFM /V2 >> >>");
    let cat = "<< /Type /Catalog /Pages 2 0 R >> ";
    let header = format!("1 0 2 {} ", cat.len());
    let members = format!("{header}{cat}<< /Type /Pages /Kids [] /Count 0 >>");
    let packed = compress_to_vec_zlib(members.as_bytes(), 6);
    let objstm = rc4.stream(
        3,
        &format!(
            "/Type /ObjStm /N 2 /First {} /Filter [/Crypt /FlateDecode] /DecodeParms 7 0 R",
            header.len()
        ),
        &packed,
    );
    let (pdf, _) = build_pdf(
        &[
            (3, &objstm),
            (5, dict.as_bytes()),
            (7, b"[<< /Name /Other >> null]"),
        ],
        &format!("/Root 1 0 R {}", enc.trailer(5)),
    );
    let xref = xref_offset(&pdf).to_string();
    let at = pdf.len() - "\n%%EOF\n".len() - xref.len();
    let mut pdf = pdf;
    pdf.splice(at..at + xref.len(), "9".repeat(xref.len()).into_bytes());

    let file = File::from_bytes_with_password(pdf, "u").unwrap();
    let catalog = file.resolve(r(1)).as_dict().unwrap();
    assert_eq!(catalog.get(b"Pages"), Some(&Object::Ref(r(2))));
}
