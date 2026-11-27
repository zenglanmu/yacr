//! One frame's distance fog (SketchUp's Fog), split out of `lib.rs` for the
//! file-size gate.

/// One frame's distance fog (SketchUp's Fog): a colour and the model-unit
/// distance band it ramps over from the camera.
///
/// The renderer only carries the value into the shared uniform (`Camera.fog_*`)
/// of the geometry it draws; the shaders blend toward `color` as the fragment's
/// distance from the eye goes from `start` to `end`. A non-finite or reversed
/// band is sanitised to the documented default rather than encoded as NaN.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameFog {
    /// Target colour, linear channels in `[0, 1]`.
    pub color: [f32; 3],
    /// Distance where the fog begins, in model units.
    pub start: f32,
    /// Distance where geometry is fully fogged, in model units (`> start`).
    pub end: f32,
}

impl FrameFog {
    /// The fog uniform words `[r, g, b, 1, start, end, 0, 0]`, sanitised.
    pub(crate) fn words(self) -> [f32; 8] {
        let channel = |c: f32| {
            if c.is_finite() {
                c.clamp(0.0, 1.0)
            } else {
                1.0
            }
        };
        let start = if self.start.is_finite() {
            self.start.max(0.0)
        } else {
            0.0
        };
        let end = if self.end.is_finite() {
            self.end
        } else {
            start + 1.0
        };
        let end = end.max(start + 1e-3);
        [
            channel(self.color[0]),
            channel(self.color[1]),
            channel(self.color[2]),
            1.0,
            start,
            end,
            0.0,
            0.0,
        ]
    }
}
