//! The drawing/editing kind catalogue: kinds, arities, keys, labels and the
//! per-kind metadata the capture state machine reads.
//!
//! Split out of [`super`] to keep the state-machine module inside the code-size
//! limit. Every item here is re-exported by [`super`] under its original name,
//! so `crate::draw_tool::DrawToolKind`, `crate::draw_tool::PointArity` and the
//! crate-root re-exports are unchanged.

mod definition;

/// How many points a capture needs: a fixed complement, or an open-ended
/// minimum.
///
/// The interactive tools were historically fixed-arity (`required_points`),
/// which cannot express a polyline: PLINE accepts as many vertices as the user
/// clicks and is finished with Enter. Making the arity explicit lets the state
/// machine keep the two models apart instead of pretending an open-ended tool
/// has a fixed "0 remaining".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointArity {
    /// Exactly this many points; the `(n + 1)`-th is refused.
    Exact(usize),
    /// At least this many points; further points are accepted (up to the hard
    /// cap [`MAX_OPEN_CAPTURE_POINTS`]) and the capture is confirmed with Enter.
    AtLeast(usize),
}

impl PointArity {
    /// The minimum number of points required to define the operation.
    pub fn minimum(self) -> usize {
        match self {
            Self::Exact(n) | Self::AtLeast(n) => n,
        }
    }

    /// Whether the capture accepts points beyond the minimum (open-ended).
    pub fn is_open_ended(self) -> bool {
        matches!(self, Self::AtLeast(_))
    }
}

/// The drawing/editing operation a tool captures for — see the
/// [`definition`] submodule for the variant catalogue and each kind's documented
/// subset.
pub use definition::DrawToolKind;

impl DrawToolKind {
    /// Every kind, in the order the UI selector presents them. This is the one
    /// authoritative ordering; the UI shell mirrors it instead of keeping a
    /// second hand-written list that can drift.
    pub const ALL: [DrawToolKind; 40] = [
        DrawToolKind::Line,
        DrawToolKind::Circle,
        DrawToolKind::Arc,
        DrawToolKind::Polyline,
        DrawToolKind::Rectang,
        DrawToolKind::Ellipse,
        DrawToolKind::Polygon,
        DrawToolKind::Point,
        DrawToolKind::Spline,
        DrawToolKind::Move,
        DrawToolKind::Trim,
        DrawToolKind::Copy,
        DrawToolKind::Extend,
        DrawToolKind::Rotate,
        DrawToolKind::Scale,
        DrawToolKind::Mirror,
        DrawToolKind::Offset,
        DrawToolKind::Chamfer,
        DrawToolKind::Fillet,
        DrawToolKind::ArrayRect,
        DrawToolKind::ArrayPolar,
        DrawToolKind::Align,
        DrawToolKind::Stretch,
        DrawToolKind::DimLinear,
        DrawToolKind::DimAligned,
        DrawToolKind::DimRadius,
        DrawToolKind::DimDiameter,
        DrawToolKind::DimCenter,
        DrawToolKind::DimAngular,
        DrawToolKind::DimOrdinate,
        DrawToolKind::Hatch,
        // Appended last so every pre-existing index stays stable (the UI mirrors
        // this order rather than keeping a second list).
        DrawToolKind::Text,
        DrawToolKind::TextEdit,
        DrawToolKind::MLeader,
        // P5f's pair is appended too, for the same index-stability reason.
        DrawToolKind::DimBaseline,
        DrawToolKind::DimContinue,
        // MTEXT closes the list for the same reason: every pre-existing index
        // (which the UI mirrors) stays stable.
        DrawToolKind::MText,
        // The clipboard tool cluster is appended for the same index-stability
        // reason: COPYBASE/CUTBASE (one base point over the selection) and
        // PASTECLIP (one insertion point over the clipboard). They are started by
        // their command selectors, not from the draw-tool selector list, but they
        // are real kinds (the `index`/`from_index` contract covers every kind).
        DrawToolKind::CopyBase,
        DrawToolKind::CutBase,
        DrawToolKind::PasteClip,
    ];

