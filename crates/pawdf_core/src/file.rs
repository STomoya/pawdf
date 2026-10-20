use std::cell::OnceCell;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::thread::ThreadId;

use crate::crypt::{Crypt, Encryption, SecurityHandler, authenticate};
use crate::diag::{Diagnostic, DiagnosticKind, Diagnostics, Location, Origin};
use crate::error::{Error, ErrorKind};
use crate::lexer::Lexer;
use crate::object::{Dict, Object, Ref};
use crate::parser::{find, parse_indirect, parse_object};
use crate::xref::{self, Entries, Entry};

/// Bounds reference chains and nested resolution (object streams, indirect filter values).
const MAX_RESOLVE_DEPTH: usize = 32;

static NULL: Object = Object::Null;

/// One cross-reference table with a lazily filled object cache per entry.
struct Table {
    entries: Entries,
    cache: HashMap<u32, OnceLock<Object>>,
    /// Object streams already parsed into `cache`. A member still empty afterwards is
    /// missing from its stream, and the stream is not decoded again.
    loaded_streams: Mutex<HashSet<u32>>,
    /// Object streams being parsed, per thread. Re-entering one on the same thread is a
    /// cycle. Another thread parses the stream itself instead of waiting, since waiting
    /// could deadlock on streams whose members' filters point into each other.
    loading_streams: Mutex<HashSet<(u32, ThreadId)>>,
}

