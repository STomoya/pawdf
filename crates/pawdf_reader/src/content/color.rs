//! Color spaces and color operators (§8.6).

use std::sync::Arc;

use pawdf_core::{Diagnostics, File, Location, Object};

use crate::model::{Color, ColorSpaceFamily};

const MAX_SPACE_DEPTH: usize = 4;

/// A resolved color space.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Space {
    Gray,
    Rgb,
    Cmyk,
    Pattern,
    Indexed {
        base: Box<Space>,
        hival: u8,
        /// Shared: the graphics state is cloned on every `q`.
        lookup: Arc<[u8]>,
    },
    Other {
        family: ColorSpaceFamily,
        n: usize,
    },
}

impl Space {
    /// Number of color components; a pattern takes a name instead.
    pub fn components(&self) -> usize {
        match self {
            Space::Gray | Space::Indexed { .. } => 1,
            Space::Rgb => 3,
            Space::Cmyk => 4,
            Space::Pattern => 0,
            Space::Other { n, .. } => *n,
        }
    }

    /// The initial color after selecting this space (§8.6.8).
    pub fn initial_color(&self) -> Color {
        match self {
            Space::Gray => Color::Gray(0.),
            Space::Rgb => Color::Rgb([0.; 3]),
            Space::Cmyk => Color::Cmyk([0., 0., 0., 1.]),
            Space::Pattern => Color::Pattern(String::new()),
            Space::Indexed { .. } => Color::Other {
                family: ColorSpaceFamily::Indexed,
                components: vec![0.],
            },
            Space::Other { family, n } => Color::Other {
                family: *family,
                components: vec![0.; *n],
            },
        }
    }

    /// The color for `sc`/`scn` operands, or `None` if they do not fit this space.
    pub fn color(&self, ops: &[Object]) -> Option<Color> {
        if *self == Space::Pattern {
            let n = ops.last()?.as_name()?;
            return Some(Color::Pattern(String::from_utf8_lossy(n).into_owned()));
        }
        if ops.len() != self.components() {
            return None;
        }
        let mut c = Vec::with_capacity(ops.len());
        for o in ops {
            c.push(o.as_number().filter(|n| n.is_finite())? as f32);
        }
        Some(match (self, c.as_slice()) {
            (Space::Gray, &[g]) => Color::Gray(g),
            (Space::Rgb, &[r, g, b]) => Color::Rgb([r, g, b]),
            (Space::Cmyk, &[c0, m, y, k]) => Color::Cmyk([c0, m, y, k]),
            (Space::Indexed { .. }, _) => Color::Other {
                family: ColorSpaceFamily::Indexed,
                components: c,
            },
            (Space::Other { family, .. }, _) => Color::Other {
                family: *family,
                components: c,
            },
            _ => return None,
        })
    }
}

/// A device or pattern space named directly in content (§8.6.3), including the
/// inline-image abbreviations (§8.9.7, Table 92).
pub(crate) fn device_space(name: &[u8]) -> Option<Space> {
    Some(match name {
        b"DeviceGray" | b"G" => Space::Gray,
        b"DeviceRGB" | b"RGB" => Space::Rgb,
        b"DeviceCMYK" | b"CMYK" => Space::Cmyk,
        b"Pattern" => Space::Pattern,
        _ => return None,
    })
}

/// Resolves a color space object: a device name or a family array. Names other than
/// device names must already have been looked up in `/ColorSpace` by the caller.
pub(crate) fn resolve_space(file: &File, obj: &Object, diags: &mut Diagnostics) -> Option<Space> {
    resolve_depth(file, obj, diags, 0)
}

fn resolve_depth(
    file: &File,
    obj: &Object,
    diags: &mut Diagnostics,
    depth: usize,
) -> Option<Space> {
    if depth > MAX_SPACE_DEPTH {
        return None;
    }
    let obj = file.deref(obj);
    if let Some(n) = obj.as_name() {
        return device_space(n);
    }
    let arr = obj.as_array()?;
    let family = file.deref(arr.first()?).as_name()?;
    let arg = |i: usize| arr.get(i).map(|o| file.deref(o));
    let other = |family, n| Some(Space::Other { family, n });
    match family {
        b"ICCBased" => {
            let n = arg(1)?
                .as_stream()?
                .dict
                .get(b"N")
                .and_then(|n| file.deref(n).as_int());
            match n {
                Some(1) => Some(Space::Gray),
                Some(3) => Some(Space::Rgb),
                Some(4) => Some(Space::Cmyk),
                Some(n) => other(ColorSpaceFamily::Unknown, usize::try_from(n).ok()?.min(32)),
                None => None,
            }
        }
        b"CalGray" => other(ColorSpaceFamily::CalGray, 1),
        b"CalRGB" => other(ColorSpaceFamily::CalRgb, 3),
        b"Lab" => other(ColorSpaceFamily::Lab, 3),
        b"Separation" => other(ColorSpaceFamily::Separation, 1),
        b"DeviceN" => other(ColorSpaceFamily::DeviceN, arg(1)?.as_array()?.len().min(32)),
        b"Indexed" | b"I" => {
            let base = resolve_depth(file, arr.get(1)?, diags, depth + 1)?;
            let hival = u8::try_from(arg(2)?.as_int()?.clamp(0, 255)).ok()?;
            let lookup = match arr.get(3)? {
                Object::String(s) => s.clone(),
                Object::Ref(r) => match file.resolve(*r) {
                    Object::String(s) => s.clone(),
                    Object::Stream(s) => {
                        let at = Location::Stream { obj: *r, offset: 0 };
                        s.decode(at, diags).data
                    }
                    _ => return None,
                },
                _ => return None,
            };
            // §8.6.6.3: only (hival + 1) × components bytes are meaningful.
            let used = (usize::from(hival) + 1) * base.components();
            let mut lookup = lookup;
            lookup.truncate(used);
            Some(Space::Indexed {
                base: Box::new(base),
                hival,
                lookup: lookup.into(),
            })
        }
        b"Pattern" => Some(Space::Pattern),
        n => device_space(n),
    }
}
