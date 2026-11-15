//! The semantic geometry enum and the SPLINE closedness rule.
//!
//! Split out of `lib.rs` (the code-size limit); every item is re-exported at
//! the crate root, so `cad_domain::SemanticGeometry` and
//! `cad_domain::is_closed_spline` are unchanged.

use super::*;

/// Whether a SPLINE definition is a closed curve.
///
/// The stored [`SemanticGeometry::Spline::closed`] flag (DXF group 70 bit 1) is
/// authoritative when set. When it is **not** set, a definition whose first and
/// last control points coincide is still reported closed: some sources write a
/// closed spline as a clamped curve with a repeated end control point instead of
/// setting the bit, and both CAD's own `JOIN CLOSE` and LENGTHEN must treat such
/// a frame as having no free end. The fallback is a read-time inference only — it
/// never rewrites the stored geometry or the flag.
pub fn is_closed_spline(closed: bool, control_points: &[Point3], tolerance: f64) -> bool {
    if closed {
        return true;
    }
    let (Some(first), Some(last)) = (control_points.first(), control_points.last()) else {
        return false;
    };
    if control_points.len() < 2 {
        return false;
    }
    let tolerance = tolerance.abs();
    (first.x - last.x).abs() <= tolerance
        && (first.y - last.y).abs() <= tolerance
        && (first.z - last.z).abs() <= tolerance
}

/// How an MTEXT line-spacing factor is applied to the baselines (DXF group 73).
///
/// AutoCAD's two documented styles are the whole domain: `1` = "At least" and
/// `2` = "Exactly". The DXF reader normalises any other group-73 value to
/// [`AtLeast`](Self::AtLeast) before it reaches the domain, so nothing about the
/// style is left unrepresentable (see `docs/compatibility.md`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TextLineSpacingStyle {
    /// At least the stated spacing (DXF group 73 = 1, the DXF default): a line
    /// whose runs are taller than the base height pushes the next baseline down
    /// by that taller height times the factor.
    #[default]
    AtLeast,
    /// Exactly the stated spacing (DXF group 73 = 2): every baseline advances by
    /// the **base** text height times the factor, so a taller run on one line
    /// does not change where the next line starts.
    Exactly,
}

/// The multi-line vertical advance of a TEXT/MTEXT (DXF MTEXT groups 44 + 73).
///
/// [`SemanticGeometry::Text::line_spacing`] carries this. Both halves are
/// dimensionless, so a similarity transform copies them unchanged, and the
/// neutral value ([`TextLineSpacing::default`]: factor
/// [`DEFAULT_TEXT_LINE_SPACING_FACTOR`] = `1.0`, style
/// [`TextLineSpacingStyle::AtLeast`]) reproduces the pre-line-spacing layout
/// exactly.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextLineSpacing {
    /// DXF group 44: the multiple of a single line height that separates
    /// baselines. Must be finite and strictly positive (enforced by `cad-db`);
    /// [`text_line_spacing_factor_from_dxf`] is the one import sanitization rule.
    /// AutoCAD's edit box restricts the value to
    /// [`MIN_TEXT_LINE_SPACING_FACTOR`]..=[`MAX_TEXT_LINE_SPACING_FACTOR`]; a
    /// stored value outside that range still renders (the storage contract is
    /// only "positive and finite").
    pub factor: f64,
    /// DXF group 73: which of the two styles applies the factor.
    pub style: TextLineSpacingStyle,
}

impl Default for TextLineSpacing {
    fn default() -> Self {
        Self {
            factor: DEFAULT_TEXT_LINE_SPACING_FACTOR,
            style: TextLineSpacingStyle::AtLeast,
        }
    }
}

impl TextLineSpacing {
    /// The factor guarded for layout: a non-finite or non-positive stored value
    /// is treated as the neutral one instead of producing NaN geometry. (The
    /// database rejects such values; this guard covers programmatic producers.)
    pub fn effective_factor(&self) -> f64 {
        if self.factor.is_finite() && self.factor > 0.0 {
            self.factor
        } else {
            DEFAULT_TEXT_LINE_SPACING_FACTOR
        }
    }