impl Table {
    fn new(entries: Entries) -> Self {
        let cache = entries.keys().map(|&n| (n, OnceLock::new())).collect();
        Self {
            entries,
            cache,
            loaded_streams: Mutex::new(HashSet::new()),
            loading_streams: Mutex::new(HashSet::new()),
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A parsed PDF file: the cross-reference data plus lazy, cached object resolution.
///
/// Objects are parsed on first access. Structural problems are recorded as diagnostics
/// instead of failing.
pub struct File {
    data: Vec<u8>,
    table: Table,
    /// Built on first need when an xref entry turns out to be wrong.
    repaired: OnceLock<Table>,
    trailer: Dict,
    diags: Mutex<Diagnostics>,
    /// Object numbers whose main-table entry proved unusable; later lookups go straight
    /// to the repaired table without re-diagnosing.
    failed: Mutex<HashSet<u32>>,
    /// Object numbers found nowhere; reported once, then resolved to null directly.
    missing: Mutex<HashSet<u32>>,
    /// Loading the xref reported problems, so an absent object may have been lost with a
    /// damaged section and is worth looking for in the repaired table.
    xref_damaged: bool,
    crypt: Option<Crypt>,
}

impl File {
    /// Parses the header, cross-reference data, and trailer. An encrypted file is opened
    /// with the empty user password.
    ///
    /// Fails only if the file is encrypted and cannot be decrypted, or no document catalog
    /// can be found even after a full repair scan.
    pub fn from_bytes(data: Vec<u8>) -> Result<File, Error> {
        Self::open(data, None)
    }

    /// Like [`File::from_bytes`], for an encrypted file: tries `password` as the user, then
    /// the owner password, then the empty user password.
    pub fn from_bytes_with_password(data: Vec<u8>, password: &str) -> Result<File, Error> {
        Self::open(data, Some(password))
    }

    fn open(data: Vec<u8>, password: Option<&str>) -> Result<File, Error> {
        let mut diags = Diagnostics::new();
        let header_window = &data[..data.len().min(1024)];
        if find(header_window, b"%PDF-").is_none() {
            diags.push(DiagnosticKind::MissingHeader, Location::File(0));
        }
        let before = diags.as_slice().len();
        let loaded = xref::load(&data, &mut diags)
            .filter(|(entries, trailer, _)| root_ok(&data, entries, trailer));
        let xref_damaged = diags.as_slice().len() > before;
        let (mut entries, mut trailer, offset, repaired) = match loaded {
            Some((entries, trailer, offset)) => (entries, trailer, offset, None),
            None => {
                diags.push(DiagnosticKind::XrefRepaired, Location::File(0));
                let (entries, trailer) = xref::repair(&data, None);
                // The main table is already the repaired one; an empty fallback avoids a rescan.
                (entries, trailer, 0, Some(Table::new(Entries::new())))
            }
        };
        let fail = |kind| Error::new(kind, offset as u64);
        let crypt = match crypt_handler(&data, &entries, &trailer, &mut diags)
            .map_err(|name| fail(ErrorKind::UnsupportedEncryption(name)))?
        {
            None => None,
            Some((handler, encrypt_ref, id)) => {
                // §7.6.4.3.2 Algorithm 2 step (e): R5/R6 do not use /ID.
                if id.is_none() && handler.r <= 4 {
                    diags.push(DiagnosticKind::BadEncryption, Location::File(0));
                }
                let id = id.unwrap_or_default();
                let Some((key, kind)) = authenticate(&handler, &id, password, &mut diags) else {
                    return Err(fail(ErrorKind::IncorrectPassword {
                        password_given: password.is_some(),
                    }));
                };
                Some(Crypt::new(handler, key, kind, encrypt_ref))
            }
        };
        // The first scan could not decode encrypted object streams, so their members are
        // indexed only now that the key is known.
        if let (Some(_), Some(c)) = (&repaired, &crypt) {
            (entries, trailer) = xref::repair(&data, Some(c));
        }
        if !matches!(trailer.get(b"Root"), Some(Object::Ref(r)) if entries.contains_key(&r.num)) {
            return Err(Error::new(ErrorKind::Unrecoverable, offset as u64));
        }
        let file = File {
            data,
            table: Table::new(entries),
            repaired: OnceLock::new(),
            trailer,
            diags: Mutex::new(diags),
            failed: Mutex::new(HashSet::new()),
            missing: Mutex::new(HashSet::new()),
            xref_damaged,
            crypt,
        };
        if let Some(t) = repaired {
            let _ = file.repaired.set(t);
        }
        Ok(file)
    }

    /// How the file was decrypted, or `None` if it is not encrypted.
    pub fn encryption(&self) -> Option<Encryption> {
        self.crypt.as_ref().map(Crypt::info)
    }

    /// The (merged) trailer dictionary.
    pub fn trailer(&self) -> &Dict {
        &self.trailer
    }

    /// The raw file bytes.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// All object references known to the cross-reference data, in-use entries only.
    pub fn objects(&self) -> Vec<Ref> {
        self.table
            .entries
            .keys()
            .map(|&num| Ref { num, generation: 0 })
            .collect()
    }

    /// Structural diagnostics collected so far.
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        self.lock_diags().as_slice().to_vec()
    }

    /// Resolves a reference, following reference chains. Missing objects resolve to null
    /// (§7.3.10). Generation numbers are not checked, which tolerates writers that get
    /// them wrong.
    pub fn resolve(&self, r: Ref) -> &Object {
        self.resolve_depth(r, 0)
    }

    /// `obj` itself, or the object it references.
    pub fn deref<'a>(&'a self, obj: &'a Object) -> &'a Object {
        match obj {
            Object::Ref(r) => self.resolve(*r),
            o => o,
        }
    }

    fn lock_diags(&self) -> std::sync::MutexGuard<'_, Diagnostics> {
        lock(&self.diags)
    }

    fn report(&self, kind: DiagnosticKind, location: Location) {
        self.lock_diags().push(kind, location);
    }

    fn resolve_depth(&self, r: Ref, depth: usize) -> &Object {
        if depth > MAX_RESOLVE_DEPTH {
            self.report(DiagnosticKind::RecursionLimit, Location::File(0));
            return &NULL;
        }
        let obj = self.find(r, depth).unwrap_or(&NULL);
        match obj {
            Object::Ref(next) => self.resolve_depth(*next, depth + 1),
            o => o,
        }
    }

    /// The object for `r.num`. In an undamaged xref, a number that is absent (or free) is
    /// simply missing, without a repair scan (§7.3.10); only an entry that proves wrong, or
    /// any absence in a damaged xref, falls back to the repaired table. Each failure is
    /// diagnosed once.
    fn find(&self, r: Ref, depth: usize) -> Option<&Object> {
        if lock(&self.missing).contains(&r.num) {
            return None;
        }
        let listed = self.table.entries.contains_key(&r.num);
        if listed && !lock(&self.failed).contains(&r.num) {
            if let Some(o) = self.lookup(&self.table, r, depth) {
                return Some(o);
            }
            lock(&self.failed).insert(r.num);
        }
        if (listed || self.xref_damaged)
            && let Some(o) = self.lookup(self.repaired_table(), r, depth)
        {
            return Some(o);
        }
        if lock(&self.missing).insert(r.num) {
            self.report(DiagnosticKind::ObjectNotFound, Location::File(0));
        }
        None
    }

    fn repaired_table(&self) -> &Table {
        self.repaired.get_or_init(|| {
            self.report(DiagnosticKind::XrefRepaired, Location::File(0));
            Table::new(xref::repair(&self.data, self.crypt.as_ref()).0)
        })
    }

    /// `None` when `table` has no usable entry for `r`, so the caller can fall back to
    /// the repaired table.
    fn lookup<'a>(&'a self, table: &'a Table, r: Ref, depth: usize) -> Option<&'a Object> {
        let slot = table.cache.get(&r.num)?;
        if let Some(o) = slot.get() {
            return Some(o);
        }
        // Computed outside the OnceLock: resolution can re-enter (indirect /Length, object
        // streams), and `get_or_init` must not be called re-entrantly.
        let obj = match *table.entries.get(&r.num)? {
            Entry::InFile { offset } => self.load_in_file(r.num, offset, depth)?,
            Entry::InStream { stream, .. } => {
                let key = (stream, std::thread::current().id());
                if !lock(&table.loaded_streams).contains(&stream)
                    && lock(&table.loading_streams).insert(key)
                {
                    self.load_object_stream(table, stream, depth);
                    lock(&table.loading_streams).remove(&key);
                }
                return slot.get();
            }
        };
        let _ = slot.set(obj);
        slot.get()
    }

    fn load_in_file(&self, num: u32, offset: usize, depth: usize) -> Option<Object> {
        if !xref::header_matches(&self.data, offset, num) {
            self.report(
                DiagnosticKind::ObjectMismatch,
                Location::File(offset as u64),
            );
            return None;
        }
        let mut diags = Diagnostics::new();
        let length_of = |len_ref: Ref| self.length_of(len_ref);
        let (header, mut obj) = parse_indirect(&self.data, offset, &length_of, &mut diags)?;
        let at = Location::File(offset as u64);
        // §7.6.2: the key comes from the `N G obj` header, since references' generations
        // are not checked.
        let crypt = self.crypt.as_ref().filter(|c| !c.is_encrypt_dict(num));
        // §7.5.8.2: a cross-reference stream's dictionary is not encrypted either.
        let is_xref = matches!(&obj, Object::Stream(s) if s.dict.get(b"Type").and_then(Object::as_name) == Some(b"XRef"));
        if let Some(c) = crypt.filter(|_| !is_xref) {
            c.decrypt_strings(header, &mut obj, at, &mut diags);
        }
        self.lock_diags().append(diags);
        if let Object::Stream(s) = &mut obj {
            self.resolve_filter_values(&mut s.dict, depth);
            if let Some(c) = crypt {
                let mut diags = Diagnostics::new();
                c.decrypt_stream(header, s, at, &mut diags);
                self.lock_diags().append(diags);
            }
        }
        Some(obj)
    }

    /// An indirect stream `/Length`. Only direct objects in the main table are consulted,
    /// so this cannot recurse.
    fn length_of(&self, r: Ref) -> Option<i64> {
        let Some(Entry::InFile { offset }) = self.table.entries.get(&r.num).copied() else {
            return None;
        };
        let mut lex = Lexer::at(&self.data, offset);
        if crate::parser::parse_header(&mut lex)?.num != r.num {
            return None;
        }
        parse_object(&mut lex, Origin::File, &mut Diagnostics::new())?.as_int()
    }

    /// Replaces indirect `/Filter` and `/DecodeParms` values (and their array elements) so
    /// `Stream::decode` can run without file access.
    fn resolve_filter_values(&self, dict: &mut Dict, depth: usize) {
        for key in [&b"Filter"[..], b"DecodeParms"] {
            let Some(value) = dict.get_mut(key) else {
                continue;
            };
            if let Object::Ref(r) = value {
                *value = self.resolve_depth(*r, depth + 1).clone();
            }
            if let Object::Array(items) = value {
                for item in items {
                    if let Object::Ref(r) = item {
                        *item = self.resolve_depth(*r, depth + 1).clone();
                    }
                }
            }
        }
    }

    /// Parses every member of object stream `stream_num` into `table`'s cache (§7.5.7).
    fn load_object_stream(&self, table: &Table, stream_num: u32, depth: usize) {
        let stream_ref = Ref {
            num: stream_num,
            generation: 0,
        };
        let Object::Stream(stream) = self.resolve_depth(stream_ref, depth + 1) else {
            if lock(&table.loaded_streams).insert(stream_num) {
                self.report(DiagnosticKind::ObjectNotFound, Location::File(0));
            }
            return;
        };
        let mut diags = Diagnostics::new();
        let decoded = stream
            .decode(
                Location::Stream {
                    obj: stream_ref,
                    offset: 0,
                },
                &mut diags,
            )
            .data;
        let first = stream
            .dict
            .get(b"First")
            .and_then(|o| o.as_int())
            .unwrap_or(0);
        let first = usize::try_from(first).unwrap_or(0);
        let origin = Origin::Stream(stream_ref);
        for (num, off) in xref::objstm_header(&stream.dict, &decoded) {
            let belongs = matches!(table.entries.get(&num), Some(Entry::InStream { stream, .. }) if *stream == stream_num);
            let Some(slot) = table.cache.get(&num).filter(|_| belongs) else {
                continue;
            };
            if slot.get().is_some() {
                continue;
            }
            let pos = first.saturating_add(off);
            let obj = match parse_object(&mut Lexer::at(&decoded, pos), origin, &mut diags) {
                Some(o) => o,
                None => {
                    diags.push(DiagnosticKind::BadObject, origin.at(pos));
                    Object::Null
                }
            };
            let _ = slot.set(obj);
        }
        // Only the first thread to finish reports, so a race does not duplicate diagnostics.
        if lock(&table.loaded_streams).insert(stream_num) {
            self.lock_diags().append(diags);
        }
    }
}

/// Loads objects without decryption, for the encryption dictionary and `/ID`, which are
/// read before the key exists. They are never in object streams (§7.5.7), so one hop
/// through the main table, then a repair scan, finds them.
struct RawObjects<'a> {
    data: &'a [u8],
    entries: &'a Entries,
    repaired: OnceCell<Entries>,
}

