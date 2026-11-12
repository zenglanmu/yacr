/// Display-only clip boundary of a block reference (`INSERT`, `XCLIP`).
///
/// The boundary is stored in the **block definition's own local coordinate
/// space** (the space the block's children are authored in), not in world space.
/// That is what makes the clip follow the reference through move/rotate/scale:
/// the representation layer maps the corners through the composed INSERT
/// transform at expansion time and clips the expanded geometry there.
///
/// `vertices` is either two opposite corners of a rectangle (the only form the
/// representation layer can render exactly when the transformed rectangle stays
/// axis-aligned in world XY) or three or more points of a block-local polygon.
/// A polygon is stored and reported but **not renderable** in this build (the
/// same declared boundary `VPCLIP` has), so it never claims to be drawn.
///
/// `inside` mirrors DXF clip_mode: `true` keeps the region inside the boundary,
/// `false` keeps the region outside it. `visible` is the `XCLIP ON`/`OFF` switch:
/// `false` stores the boundary but applies no clip.
///
/// This is a display attribute, not geometry: it lives beside the entity in the
/// database's render attributes, never in [`SemanticGeometry`].
#[derive(Debug, Clone, PartialEq)]
pub struct BlockClip {
    pub vertices: Vec<[f64; 2]>,
    pub inside: bool,
    pub visible: bool,
}

impl BlockClip {
    /// Whether this boundary is a two-corner rectangle.
    pub fn is_rect(&self) -> bool {
        self.vertices.len() == 2
    }

    /// Whether the clip should cut geometry at all (`XCLIP ON`, inside mode).
    pub fn is_active(&self) -> bool {
        self.visible && self.inside
    }
}