    /// Whether this is the neutral spacing, i.e. the value every text entity
    /// had before line spacing was modelled.
    pub fn is_default(&self) -> bool {
        self.style == TextLineSpacingStyle::AtLeast
            && (self.factor - DEFAULT_TEXT_LINE_SPACING_FACTOR).abs() <= f64::EPSILON
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SemanticGeometry {
    Line {
        start: Point3,
        end: Point3,
    },
    Polyline {
        points: Vec<Point3>,
        bulges: Vec<f64>,
        closed: bool,
        /// Uniform polyline width in world units, centred on the vertices
        /// (DXF group 43). `0` is the hairline draw; `> 0` is a wide stroke
        /// offset by +/-`width / 2`. Bounds/picking/measurement stay
        /// centreline-based. Per-vertex start/end taper (groups 40/41) is not
        /// modeled; an import reports it as partial, never as an invented taper.
        /// Derived-new-polyline commands (OFFSET/BREAK/JOIN/STRETCH/FILLET/
        /// LENGTHEN) are not yet wired to propagate the width in v1.
        width: f64,
    },
    Circle {
        center: Point3,
        normal: Point3,
        radius: f64,
    },
    Arc {
        center: Point3,
        normal: Point3,
        radius: f64,
        start: f64,
        sweep: f64,
    },
    Ellipse {
        center: Point3,
        /// Plane normal (extrusion). The ellipse lies in the plane through
        /// `center` with this normal; the minor axis is
        /// `cross(normal, major_axis)`, so an ellipse on an arbitrary OCS plane
        /// keeps its true orientation instead of being folded into world XY
        /// (audit B23).
        normal: Point3,
        major_axis: Point3,
        ratio: f64,
        start: f64,
        sweep: f64,
    },
    Spline {
        degree: u32,
        knots: Vec<f64>,
        control_points: Vec<Point3>,
        weights: Vec<f64>,
        /// Whether the curve is closed (DXF SPLINE group 70 bit 1).
        ///
        /// The flag is **metadata about how the curve joins up**, not a
        /// duplicate of the first control point: a closed spline's stored
        /// control polygon stays exactly as the source wrote it (no first/last
        /// point is appended or repeated). Consumers that need a closed loop
        /// (tessellation, JOIN CLOSE) read this flag; a legacy definition with
        /// coincident first/last control points and no flag bit can still be
        /// recognised by that coincidence (see
        /// [`is_closed_spline`]), but the flag is authoritative when set.
        closed: bool,
    },
    Point(Point3),
    Mesh(Mesh),
    Insert {
        block: BlockId,
        transform: Transform3,
    },
    Text {
        text: String,
        position: Point3,
        style: StyleId,
        height: f64,
        rotation: f64,
        /// Resource key of the font this text uses (for example `arial.ttf`),
        /// when the source style declares one. Display providers resolve it
        /// through the font catalog; `None` means the font is unknown.
        font: Option<String>,
        /// Horizontal placement of the run relative to `position`.
        h_align: TextAlignH,
        /// Vertical placement of the run relative to `position`.
        v_align: TextAlignV,
        /// Optional wrap boundary in world units (the DXF width boundary, the
        /// MLEADER group 43 equivalent).
        ///
        /// Text layout word-wraps (and character-wraps scripts without spaces)
        /// so that no line is wider than this boundary. `None` means "no wrap":
        /// paragraphs are laid out exactly as written. Build it with
        /// [`wrap_width_from_dxf`] so the sanitization rule stays in one place.
        wrap_width: Option<f64>,
        /// Horizontal width scale (DXF TEXT group 41): every glyph advance and
        /// glyph x coordinate is multiplied by this factor, so the rendered
        /// width stretches while the height and justification are unchanged.
        ///
        /// Must be finite and strictly positive. [`DEFAULT_TEXT_WIDTH_FACTOR`]
        /// (`1.0`) means "not stretched"; [`text_width_factor_from_dxf`] is the
        /// one sanitization rule for imported values.
        width_factor: f64,
        /// Baseline-to-baseline spacing for the paragraphs of this text (DXF
        /// MTEXT groups 44 + 73). [`TextLineSpacing::default`] — factor `1.0`,
        /// style "at least" — is the value every source without line-spacing
        /// data carries and renders exactly as before the field existed.
        line_spacing: TextLineSpacing,
    },
    /// A SHAPE entity: one glyph of a compiled SHX shape font, referenced by
    /// shape code (or, when `code` is 0, by `shape_name`). `font` is the style's
    /// font key; the representation layer resolves and outlines it.
    Shape {
        shape_name: String,
        code: u32,
        position: Point3,
        size: f64,
        rotation: f64,
        font: Option<String>,
    },
    Opaque {
        type_key: String,
        version: u32,
        payload: Vec<u8>,
    },
    /// Sub-geometries belonging to a single source entity, rendered in order.
    ///
    /// Used where one imported entity yields several primitives. Consumers
    /// recurse. (HATCH is a first-class variant, not a compound.)
    Compound(Vec<SemanticGeometry>),
    /// A DIMENSION: subtype, definition points, style snapshot and derived
    /// display geometry (ADR 0017). Consumers recurse into
    /// [`DimensionGeometry::display`], exactly like `Compound`.
    ///
    /// Boxed because the payload is the largest variant by far; boxing keeps
    /// `SemanticGeometry` small (the repo runs clippy with `-D warnings` and no
    /// `large_enum_variant` exemption).
    Dimension(Box<DimensionGeometry>),
    /// A HATCH (ADR 0018): boundary loops in hatch-plane 2D plus the pattern /
    /// gradient definition, with a derived `display` (every boundary loop plus
    /// the style-filtered fill mesh / pattern polylines / gradient mesh).
    ///
    /// Boxed for the same reason as `Dimension`.
    Hatch(Box<HatchGeometry>),
    /// A MULTILEADER (ADR 0019 decision C): resolved style snapshot, annotation
    /// text, leader roots/lines, the opaque `reserved` bag, plane frame and a
    /// derived `display` (leader paths, arrowheads, doglegs and text).
    ///
    /// Boxed for the same reason as `Dimension`.
    MultiLeader(Box<MultiLeaderGeometry>),
    /// A TABLE (`ACAD_TABLE`): the plane frame, the column/row definition with
    /// per-cell content and resolved borders, plus a derived `display` (grid
    /// lines and one text run per anchoring cell).
    ///
    /// Boxed for the same reason as `Dimension`.
    Table(Box<TableGeometry>),
    /// A raster image (DXF `IMAGE` / `AcDbRasterImage`).
    ///
    /// The image is placed by `origin` and two edge vectors: `u` and `v` span
    /// the image's local axes, and their lengths are one pixel's world size, so
    /// the full extent is `pixels.0 * |u|` by `pixels.1 * |v|`. `pixels` is
    /// `(width, height)` in pixels. `file` is a *logical* resource key resolved
    /// by the host, never a platform path (the same role as `Text::font`).
    /// `visible` mirrors the SHOW_IMAGE flag; `adjust` is the display-only
    /// brightness/contrast/fade/monochrome transfer (see [`ImageAdjust`]).
    Image {
        origin: Point3,
        u: Point3,
        v: Point3,
        pixels: [f64; 2],
        file: Option<String>,
        clip: Option<ImageClip>,
        visible: bool,
        adjust: ImageAdjust,
    },
    /// An erasing mask (DXF `WIPEOUT`).
    ///
    /// `boundary` is a closed world-space polygon; `inverted` corresponds to
    /// clip_mode Inside (mask the outside). The renderer floods the masked region
    /// with the viewport background colour in a final pass, so it carries no
    /// colour of its own and the representation layer maps it to a colourless
    /// mask primitive rather than a coloured mesh.
    ///
    /// Both clip modes are rendered (`docs/adr/0022`): the inverted region is the
    /// complement of `boundary`, which the renderer fills by adding a
    /// viewport-covering ring to the same even-odd fill (no stencil).
    Mask {
        boundary: Vec<Point3>,
        inverted: bool,
    },
}
