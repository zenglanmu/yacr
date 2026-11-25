//! The typed tool-parameter block of [`super::CadTool`] (ADR 0016 decision A).
//!
//! Split out of `status.rs` to keep that module inside the code-size limit; the
//! type is re-exported there under its original name, so every existing path and
//! serde shape is unchanged.

use serde::Serialize;

/// The typed tool-parameter block of [`super::CadTool`] (ADR 0015 decision C, extended
/// by ADR 0016 decisions A/B/C/D/F).
///
/// A frozen frontend shape. Each field is `null` whenever the active tool does not
/// own that parameter (and the whole `tool` is `null` when no tool is active), so a
/// UI control's enablement can be derived from `tool.key` and never from a stale
/// local value. The legacy `sides` field keeps its exact name and meaning; the
/// FILLET/CHAMFER/ARRAY/ALIGN parameters are added alongside it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadToolParams {
    /// The POLYGON side count, or `null` for every other tool.
    pub sides: Option<u32>,
    /// The FILLET radius, or `null` for every other tool.
    pub fillet_radius: Option<f64>,
    /// The CHAMFER first distance, or `null` for every other tool.
    pub chamfer_distance_a: Option<f64>,
    /// The CHAMFER second distance, or `null` for every other tool.
    pub chamfer_distance_b: Option<f64>,
    /// The ARRAYRECT row count, or `null` for every other tool.
    pub array_rows: Option<f64>,
    /// The ARRAYRECT column count, or `null` for every other tool.
    pub array_columns: Option<f64>,
    /// The ARRAYRECT row spacing, or `null` for every other tool.
    pub array_row_spacing: Option<f64>,
    /// The ARRAYRECT column spacing, or `null` for every other tool.
    pub array_column_spacing: Option<f64>,
    /// The ARRAYPOLAR item count, or `null` for every other tool.
    pub array_items: Option<f64>,
    /// The ARRAYPOLAR fill angle (degrees), or `null` for every other tool.
    pub array_fill_angle: Option<f64>,
    /// The ARRAYPOLAR rotate-items flag, or `null` for every other tool.
    pub array_rotate_items: Option<bool>,
    /// The ALIGN scale flag, or `null` for every other tool.
    pub align_scale: Option<bool>,
    /// The ALIGN pair count (`1`/`2`/`3`), or `null` for every other tool.
    pub align_pairs: Option<f64>,
    /// The BREAK `F` first-point override flag, or `null` for every other tool.
    pub break_first_point: Option<bool>,
    /// The DIVIDE block-option flag, or `null` for every other tool.
    pub divide_use_block: Option<bool>,
    /// The DIVIDE block-align flag (AutoCAD 2026's "Align block with object?
    /// [Yes/No]"), or `null` for every other tool.
    pub divide_align_block: Option<bool>,
    /// The HATCH pattern type (`0` solid, `1` user-defined, `2` predefined), or
    /// `null` for every other tool (ADR 0018 decision D, P6b).
    pub hatch_pattern_type: Option<f64>,
    /// The HATCH predefined pattern index (**1-based** into the implemented
    /// `ANSI31`/`ANSI32`/`ANSI37` table), or `null` for every other tool.
    pub hatch_pattern: Option<f64>,
    /// The HATCH pattern spacing (also the predefined pattern scale / `HPSCALE`),
    /// or `null` for every other tool.
    pub hatch_spacing: Option<f64>,
    /// The HATCH pattern angle in degrees, or `null` for every other tool.
    pub hatch_angle: Option<f64>,
    /// The HATCH `HPDOUBLE` flag, or `null` for every other tool.
    pub hatch_double: Option<bool>,
    /// The HATCH style (`0` Normal, `1` Outer, `2` Ignore), or `null` for every
    /// other tool.
    pub hatch_style: Option<f64>,
    /// The HATCH gradient kind (`0` Linear, `1` Spherical), or `null` for every
    /// other tool (a gradient fill is pattern type `3`).
    pub hatch_gradient_kind: Option<f64>,
    /// The HATCH gradient angle in **degrees** (the domain stores radians), or
    /// `null` for every other tool.
    pub hatch_gradient_angle: Option<f64>,
    /// The HATCH gradient start-stop colour as an integer ACI index, or `null`
    /// for every other tool.
    pub hatch_gradient_color_a: Option<f64>,
    /// The HATCH gradient end-stop colour as an integer ACI index, or `null` for
    /// every other tool.
    pub hatch_gradient_color_b: Option<f64>,
    /// The TEXT glyph height, or `null` for every other tool (ADR 0019 F.1).
    pub text_height: Option<f64>,
    /// The TEXT/MTEXT rotation in **degrees** (the domain stores radians), or
    /// `null` for every other tool.
    pub text_rotation: Option<f64>,
    /// The MTEXT attachment code — AutoCAD's closed set `1..=9` (`1` top-left,
    /// `5` centre-middle, `9` bottom-right) — or `null` for every other tool.
    pub mtext_attachment: Option<f64>,
    /// The DIVIDE segment count (an integer in `2..=1024`), projected only while a
    /// DIVIDE capture is active (2026-11-01).
    pub divide_segments: Option<f64>,
    /// The LENGTHEN mode (`0` Delta, `1` Percent, `2` Total), projected only while
    /// a LENGTHEN capture is active.
    pub lengthen_mode: Option<f64>,
    /// The LENGTHEN amount (finite, non-zero on the channel), projected only while
    /// a LENGTHEN capture is active.
    pub lengthen_value: Option<f64>,
    /// The SCALETEXT glyph-height factor (finite and strictly positive on the
    /// channel; default `1.0`), projected only while a SCALETEXT capture is
    /// active.
    pub scale_text_factor: Option<f64>,
    /// The DIMROTATED dimension-line angle in **degrees** (the domain stores
    /// radians), projected only while a DIMROTATED capture is active.
    pub dim_rotation: Option<f64>,
    /// The PLINE **uniform** width in world units, centred on the vertices
    /// (`0` is the hairline draw), projected only while a PLINE capture is
    /// active; `null` for every other tool. Per-vertex start/end taper is not
    /// modeled, so this is one value per polyline.
    pub polyline_width: Option<f64>,
    /// The STRETCH crossing-window mode (`0` rectangle, `1` polygon, `2` lasso),
    /// projected only while a STRETCH capture is active (the polygon/lasso ring
    /// wiring); `null` for every other tool. The mode selects the capture's
    /// **first phase** only — all three hand the same crossing window to the same
    /// planner.
    pub stretch_window: Option<f64>,
    /// LINE's typed length in world units, projected only while a LINE capture is
    /// active; `0` is the "no typed length" sentinel (`null` for every other
    /// tool).
    pub line_length: Option<f64>,
    /// LINE's typed direction in degrees, projected only while a LINE capture is
    /// active; `0` means "keep the cursor direction" (`null` for every other
    /// tool).
    pub line_angle: Option<f64>,
    /// RECTANG's typed width in world units, projected only while a RECTANG
    /// capture is active; `0` keeps the cursor's x extent (`null` otherwise).
    pub rect_width: Option<f64>,
    /// RECTANG's typed height in world units, projected only while a RECTANG
    /// capture is active; `0` keeps the cursor's y extent (`null` otherwise).
    pub rect_height: Option<f64>,
    /// CIRCLE's typed radius in world units, projected only while a CIRCLE
    /// capture is active; `0` keeps the cursor-defined radius (`null` otherwise).
    pub circle_radius: Option<f64>,
    /// ROTATE's typed absolute angle in degrees, projected only while a ROTATE
    /// capture is active; `0` keeps the cursor-defined angle (`null` otherwise).
    pub rotate_angle: Option<f64>,
    /// MOVE's typed distance in world units, projected only while a MOVE capture
    /// is active; `0` keeps the cursor-defined displacement (`null` otherwise).
    pub move_distance: Option<f64>,
    /// SCALE's typed factor, projected only while a SCALE capture is active; `0`
    /// keeps the cursor-defined factor (`null` otherwise).
    pub scale_factor: Option<f64>,
    /// OFFSET's typed distance in world units, projected only while an OFFSET
    /// capture is active; `0` keeps the session's stored mode (`null` otherwise).
    pub offset_distance: Option<f64>,
    /// PRESSPULL's signed distance in world units, projected only while a
    /// PRESSPULL capture is active; `0` is unset (`null` otherwise).
    pub press_pull_distance: Option<f64>,
    /// EXTRUDE's signed distance in world units, projected only while an EXTRUDE
    /// capture is active; `0` is unset (`null` otherwise).
    pub extrude_distance: Option<f64>,
}
