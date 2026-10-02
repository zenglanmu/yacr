//! View-bookmark codec.

use super::*;

pub(crate) fn encode_bookmark(bookmark: &ViewBookmark) -> Value {
    json!({
        "name": bookmark.name,
        "viewport": bookmark.viewport.0.to_string(),
        "space": encode_space(&bookmark.space),
        "camera_transform": encode_transform(&bookmark.camera_transform),
    })
}

pub(crate) fn decode_bookmark(value: &Value) -> CadResult<ViewBookmark> {
    let object = value
        .as_object()
        .ok_or_else(|| corrupt("view bookmark must be an object"))?;
    let viewport_raw = require_str(object, "viewport")?;
    let viewport = viewport_raw
        .parse::<u128>()
        .map_err(|_| corrupt("view bookmark 'viewport' is not a valid integer"))?;
    Ok(ViewBookmark {
        name: require_str(object, "name")?.to_string(),
        viewport: ViewportId(viewport),
        space: decode_space(object.get("space"))?,
        camera_transform: decode_transform(require(object, "camera_transform")?)?,
    })
}
