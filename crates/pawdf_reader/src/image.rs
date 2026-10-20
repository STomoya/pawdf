//! Image XObjects and inline images (§8.9).

use pawdf_core::{
    Codec, DiagnosticKind, Diagnostics, Dict, File, Location, MAX_DECODED_LEN, Name, Object, Stream,
};

use crate::content::color::Space;
use crate::model::{CcittParams, ColorSpace, ImageData, SoftMask};

/// Everything of an [`crate::model::Image`] that does not depend on where it is placed.
#[derive(Clone)]
pub(crate) struct ImageParts {
    pub width: u32,
    pub height: u32,
    pub bpc: u8,
    pub color_space: Option<ColorSpace>,
    pub image_mask: bool,
    pub decode: Option<Vec<f32>>,
    pub data: ImageData,
    pub smask: Option<SoftMask>,
}

impl ImageParts {
    /// Bytes of image and soft-mask data.
    pub fn data_len(&self) -> usize {
        let len = |d: &ImageData| match d {
            ImageData::Samples(b) | ImageData::Jpeg(b) | ImageData::Jpx(b) => b.len(),
            ImageData::Jbig2 { data, globals } => data.len() + globals.as_ref().map_or(0, Vec::len),
            ImageData::Ccitt { data, .. } => data.len(),
        };
        len(&self.data) + self.smask.as_ref().map_or(0, |m| len(&m.data))
    }
}

/// Expands inline-image abbreviations (§8.9.7, Table 91) to full key names.
pub(crate) fn expand_inline(dict: &Dict) -> Dict {
    let mut out = Dict::new();
    for (k, v) in dict.iter() {
        let full: &[u8] = match k.as_bytes() {
            b"BPC" => b"BitsPerComponent",
            b"CS" => b"ColorSpace",
            b"D" => b"Decode",
            b"DP" => b"DecodeParms",
            b"F" => b"Filter",
            b"H" => b"Height",
            b"IM" => b"ImageMask",
            b"I" => b"Interpolate",
            b"W" => b"Width",
            b"L" => b"Length",
            other => other,
        };
        out.insert(Name(full.to_vec()), v.clone());
    }
    out
}

