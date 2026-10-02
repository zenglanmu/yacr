//! Space-id encoding and strict decoding (audit B11).

use super::*;

pub(crate) fn encode_space(space: &SpaceId) -> Value {
    match space {
        SpaceId::Model => json!("Model"),
        SpaceId::Paper(id) => json!({ "Paper": id.0.to_string() }),
        SpaceId::Block(id) => json!({ "Block": id.0.to_string() }),
    }
}

/// Decode a space id strictly: a malformed or unknown space is corruption, not
/// a silent `Model`/`Paper(0)` (audit B11).
pub(crate) fn decode_space(value: Option<&Value>) -> CadResult<SpaceId> {
    match value {
        Some(Value::String(s)) if s == "Model" => Ok(SpaceId::Model),
        Some(Value::Object(m)) => {
            if let Some(block) = m.get("Block") {
                let raw = block
                    .as_str()
                    .ok_or_else(|| corrupt("space 'Block' id must be a string"))?;
                let id = raw
                    .parse::<u128>()
                    .map_err(|_| corrupt("space 'Block' id is not a valid integer"))?;
                return Ok(SpaceId::Block(BlockId(id)));
            }
            if let Some(paper) = m.get("Paper") {
                let raw = paper
                    .as_str()
                    .ok_or_else(|| corrupt("space 'Paper' id must be a string"))?;
                let id = raw
                    .parse::<u128>()
                    .map_err(|_| corrupt("space 'Paper' id is not a valid integer"))?;
                return Ok(SpaceId::Paper(LayoutId(id)));
            }
            Err(corrupt("space object must name 'Paper' or 'Block'"))
        }
        Some(_) => Err(corrupt("space must be \"Model\" or an object")),
        None => Err(corrupt("missing required field 'space'")),
    }
}
