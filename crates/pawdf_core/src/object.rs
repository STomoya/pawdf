use std::collections::HashSet;

/// An indirect object reference (`num gen R`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Ref {
    /// Object number.
    pub num: u32,
    /// Generation number. Not checked during resolution.
    pub generation: u16,
}

/// A PDF name, as raw bytes after `#xx` unescaping. Names need not be UTF-8.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Name(pub Vec<u8>);

impl Name {
    /// The name bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl From<&[u8]> for Name {
    fn from(b: &[u8]) -> Self {
        Name(b.to_vec())
    }
}

impl From<&str> for Name {
    fn from(s: &str) -> Self {
        Name(s.as_bytes().to_vec())
    }
}

/// A PDF object.
#[derive(Debug, Clone, PartialEq)]
pub enum Object {
    /// `null`, also the value of unresolvable references.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// Integer.
    Int(i64),
    /// Real number.
    Real(f64),
    /// String bytes after escape or hex decoding.
    String(Vec<u8>),
    /// Name.
    Name(Name),
    /// Array.
    Array(Vec<Object>),
    /// Dictionary.
    Dict(Dict),
    /// Stream. Only appears as a top-level indirect object.
    Stream(Stream),
    /// Indirect reference.
    Ref(Ref),
}

impl Object {
    /// Integer value. Integral reals are accepted because some writers emit `/Length 100.0`.
    pub fn as_int(&self) -> Option<i64> {
        match *self {
            Object::Int(i) => Some(i),
            Object::Real(r) if r.fract() == 0.0 && r.abs() < 9.0e15 => Some(r as i64),
            _ => None,
        }
    }

    /// Integer or real value as `f64`.
    pub fn as_number(&self) -> Option<f64> {
        match *self {
            Object::Int(i) => Some(i as f64),
            Object::Real(r) => Some(r),
            _ => None,
        }
    }

    /// Boolean value.
    pub fn as_bool(&self) -> Option<bool> {
        match *self {
            Object::Bool(b) => Some(b),
            _ => None,
        }
    }

    /// Name bytes.
    pub fn as_name(&self) -> Option<&[u8]> {
        match self {
            Object::Name(n) => Some(n.as_bytes()),
            _ => None,
        }
    }

    /// String bytes.
    pub fn as_string(&self) -> Option<&[u8]> {
        match self {
            Object::String(s) => Some(s),
            _ => None,
        }
    }

    /// Array elements.
    pub fn as_array(&self) -> Option<&[Object]> {
        match self {
            Object::Array(a) => Some(a),
            _ => None,
        }
    }

    /// Dictionary. A stream's dictionary is not returned; use [`Object::as_stream`].
    pub fn as_dict(&self) -> Option<&Dict> {
        match self {
            Object::Dict(d) => Some(d),
            _ => None,
        }
    }

    /// Stream.
    pub fn as_stream(&self) -> Option<&Stream> {
        match self {
            Object::Stream(s) => Some(s),
            _ => None,
        }
    }

    /// Reference.
    pub fn as_ref(&self) -> Option<Ref> {
        match *self {
            Object::Ref(r) => Some(r),
            _ => None,
        }
    }
}

/// A PDF dictionary. Keys are unique; inserting an existing key replaces its value.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Dict(Vec<(Name, Object)>);

impl Dict {
    /// An empty dictionary.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a dictionary from parsed entries; for a repeated key the last value wins.
    /// Deduplicating once keeps parsing linear in the number of keys.
    pub(crate) fn from_entries(entries: Vec<(Name, Object)>) -> Self {
        let mut seen = HashSet::new();
        let mut kept: Vec<(Name, Object)> = entries
            .into_iter()
            .rev()
            .filter(|(k, _)| seen.insert(k.0.clone()))
            .collect();
        kept.reverse();
        Dict(kept)
    }

    /// Wraps entries whose keys the caller already checked are unique.
    pub(crate) fn from_unique(entries: Vec<(Name, Object)>) -> Self {
        Dict(entries)
    }

    /// Value for `key`.
    pub fn get(&self, key: &[u8]) -> Option<&Object> {
        self.0
            .iter()
            .find(|(k, _)| k.as_bytes() == key)
            .map(|(_, v)| v)
    }

    /// Mutable value for `key`.
    pub fn get_mut(&mut self, key: &[u8]) -> Option<&mut Object> {
        self.0
            .iter_mut()
            .find(|(k, _)| k.as_bytes() == key)
            .map(|(_, v)| v)
    }

    /// Whether `key` is present.
    pub fn contains_key(&self, key: &[u8]) -> bool {
        self.get(key).is_some()
    }

    /// Inserts or replaces `key`.
    pub fn insert(&mut self, key: Name, value: Object) {
        match self.0.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = value,
            None => self.0.push((key, value)),
        }
    }

    pub(crate) fn remove(&mut self, key: &[u8]) -> Option<Object> {
        let i = self.0.iter().position(|(k, _)| k.as_bytes() == key)?;
        Some(self.0.remove(i).1)
    }

    pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = (&Name, &mut Object)> {
        self.0.iter_mut().map(|(k, v)| (&*k, v))
    }

    pub(crate) fn into_entries(self) -> Vec<(Name, Object)> {
        self.0
    }

    /// Entries in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&Name, &Object)> {
        self.0.iter().map(|(k, v)| (k, v))
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are no entries.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A stream object: its dictionary and its raw (still filtered) bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct Stream {
    /// The stream dictionary.
    pub dict: Dict,
    pub(crate) raw: Vec<u8>,
}

impl Stream {
    /// A stream from its dictionary and raw bytes.
    pub fn new(dict: Dict, raw: Vec<u8>) -> Self {
        Self { dict, raw }
    }

    /// The bytes between `stream` and `endstream`, after decryption and before any filter
    /// is applied.
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dict_insert_replaces() {
        let mut d = Dict::new();
        d.insert("A".into(), Object::Int(1));
        d.insert("A".into(), Object::Int(2));
        assert_eq!(d.len(), 1);
        assert_eq!(d.get(b"A"), Some(&Object::Int(2)));
        assert_eq!(d.get(b"B"), None);
    }

    #[test]
    fn as_int_accepts_integral_reals_only() {
        assert_eq!(Object::Int(5).as_int(), Some(5));
        assert_eq!(Object::Real(100.0).as_int(), Some(100));
        assert_eq!(Object::Real(1.5).as_int(), None);
        assert_eq!(Object::Real(f64::NAN).as_int(), None);
        assert_eq!(Object::Null.as_int(), None);
    }
}
