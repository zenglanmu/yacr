use std::fmt;

/// Display-only clip boundary of a block reference (`INSERT`, `XCLIP`).
///
/// The boundary is stored in the **block definition's own local coordinate
/// space** (the space the block's children are authored in), not in world space.
/// That is what makes the clip follow the reference through move/rotate/scale:
/// the representation layer maps the corners through the composed INSERT
/// transform at expansion time and clips the expanded geometry there.
///
/// `vertices` is either two opposite corners of a rectangle (the form the
/// representation layer resolves with the dedicated half-plane clippers when it
/// stays axis-aligned in world XY) or three or more points of a block-local
/// polygon (rendered as that ring, concave and rotated quads included).
///
/// `inside` mirrors DXF clip_mode: `true` keeps the region inside the boundary,
/// `false` keeps the region outside it. `visible` is the `XCLIP ON`/`OFF` switch:
/// `false` stores the boundary but applies no clip.
///
/// `front_clip`/`back_clip` are the DXF `SPATIAL_FILTER` **clip distances**
/// (groups `40`/`41`, enabled by flags `72`/`73`). The source expresses them
/// along the clip boundary's plane normal, which this build fixes at the block's
/// `+Z`, so the pair is the **kept block-local z band** `[front, back]`:
/// `None` means that side is unbounded (`[-∞, back]` or `[front, +∞]`), and both
/// `None` means no depth clip at all — the historical 2D boundary, byte for byte.
/// A value is `Some` exactly when its DXF flag is set, so a one-sided band is
/// carried by that side's flag alone and a distance can never be stored without
/// its flag. Build one through [`BlockClip::with_depth`] (or
/// [`validate_clip_depth`]); the fields are public only because the database
/// stores the struct verbatim.
///
/// This is a display attribute, not geometry: it lives beside the entity in the
/// database's render attributes, never in [`SemanticGeometry`].
///
/// [`SemanticGeometry`]: crate::SemanticGeometry
#[derive(Debug, Clone, PartialEq)]
pub struct BlockClip {
    pub vertices: Vec<[f64; 2]>,
    pub inside: bool,
    pub visible: bool,
    /// Front clip distance along the block's `+Z` (DXF group `40`, flag `72`).
    pub front_clip: Option<f64>,
    /// Back clip distance along the block's `+Z` (DXF group `41`, flag `73`).
    pub back_clip: Option<f64>,
}

/// Why a front/back depth pair cannot be stored as a `BlockClip` depth band.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClipDepthError {
    /// A distance is NaN or infinite, so it names no plane.
    NonFinite { front: bool, value: f64 },
    /// The front plane is behind the back plane, so `[front, back]` is empty.
    Reversed { front: f64, back: f64 },
}

impl fmt::Display for ClipDepthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite { front, value } => write!(
                f,
                "the {} clip distance {value} is not a finite number",
                if *front { "front" } else { "back" }
            ),
            Self::Reversed { front, back } => write!(
                f,
                "the front clip distance {front} is behind the back clip distance {back}, so the \
                 kept depth band is empty"
            ),
        }
    }
}

impl std::error::Error for ClipDepthError {}

/// Validate one front/back depth pair: each value that is present must be
/// finite, and a pair must satisfy `front <= back` (equality is the degenerate
/// zero-width band, which is still a well-defined "keep only this plane").
///
/// `None` is an absent plane, not zero: a one-sided band such as
/// `[0, +∞)` is valid and is exactly what a source with only flag `72` set
/// carries.
pub fn validate_clip_depth(front: Option<f64>, back: Option<f64>) -> Result<(), ClipDepthError> {
    for (is_front, value) in [(true, front), (false, back)] {
        if let Some(value) = value {
            if !value.is_finite() {
                return Err(ClipDepthError::NonFinite {
                    front: is_front,
                    value,
                });
            }
        }
    }
    if let (Some(front), Some(back)) = (front, back) {
        if front > back {
            return Err(ClipDepthError::Reversed { front, back });
        }
    }
    Ok(())
}

impl BlockClip {
    /// A boundary with no depth clip (the pre-clip-depth model, unchanged).
    pub fn new(vertices: Vec<[f64; 2]>, inside: bool, visible: bool) -> Self {
        Self {
            vertices,
            inside,
            visible,
            front_clip: None,
            back_clip: None,
        }
    }