    /// Stable, locale-independent machine key (never translated).
    pub fn key(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::Circle => "circle",
            Self::Arc => "arc",
            Self::Polyline => "polyline",
            Self::Rectang => "rectang",
            Self::Ellipse => "ellipse",
            Self::Polygon => "polygon",
            Self::Point => "point",
            Self::Spline => "spline",
            Self::Move => "move",
            Self::Trim => "trim",
            Self::Copy => "copy",
            Self::Extend => "extend",
            Self::Rotate => "rotate",
            Self::Scale => "scale",
            Self::Mirror => "mirror",
            Self::Offset => "offset",
            Self::Chamfer => "chamfer",
            Self::Fillet => "fillet",
            Self::ArrayRect => "arrayrect",
            Self::ArrayPolar => "arraypolar",
            Self::Align => "align",
            Self::Stretch => "stretch",
            Self::DimLinear => "dimlinear",
            Self::DimAligned => "dimaligned",
            Self::DimRadius => "dimradius",
            Self::DimDiameter => "dimdiameter",
            Self::DimCenter => "dimcenter",
            Self::DimAngular => "dimangular",
            Self::DimOrdinate => "dimordinate",
            Self::DimBaseline => "dimbaseline",
            Self::DimContinue => "dimcontinue",
            Self::Hatch => "hatch",
            Self::Text => "text",
            Self::TextEdit => "textedit",
            Self::MLeader => "mleader",
            Self::MText => "mtext",
            Self::CopyBase => "copybase",
            Self::CutBase => "cutbase",
            Self::PasteClip => "pasteclip",
        }
    }

    /// Parse a machine key back to a kind; unknown keys are rejected instead of
    /// silently falling back to a default operation.
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "line" => Some(Self::Line),
            "circle" => Some(Self::Circle),
            "arc" => Some(Self::Arc),
            "polyline" => Some(Self::Polyline),
            "rectang" => Some(Self::Rectang),
            "ellipse" => Some(Self::Ellipse),
            "polygon" => Some(Self::Polygon),
            "point" => Some(Self::Point),
            "spline" => Some(Self::Spline),
            "move" => Some(Self::Move),
            "trim" => Some(Self::Trim),
            "copy" => Some(Self::Copy),
            "extend" => Some(Self::Extend),
            "rotate" => Some(Self::Rotate),
            "scale" => Some(Self::Scale),
            "mirror" => Some(Self::Mirror),
            "offset" => Some(Self::Offset),
            "chamfer" => Some(Self::Chamfer),
            "fillet" => Some(Self::Fillet),
            "arrayrect" => Some(Self::ArrayRect),
            "arraypolar" => Some(Self::ArrayPolar),
            "align" => Some(Self::Align),
            "stretch" => Some(Self::Stretch),
            "dimlinear" => Some(Self::DimLinear),
            "dimaligned" => Some(Self::DimAligned),
            "dimradius" => Some(Self::DimRadius),
            "dimdiameter" => Some(Self::DimDiameter),
            "dimcenter" => Some(Self::DimCenter),
            "dimangular" => Some(Self::DimAngular),
            "dimordinate" => Some(Self::DimOrdinate),
            "dimbaseline" => Some(Self::DimBaseline),
            "dimcontinue" => Some(Self::DimContinue),
            "hatch" => Some(Self::Hatch),
            "text" => Some(Self::Text),
            "textedit" => Some(Self::TextEdit),
            "mleader" => Some(Self::MLeader),
            "mtext" => Some(Self::MText),
            "copybase" => Some(Self::CopyBase),
            "cutbase" => Some(Self::CutBase),
            "pasteclip" => Some(Self::PasteClip),
            _ => None,
        }
    }

    /// Resolve a kind from its user-facing label (as displayed by the shell).
    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|kind| kind.label() == label)
    }

    /// Position in [`Self::ALL`].
    pub fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|kind| *kind == self)
            .expect("every kind is listed in ALL")
    }

    /// Kind at a UI index, or `None` for an out-of-range index.
    pub fn from_index(index: i32) -> Option<Self> {
        usize::try_from(index)
            .ok()
            .and_then(|i| Self::ALL.get(i).copied())
    }

    /// Short user-facing label. The shell substitutes the catalog label for the
    /// same ordering, so this is a default/fallback, not the chrome source.
    pub fn label(self) -> &'static str {
        match self {
            Self::Line => "直线",
            Self::Circle => "圆",
            Self::Arc => "圆弧",
            Self::Polyline => "多段线",
            Self::Rectang => "矩形",
            Self::Ellipse => "椭圆",
            Self::Polygon => "正多边形",
            // POINT/SPLINE deliberately use locale-neutral ASCII fallback labels:
            // the ADR fence forbids new CJK literals on the Rust side, and the
            // shell substitutes the catalog label for chrome anyway, so `label()`
            // stays a non-chrome fallback here (see the CJK baseline test).
            Self::Point => "POINT",
            Self::Spline => "SPLINE",
            Self::Move => "移动",
            Self::Trim => "修剪",
            Self::Copy => "复制",
            Self::Extend => "延伸",
            Self::Rotate => "旋转",
            Self::Scale => "缩放",
            Self::Mirror => "镜像",
            Self::Offset => "偏移",
            // CHAMFER/FILLET use locale-neutral ASCII fallback labels, exactly like
            // POINT/SPLINE: the ADR fence forbids new CJK literals on the Rust side
            // and the shell substitutes the catalog label for chrome anyway.
            Self::Chamfer => "CHAMFER",
            Self::Fillet => "FILLET",
            // ARRAYRECT/ARRAYPOLAR use locale-neutral ASCII fallback labels,
            // exactly like POINT/SPLINE/CHAMFER/FILLET: the ADR fence forbids new
            // CJK literals on the Rust side and the shell substitutes the catalog
            // label for chrome anyway.
            Self::ArrayRect => "ARRAYRECT",
            Self::ArrayPolar => "ARRAYPOLAR",
            // ALIGN uses a locale-neutral ASCII fallback label, exactly like
            // POINT/SPLINE/CHAMFER/FILLET/ARRAY*: the ADR fence forbids new CJK
            // literals on the Rust side and the shell substitutes the catalog label
            // for chrome anyway.
            Self::Align => "ALIGN",
            // STRETCH uses a locale-neutral ASCII fallback label, exactly like
            // POINT/SPLINE/CHAMFER/FILLET/ARRAY*/ALIGN: the ADR fence forbids new
            // CJK literals on the Rust side and the shell substitutes the catalog
            // label for chrome anyway.
            Self::Stretch => "STRETCH",
            // DIMLINEAR/DIMALIGNED use locale-neutral ASCII fallback labels,
            // exactly like POINT/SPLINE/CHAMFER/FILLET/ARRAY*/ALIGN/STRETCH: the
            // ADR fence forbids new CJK literals on the Rust side and the shell
            // substitutes the catalog label for chrome anyway.
            Self::DimLinear => "DIMLINEAR",
            Self::DimAligned => "DIMALIGNED",
            // DIMRADIUS/DIMDIAMETER/DIMCENTER use locale-neutral ASCII fallback
            // labels, exactly like the other post-ADR-0015 kinds: the ADR fence
            // forbids new CJK literals on the Rust side and the shell substitutes
            // the catalog label for chrome anyway.
            Self::DimRadius => "DIMRADIUS",
            Self::DimDiameter => "DIMDIAMETER",
            Self::DimCenter => "DIMCENTER",
            // DIMANGULAR/DIMORDINATE use locale-neutral ASCII fallback labels,
            // exactly like the other post-ADR-0015 kinds: the ADR fence forbids
            // new CJK literals on the Rust side and the shell substitutes the
            // catalog label for chrome anyway.
            Self::DimAngular => "DIMANGULAR",
            Self::DimOrdinate => "DIMORDINATE",
            // DIMBASELINE/DIMCONTINUE use locale-neutral ASCII fallback labels,
            // exactly like the other dimension kinds.
            Self::DimBaseline => "DIMBASELINE",
            Self::DimContinue => "DIMCONTINUE",
            // HATCH uses a locale-neutral ASCII fallback label, exactly like the
            // other post-ADR-0015 kinds: the ADR fence forbids new CJK literals on
            // the Rust side and the shell substitutes the catalog label for chrome
            // anyway.
            Self::Hatch => "HATCH",
            // TEXT uses a locale-neutral ASCII fallback label, exactly like the
            // other post-ADR-0015 kinds: the ADR fence forbids new CJK literals on
            // the Rust side and the shell substitutes the catalog label for chrome
            // anyway.
            Self::Text => "TEXT",
            // TEXTEDIT uses a locale-neutral ASCII fallback label, exactly like
            // the other post-ADR-0015 kinds: the ADR fence forbids new CJK
            // literals on the Rust side and the shell substitutes the catalog
            // label for chrome anyway.
            Self::TextEdit => "TEXTEDIT",
            // MLEADER uses a locale-neutral ASCII fallback label, exactly like the
            // other post-ADR-0015 kinds: the ADR fence forbids new CJK literals on
            // the Rust side and the shell substitutes the catalog label anyway.
            Self::MLeader => "MLEADER",
            // MTEXT shares the same locale-neutral fallback rule (no new CJK on
            // the Rust side; the catalog carries the chrome copy).
            Self::MText => "MTEXT",
            // The clipboard cluster uses locale-neutral ASCII fallback labels for
            // the same reason (no new CJK literals on the Rust side; the shell
            // substitutes the catalog label for chrome).
            Self::CopyBase => "COPYBASE",
            Self::CutBase => "CUTBASE",
            Self::PasteClip => "PASTECLIP",
        }
    }

    /// The capture arity: an exact complement, or an open-ended minimum.
    pub fn arity(self) -> PointArity {
        match self {
            // PLINE and SPLINE are the two open-ended captures: at least two
            // vertices (PLINE) / four fit points (SPLINE), more accepted,
            // finished with Enter.
            Self::Polyline => PointArity::AtLeast(2),
            Self::Spline => PointArity::AtLeast(4),
            // DIMBASELINE/DIMCONTINUE: the base **pick** consumes the first
            // capture step (it is stored as a point too), so the minimum is the
            // pick plus **one measured point** — a base pick alone must never be
            // confirmable. More points are accepted; Enter finishes the capture.
            Self::DimBaseline | Self::DimContinue => PointArity::AtLeast(2),
            // Start, a point on the arc, end; two axis endpoints plus the
            // other-axis distance; base, reference, target; or the two dimension
            // origins plus the dimension-line location.
            Self::Arc
            | Self::Ellipse
            | Self::Scale
            | Self::DimLinear
            | Self::DimAligned
            // DIMANGULAR: two entity picks plus the dimension-arc location.
            | Self::DimAngular => PointArity::Exact(3),
            // ALIGN: the first source point, its destination, then the second
            // source point and its destination (ADR 0016 decision F). STRETCH: the
            // two opposite corners of the crossing window, then the base point and
            // the displacement point (ADR 0016 decision E).
            Self::Align | Self::Stretch => PointArity::Exact(4),
            // One side/distance point, one point placement, the ARRAYPOLAR
            // centre, the single arc/circle pick of DIMCENTER, or TEXT's single
            // insertion point (its text is a typed string, not a point).
            Self::Offset
            | Self::Point
            | Self::ArrayPolar
            | Self::DimCenter
            | Self::Text
            // TEXTEDIT's single captured step is its entity pick.
            | Self::TextEdit
            // COPYBASE/CUTBASE capture their one base point; PASTECLIP captures
            // its one insertion point.
            | Self::CopyBase
            | Self::CutBase
            | Self::PasteClip => PointArity::Exact(1),
            // ARRAYRECT captures no points: it is defined entirely by the session
            // typed parameters plus the selection snapshot, so its complement is
            // the zero-point exact arity (ready as soon as the selection is
            // non-empty). HATCH has the same pre-selection shape: its boundary is
            // the selected entities and the parameters are the typed channel, so
            // it captures no points either.
            Self::ArrayRect | Self::Hatch => PointArity::Exact(0),
            // Two endpoints, centre+radius point, two opposite rectangle
            // corners, a move/copy delta, target+boundary, or a rotate angle /
            // mirror axis.
            Self::Line
            | Self::Circle
            | Self::Rectang
            | Self::Polygon
            | Self::Move
            | Self::Trim
            | Self::Copy
            | Self::Extend
            | Self::Rotate
            | Self::Mirror
            | Self::Chamfer
            | Self::Fillet
            // DIMRADIUS/DIMDIAMETER: the entity pick plus the dimension-line
            // location.
            | Self::DimRadius
            | Self::DimDiameter
            // DIMORDINATE: the feature point and the leader endpoint.
            | Self::DimOrdinate
            // MLEADER: the arrow position and the landing point; its text is the
            // terminal typed step, not a point.
            | Self::MLeader
            // MTEXT: the two opposite corners of the wrap rectangle; its text is
            // the terminal typed step, not a point.
            | Self::MText => PointArity::Exact(2),
        }
    }

    /// Minimum number of points this operation needs to be defined.
    ///
    /// For a fixed-arity (`Exact`) kind this is the exact complement; for an
    /// open-ended (`AtLeast`) kind (PLINE, SPLINE) it is the minimum needed to
    /// confirm, with more points still accepted.
    pub fn required_points(self) -> usize {
        self.arity().minimum()
    }

    /// Whether this operation needs a non-empty selection before it can run.
    ///
    /// MOVE transforms existing entities, COPY duplicates them, and
    /// ROTATE/SCALE/MIRROR transform (or mirror) them; all refuse to run with no
    /// selection rather than acting on nothing. OFFSET needs exactly one selected
    /// source (enforced by the command layer). HATCH fills the selected boundary
    /// entities, so it too needs a non-empty selection before it can run.
    pub fn requires_selection(self) -> bool {
        matches!(
            self,
            Self::Move
                | Self::Copy
                | Self::Rotate
                | Self::Scale
                | Self::Mirror
                | Self::Offset
                | Self::ArrayRect
                | Self::ArrayPolar
                | Self::Align
                | Self::Hatch
                // COPYBASE/CUTBASE capture the **selection** into the clipboard
                // (a selection-less capture would be an empty success), so they
                // need one before they are ready. PASTECLIP's input is the
                // clipboard, not the selection, so it deliberately does not.
                | Self::CopyBase
                | Self::CutBase
        )
    }

    /// How many **leading** capture steps pick an existing entity rather than a
    /// free point.
    ///
    /// TRIM/EXTEND and CHAMFER/FILLET pick an entity at **every** step, so this is
    /// the whole complement. DIMRADIUS/DIMDIAMETER/DIMCENTER pick one entity first
    /// and (for the radial pair) then take a free location point, so this is `1`.
    /// Every other kind captures free points only (`0`).
    ///
    /// The capture state machine reads this to route each step: a click is an
    /// entity pick while `points.len() < picked_step_count()`, and a free point
    /// after it. It replaces the old "all steps are picks" boolean, which could not
    /// express a mixed capture.
    pub fn picked_step_count(self) -> usize {
        match self {
            Self::Trim | Self::Extend | Self::Chamfer | Self::Fillet => self.required_points(),
            // DIMBASELINE/DIMCONTINUE pick the base dimension they extend (ADR
            // 0017 decision C, P5f) before any free point.
            Self::DimRadius
            | Self::DimDiameter
            | Self::DimCenter
            | Self::DimBaseline
            | Self::DimContinue => 1,
            // TEXTEDIT picks the text entity it will edit as its only step.
            Self::TextEdit => 1,
            // DIMANGULAR's two-line form picks both lines; its third step is the
            // free dimension-arc location.
            Self::DimAngular => 2,
            _ => 0,
        }
    }

    /// Whether this operation captures **any** existing entity (an entity pick) as
    /// a step.
    ///
    /// TRIM/EXTEND are the two AutoCAD commands whose arguments are object
    /// selections, not arbitrary coordinates: the click is an entity pick, and
    /// an empty-space click is refused instead of being accepted as a point and
    /// resolved later. CHAMFER/FILLET share that shape (each step picks a straight
    /// line). DIMRADIUS/DIMDIAMETER/DIMCENTER add the radial family, whose **only**
    /// entry point is an arc/circle pick (ADR 0017 decision C): their first step is
    /// an entity pick even though a later step may be a free point.
    ///
    /// These kinds resolve their refs at capture time, so they are only ready once
    /// every entity-pick step has a real entity behind it.
    pub fn needs_picked_entities(self) -> bool {
        self.picked_step_count() > 0
    }

    /// Whether this operation's **last** step is a typed string rather than a
    /// point (ADR 0019 decision A.1).
    ///
    /// TEXT, TEXTEDIT, MLEADER and MTEXT carry the text step today. This is the
    /// single authority the capture state machine, the status projection
    /// (`awaiting_text`) and the UI prompt phase read; a UI must never infer
    /// "needs text" from `key` plus the point count, because `ready` alone cannot
    /// tell "missing points" from "missing text".
    pub fn requires_text(self) -> bool {
        matches!(
            self,
            Self::Text | Self::TextEdit | Self::MLeader | Self::MText
        )
    }
}
