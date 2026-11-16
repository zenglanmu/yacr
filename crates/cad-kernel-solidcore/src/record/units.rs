//! Number hygiene at the record boundary: units, sign of zero and finiteness.
//!
//! The upstream record is millimetres, radians, right-handed Z-up. The neutral
//! B-rep carries drawing units with no declared scale, so the seam converts
//! **nothing**: one drawing unit is one record unit. A document drawn in metres
//! is handed to the kernel as millimetre numbers, and volumes/areas come back in
//! that document's units — the modeler never re-labels them as mm³/mm².
//!
//! `-0.0` is normalised to `0.0` on the way in: the record's canonical-order and
//! vertex-dedup rules compare exact bits, and `(-0.0).to_bits() != 0.0f64.to_bits()`.
//! The upstream kernel compares doubles exactly in places (`float_cmp` is allowed
//! upstream), so a signed zero produced by a rotation must not create a second
//! vertex.

use cad_domain::Point3;

/// Normalise `-0.0` to `0.0`; leave every other value (including NaN) alone so
/// the finiteness checks still see it.
pub(crate) fn clean(value: f64) -> f64 {
    if value == 0.0 {
        0.0
    } else {
        value
    }
}

/// A neutral point or direction as a record triple.
pub(crate) fn record_point(p: Point3) -> [f64; 3] {
    [clean(p.x), clean(p.y), clean(p.z)]
}

/// A record triple as a neutral point, with `-0.0` normalised.
pub(crate) fn point(values: [f64; 3]) -> Point3 {
    Point3 {
        x: clean(values[0]),
        y: clean(values[1]),
        z: clean(values[2]),
    }
}

/// The exact bit pattern of a record triple, used as a hash key so vertex and
/// edge dedup follows the record's own exact-equality rule.
pub(crate) fn bits(values: [f64; 3]) -> [u64; 3] {
    [
        clean(values[0]).to_bits(),
        clean(values[1]).to_bits(),
        clean(values[2]).to_bits(),
    ]
}