    /// The same boundary with a validated front/back depth band.
    ///
    /// `None` on either side keeps that side unbounded; both `None` clears the
    /// depth clip. The boundary is left exactly as it is.
    pub fn with_depth(
        mut self,
        front: Option<f64>,
        back: Option<f64>,
    ) -> Result<Self, ClipDepthError> {
        validate_clip_depth(front, back)?;
        self.front_clip = front;
        self.back_clip = back;
        Ok(self)
    }

    /// The kept **block-local z band**, or `None` when no depth plane is set.
    ///
    /// An absent side becomes `-∞` (no front plane) or `+∞` (no back plane), so
    /// a one-sided clip is still one band the display layer can test.
    pub fn depth_bounds(&self) -> Option<(f64, f64)> {
        match (self.front_clip, self.back_clip) {
            (None, None) => None,
            (front, back) => Some((
                front.unwrap_or(f64::NEG_INFINITY),
                back.unwrap_or(f64::INFINITY),
            )),
        }
    }

    /// Whether the boundary is a two-corner rectangle.
    pub fn is_rect(&self) -> bool {
        self.vertices.len() == 2
    }

    /// Whether the clip should cut geometry at all (`XCLIP ON`, inside mode).
    pub fn is_active(&self) -> bool {
        self.visible && self.inside
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> BlockClip {
        BlockClip::new(vec![[0.0, 0.0], [10.0, 5.0]], true, true)
    }

    #[test]
    fn a_new_clip_carries_no_depth_band() {
        let clip = rect();
        assert_eq!(
            (clip.front_clip, clip.back_clip),
            (None, None),
            "the default is the 2D boundary every existing clip has"
        );
        assert_eq!(clip.depth_bounds(), None, "no depth clip is not [0, 0]");
        assert!(clip.is_active());
        assert!(clip.is_rect());
    }

    #[test]
    fn with_depth_stores_a_validated_band() {
        let clip = rect().with_depth(Some(-2.0), Some(3.0)).unwrap();
        assert_eq!(clip.front_clip, Some(-2.0));
        assert_eq!(clip.back_clip, Some(3.0));
        assert_eq!(clip.depth_bounds(), Some((-2.0, 3.0)));
        assert_eq!(clip.vertices, rect().vertices, "the boundary is untouched");

        // A one-sided band stays one band: the absent side is unbounded.
        let front_only = rect().with_depth(Some(1.0), None).unwrap();
        assert_eq!(front_only.depth_bounds(), Some((1.0, f64::INFINITY)));
        let back_only = rect().with_depth(None, Some(1.0)).unwrap();
        assert_eq!(back_only.depth_bounds(), Some((f64::NEG_INFINITY, 1.0)));

        // A zero-width band is well defined (both planes coincide).
        assert!(rect().with_depth(Some(2.0), Some(2.0)).is_ok());

        // Clearing both sides restores the default.
        assert_eq!(
            rect()
                .with_depth(Some(1.0), Some(2.0))
                .unwrap()
                .with_depth(None, None)
                .unwrap(),
            rect()
        );
    }

    #[test]
    fn with_depth_refuses_non_finite_and_backwards_ranges() {
        for (front, back) in [
            (Some(f64::NAN), None),
            (None, Some(f64::INFINITY)),
            (Some(f64::NEG_INFINITY), Some(1.0)),
        ] {
            assert!(
                matches!(
                    rect().with_depth(front, back),
                    Err(ClipDepthError::NonFinite { .. })
                ),
                "{front:?}/{back:?} names no plane"
            );
        }
        let error = rect().with_depth(Some(4.0), Some(1.0)).unwrap_err();
        assert_eq!(
            error,
            ClipDepthError::Reversed {
                front: 4.0,
                back: 1.0
            }
        );
        assert!(error.to_string().contains("front clip distance 4"));
        // The free validator is the rule the constructor applies; a detached
        // boundary keeps whatever it already stored.
        assert!(validate_clip_depth(Some(4.0), Some(1.0)).is_err());
        assert!(rect().front_clip.is_none());
    }
}