/// Reads an image stream whose dictionary has full key names. `space` resolves the
/// `/ColorSpace` value (names may need the current resources). `None`, with a
/// `BadImage`, when the dimensions are unusable.
pub(crate) fn read_image(
    file: &File,
    stream: &Stream,
    at: Location,
    space: &mut dyn FnMut(&Object, &mut Diagnostics) -> Option<Space>,
    with_smask: bool,
    diags: &mut Diagnostics,
) -> Option<ImageParts> {
    let dict = &stream.dict;
    let get = |k: &[u8]| dict.get(k).map(|o| file.deref(o));
    let int = |k: &[u8]| get(k).and_then(Object::as_int);
    let dim = |k: &[u8]| {
        int(k)
            .and_then(|v| u32::try_from(v).ok())
            .filter(|&v| v > 0)
    };
    let (Some(width), Some(height)) = (dim(b"Width"), dim(b"Height")) else {
        diags.push(DiagnosticKind::BadImage, at);
        return None;
    };
    let image_mask = get(b"ImageMask").and_then(Object::as_bool) == Some(true);
    let is_jpx = filter_names(file, dict).iter().any(|f| f == b"JPXDecode");
    let bpc = if image_mask {
        1
    } else {
        match int(b"BitsPerComponent").and_then(|b| u8::try_from(b).ok()) {
            Some(b @ (1 | 2 | 4 | 8 | 16)) => b,
            // §8.9.5: JPX images may omit it; the codestream carries it.
            None if is_jpx => 0,
            _ => {
                diags.push(DiagnosticKind::BadImage, at);
                8
            }
        }
    };
    let resolved = match (image_mask, dict.get(b"ColorSpace")) {
        (false, Some(cs)) => {
            let s = space(cs, diags);
            if s.is_none() {
                diags.push(DiagnosticKind::MissingResource, at);
            }
            s
        }
        _ => None,
    };
    let components = match &resolved {
        Some(s) => s.components().max(1),
        None => 1,
    };
    let expected = u64::from(width)
        .checked_mul(components as u64)
        .and_then(|v| v.checked_mul(u64::from(bpc)))
        .map(|bits| bits.div_ceil(8))
        .and_then(|row| row.checked_mul(u64::from(height)));
    let Some(expected) = expected.filter(|&n| n <= MAX_DECODED_LEN as u64) else {
        diags.push(DiagnosticKind::BadImage, at);
        return None;
    };
    let decoded = stream.decode(at, diags);
    let data = match decoded.codec {
        None => {
            if (decoded.data.len() as u64) < expected {
                diags.push(DiagnosticKind::BadImage, at);
            }
            ImageData::Samples(decoded.data)
        }
        Some((Codec::Dct, _)) => ImageData::Jpeg(decoded.data),
        Some((Codec::Jpx, _)) => ImageData::Jpx(decoded.data),
        Some((Codec::Jbig2, params)) => {
            let globals = match params.get(b"JBIG2Globals") {
                Some(Object::Ref(r)) => match file.resolve(*r) {
                    Object::Stream(s) => Some(
                        s.decode(Location::Stream { obj: *r, offset: 0 }, diags)
                            .data,
                    ),
                    _ => {
                        diags.push(DiagnosticKind::BadImage, at);
                        None
                    }
                },
                _ => None,
            };
            ImageData::Jbig2 {
                data: decoded.data,
                globals,
            }
        }
        Some((Codec::Ccitt, params)) => ImageData::Ccitt {
            data: decoded.data,
            params: ccitt_params(file, &params),
        },
    };
    let decode = get(b"Decode").and_then(Object::as_array).map(|a| {
        let nums: Vec<f32> = a
            .iter()
            .filter_map(|o| file.deref(o).as_number())
            .map(|n| n as f32)
            .collect();
        if nums.len() < a.len() {
            diags.push(DiagnosticKind::BadImage, at);
        }
        nums
    });
    let smask = match dict.get(b"SMask") {
        Some(Object::Ref(r)) if with_smask => match file.resolve(*r) {
            Object::Stream(s) => {
                let at = Location::Stream { obj: *r, offset: 0 };
                let mut gray = |_: &Object, _: &mut Diagnostics| Some(Space::Gray);
                read_image(file, s, at, &mut gray, false, diags).map(|m| SoftMask {
                    width: m.width,
                    height: m.height,
                    bpc: m.bpc,
                    decode: m.decode,
                    data: m.data,
                })
            }
            _ => {
                diags.push(DiagnosticKind::BadImage, at);
                None
            }
        },
        _ => None,
    };
    Some(ImageParts {
        width,
        height,
        bpc,
        color_space: resolved.as_ref().and_then(public_space),
        image_mask,
        decode,
        data,
        smask,
    })
}

fn filter_names(file: &File, dict: &Dict) -> Vec<Vec<u8>> {
    match dict.get(b"Filter").map(|o| file.deref(o)) {
        Some(Object::Name(n)) => vec![n.as_bytes().to_vec()],
        Some(Object::Array(a)) => a
            .iter()
            .filter_map(|o| o.as_name().map(<[u8]>::to_vec))
            .collect(),
        _ => Vec::new(),
    }
}

/// §7.4.6, Table 11, with its defaults.
fn ccitt_params(file: &File, p: &Dict) -> CcittParams {
    let int = |k: &[u8], d: i64| p.get(k).and_then(|o| file.deref(o).as_int()).unwrap_or(d);
    let flag = |k: &[u8], d: bool| p.get(k).and_then(|o| file.deref(o).as_bool()).unwrap_or(d);
    CcittParams {
        k: int(b"K", 0).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        columns: u32::try_from(int(b"Columns", 1728)).unwrap_or(1728),
        rows: u32::try_from(int(b"Rows", 0)).unwrap_or(0),
        black_is_1: flag(b"BlackIs1", false),
        encoded_byte_align: flag(b"EncodedByteAlign", false),
        end_of_line: flag(b"EndOfLine", false),
        end_of_block: flag(b"EndOfBlock", true),
    }
}

fn public_space(s: &Space) -> Option<ColorSpace> {
    Some(match s {
        Space::Gray => ColorSpace::Gray,
        Space::Rgb => ColorSpace::Rgb,
        Space::Cmyk => ColorSpace::Cmyk,
        Space::Pattern => return None,
        Space::Indexed {
            base,
            hival,
            lookup,
        } => ColorSpace::Indexed {
            base: Box::new(public_space(base)?),
            hival: *hival,
            lookup: lookup.to_vec(),
        },
        Space::Other { family, n } => ColorSpace::Other {
            family: *family,
            components: u8::try_from(*n).unwrap_or(u8::MAX),
        },
    })
}