impl RawObjects<'_> {
    fn get(&self, num: u32) -> Option<Object> {
        let at = |entries: &Entries| match entries.get(&num) {
            Some(Entry::InFile { offset }) => {
                parse_indirect(self.data, *offset, &|_| None, &mut Diagnostics::new())
                    .filter(|(found, _)| found.num == num)
                    .map(|(_, o)| o)
            }
            _ => None,
        };
        at(self.entries).or_else(|| {
            at(self
                .repaired
                .get_or_init(|| xref::repair(self.data, None).0))
        })
    }

    /// `obj`, or the object it references (null if missing).
    fn deref(&self, obj: &Object) -> Object {
        match obj {
            Object::Ref(r) => self.get(r.num).unwrap_or(Object::Null),
            o => o.clone(),
        }
    }
}

/// The parsed handler, the encryption dictionary's reference, and the first `/ID` string.
type EncryptInfo = (SecurityHandler, Option<Ref>, Option<Vec<u8>>);

/// The trailer's `/Encrypt` dictionary, parsed without decryption, its reference when
/// indirect, and the first `/ID` string. Indirect values in the dictionary, its crypt
/// filters and `/ID` are followed one level, as mature readers do. A value that is not a
/// dictionary is diagnosed and the file read as unencrypted. `Err` names an unsupported
/// handler.
fn crypt_handler(
    data: &[u8],
    entries: &Entries,
    trailer: &Dict,
    diags: &mut Diagnostics,
) -> Result<Option<EncryptInfo>, String> {
    let Some(value) = trailer.get(b"Encrypt") else {
        return Ok(None);
    };
    let raw = RawObjects {
        data,
        entries,
        repaired: OnceCell::new(),
    };
    let Object::Dict(mut dict) = raw.deref(value) else {
        diags.push(DiagnosticKind::BadEncryption, Location::File(0));
        return Ok(None);
    };
    for (_, v) in dict.iter_mut() {
        *v = raw.deref(v);
    }
    if let Some(Object::Dict(cf)) = dict.get_mut(b"CF") {
        for (_, filter) in cf.iter_mut() {
            *filter = raw.deref(filter);
            if let Object::Dict(f) = filter {
                for (_, v) in f.iter_mut() {
                    *v = raw.deref(v);
                }
            }
        }
    }
    let id = trailer
        .get(b"ID")
        .map(|id| raw.deref(id))
        .and_then(|id| {
            id.as_array()
                .and_then(|a| a.first())
                .map(|first| raw.deref(first))
        })
        .and_then(|first| first.as_string().map(<[u8]>::to_vec));
    let handler = SecurityHandler::parse(&dict, diags)?;
    Ok(Some((handler, value.as_ref(), id)))
}

/// The trailer's `/Root` exists and its xref entry points at a real object.
fn root_ok(data: &[u8], entries: &Entries, trailer: &Dict) -> bool {
    let Some(Object::Ref(root)) = trailer.get(b"Root") else {
        return false;
    };
    match entries.get(&root.num) {
        Some(Entry::InFile { offset }) => xref::header_matches(data, *offset, root.num),
        Some(Entry::InStream { .. }) => true,
        _ => false,
    }
}
