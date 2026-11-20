//! The `cad:Mesh.pbr` nested object codec (ADR-0039 D1/D2).
//!
//! One nested value, owned here rather than by the descriptor: every key is
//! always written (`null` for an absent texture) so the object has one shape and a
//! re-encode is a fixed point, while the reader still refuses a missing or
//! unknown key inside it. The parent payload (`cad:Mesh`) treats the whole `pbr`
//! key as optional — that is the ADR-0039 extension of the ADR-0038 exception —
//! but the object itself is strict.
//!
//! Split out of [`super::mesh`] for the code-size ceiling.

use serde_json::Value;

use super::{fields, PayloadError};
use crate::ymodel::json::{self, ObjectReader};
use crate::MeshPbr;

/// Encode one PBR material as its nested object.
pub(super) fn encode(pbr: &MeshPbr) -> Value {
    let texture = |key: &Option<String>| {
        key.as_ref()
            .map_or(Value::Null, |key| Value::String(key.clone()))
    };
    fields(vec![
        ("metallic", json::encode_f64(pbr.metallic)),
        ("roughness", json::encode_f64(pbr.roughness)),
        (
            "emissive",
            Value::Array(pbr.emissive.iter().map(|c| json::encode_f64(*c)).collect()),
        ),
        (
            "occlusion_strength",
            json::encode_f64(pbr.occlusion_strength),
        ),
        (
            "metallic_roughness_texture",
            texture(&pbr.metallic_roughness_texture),
        ),
        ("normal_texture", texture(&pbr.normal_texture)),
        ("emissive_texture", texture(&pbr.emissive_texture)),
        ("occlusion_texture", texture(&pbr.occlusion_texture)),
    ])
}

/// Decode the object written by [`encode`].
pub(super) fn decode(value: &Value) -> Result<MeshPbr, PayloadError> {
    let mut reader = ObjectReader::new(value, "cad:Mesh.pbr")?;
    let metallic = reader.f64("metallic")?;
    let roughness = reader.f64("roughness")?;
    let raw_emissive = reader.take("emissive")?;
    let Value::Array(channels) = &raw_emissive else {
        return Err(PayloadError::new(
            "cad:Mesh.pbr.emissive: expected a three-number array",
        ));
    };
    if channels.len() != 3 {
        return Err(PayloadError::new(
            "cad:Mesh.pbr.emissive: expected a three-number array",
        ));
    }
    let emissive = [
        json::decode_f64(&channels[0]).map_err(PayloadError::from)?,
        json::decode_f64(&channels[1]).map_err(PayloadError::from)?,
        json::decode_f64(&channels[2]).map_err(PayloadError::from)?,
    ];
    let occlusion_strength = reader.f64("occlusion_strength")?;
    let metallic_roughness_texture = reader.opt_string("metallic_roughness_texture")?;
    let normal_texture = reader.opt_string("normal_texture")?;
    let emissive_texture = reader.opt_string("emissive_texture")?;
    let occlusion_texture = reader.opt_string("occlusion_texture")?;
    reader.finish()?;
    Ok(MeshPbr {
        metallic,
        roughness,
        emissive,
        occlusion_strength,
        metallic_roughness_texture,
        normal_texture,
        emissive_texture,
        occlusion_texture,
    })
}
