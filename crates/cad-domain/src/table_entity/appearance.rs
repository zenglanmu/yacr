//! Per-primitive appearance of a table's derived display, and the edit type the
//! commands use to write it.
//!
//! # Why a per-primitive appearance exists at all
//!
//! [`SemanticGeometry`](crate::SemanticGeometry) carries no colour of its own: an
//! entity has exactly one resolved colour (its own `ByLayer`/`ByBlock`/explicit
//! value) and one lineweight, which the representation layer resolves once per
//! entity. A table cell is the one place this build needs **more than one**
//! colour inside a single entity (a cell's text, its background and its border
//! can each differ), so the table's derived display carries a parallel
//! [`TableDisplayAppearance`] per primitive.
//!
//! The channel is deliberately opt-in: [`TableDisplayAppearance::NONE`] (all
//! fields `None`) means "no override", and the primitive is drawn with the
//! entity's resolved colour/lineweight exactly as every table primitive was
//! before a per-cell appearance existed. A table that never sets a colour
//! therefore derives the same geometry **and** the same appearance it always did.
//!
//! # Units and representation
//!
//! Colours are sRGB bytes, the same convention
//! [`Mesh::colors`](crate::Mesh::colors) already uses, so the domain stays
//! independent of the renderer's float convention. A lineweight is stored in
//! **1/100 mm** — the DXF groups 275-279 unit — so the value written back is
//! exactly the value stored (no rounding on save). The
//! [`TableBorder`](super::TableBorder) documents that unit on the field itself.

use super::TableCellBorders;

/// The appearance one derived display primitive carries.
///
/// Both fields are overrides: `None` keeps the primitive on the entity's
/// resolved colour/lineweight.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TableDisplayAppearance {
    /// sRGB colour override (a cell's text colour, fill colour or border
    /// colour).
    pub color: Option<[u8; 3]>,
    /// Border lineweight in 1/100 mm (the DXF groups 275-279 unit).
    pub lineweight_hundredths: Option<u16>,
}

impl TableDisplayAppearance {
    /// No override: the primitive uses the entity's resolved appearance.
    pub const NONE: TableDisplayAppearance = TableDisplayAppearance {
        color: None,
        lineweight_hundredths: None,
    };

    /// A primitive drawn in exactly `color`.
    pub const fn colored(color: [u8; 3]) -> Self {
        TableDisplayAppearance {
            color: Some(color),
            lineweight_hundredths: None,
        }
    }

    /// A border primitive with its edge's colour and lineweight.
    pub const fn border(color: Option<[u8; 3]>, lineweight_hundredths: Option<u16>) -> Self {
        TableDisplayAppearance {
            color,
            lineweight_hundredths,
        }
    }

    /// Whether this is [`Self::NONE`] (no override at all).
    pub fn is_default(&self) -> bool {
        *self == Self::NONE
    }
}

/// What an edit does to one optional field.
///
/// Three states are needed because "leave the stored value alone" and "clear the
/// override" are different writes: `TABLEDIT FORMAT 1 1 ALIGN TL` must not clear
/// a cell colour, while `TABLEDIT FORMAT 1 1 COLOR BYLAYER` must.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableFieldEdit<T> {
    /// Leave the stored value untouched.
    #[default]
    Keep,
    /// Clear the override (back to "inherit" / "no fill").
    Clear,
    /// Store this value.
    Set(T),
}

impl<T: Copy> TableFieldEdit<T> {
    /// Whether the edit leaves the field alone.
    pub fn is_keep(&self) -> bool {
        matches!(self, TableFieldEdit::Keep)
    }

    /// Apply the edit to one stored optional field.
    pub fn apply(&self, target: &mut Option<T>) {
        match self {
            TableFieldEdit::Keep => {}
            TableFieldEdit::Clear => *target = None,
            TableFieldEdit::Set(value) => *target = Some(*value),
        }
    }

    /// The value this edit would store, if any (used for "already set" checks).
    pub fn value(&self) -> Option<T> {
        match self {
            TableFieldEdit::Keep | TableFieldEdit::Clear => None,
            TableFieldEdit::Set(value) => Some(*value),
        }
    }
}

/// One cell's appearance write (`TABLEDIT FORMAT`'s colour/fill/lineweight
/// fields).
///
/// The border fields reach **all four edges** of the cell's resolved border set:
/// the typed grammar has no per-edge selector, and a per-edge colour would need
/// a second address (`<edge>`) this build's table record does not require. The
/// per-edge selector lives on the row-level `TABLESTYLE BORDER <edge> …` form.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TableCellAppearanceEdit {
    /// The cell's text (content) colour, DXF group 64.
    pub text_color: TableFieldEdit<[u8; 3]>,
    /// The cell's background fill colour, DXF groups 63 + 283.
    pub fill: TableFieldEdit<[u8; 3]>,
    /// All four edges' border colour, DXF groups 65-69.
    pub border_color: TableFieldEdit<[u8; 3]>,
    /// All four edges' border lineweight in 1/100 mm, DXF groups 275-279.
    pub border_lineweight: TableFieldEdit<u16>,
    /// All four edges' visibility (`false` hides the edge), DXF groups
    /// 285-289 of a cell record / `GRIDFORMAT` group 93.
    pub border_visible: TableFieldEdit<bool>,
    /// All four edges' double-line spacing in drawing units — the border
    /// **type** this build stores. A spacing above zero *is* the DXF `Double`
    /// border type (`GRIDFORMAT` groups 91 + 40); `Set(0.0)` is a single line.
    pub border_spacing: TableFieldEdit<f64>,
}

impl TableCellAppearanceEdit {
    /// Whether the edit writes nothing (the command then answers
    /// `tabledit.format_empty` instead of opening a transaction).
    pub fn is_empty(&self) -> bool {
        self.text_color.is_keep()
            && self.fill.is_keep()
            && self.border_color.is_keep()
            && self.border_lineweight.is_keep()
            && self.border_visible.is_keep()
            && self.border_spacing.is_keep()
    }

    /// Apply the border fields to every edge of `borders`.
    pub fn apply_borders(&self, borders: &mut TableCellBorders) {
        for edge in [
            &mut borders.top,
            &mut borders.right,
            &mut borders.bottom,
            &mut borders.left,
        ] {
            self.border_color.apply(&mut edge.color);
            self.border_lineweight
                .apply(&mut edge.lineweight_hundredths);
            // Visibility and spacing are plain fields, not overrides, so their
            // neutral state is "visible" / "single line" rather than a `None`.
            match self.border_visible {
                TableFieldEdit::Keep => {}
                TableFieldEdit::Clear => edge.visible = true,
                TableFieldEdit::Set(visible) => edge.visible = visible,
            }
            match self.border_spacing {
                TableFieldEdit::Keep => {}
                TableFieldEdit::Clear => edge.double_spacing = 0.0,
                TableFieldEdit::Set(spacing) => edge.double_spacing = spacing,
            }
        }
    }
}
