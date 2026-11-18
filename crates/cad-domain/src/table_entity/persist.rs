//! The persisted restore constructor for [`super::TableGeometry`]
//! (ADR-0037 D8).
//!
//! Every other write path derives the display: [`super::TableGeometry::new`]
//! validates the grid and builds it, `from_import`/`from_import_with_style`
//! clamp merge spans and build it, and every `with_*` writer rebuilds it.
//!
//! Phase 1 persists the derived display **verbatim**, primitives and their
//! per-primitive appearance in lockstep (ADR-0037 D8). Restoring through
//! `from_import` would re-derive that cache on the first save (breaking the L1
//! fixed point) and `new` would reject a legacy grid whose stored spacings this
//! build's structural rules no longer accept. This constructor takes every
//! field as stored — no validation, no rebuild.
//!
//! The fields stay private: this module is a child of `table_entity`, so it can
//! reach them without widening them to `pub`.

use super::grid::CellMarginTable;
use super::{
    TableCellMargins, TableColumnGeometry, TableDisplayAppearance, TableFrame, TableGeometry,
    TableRowGeometry, TableStyleValues,
};
use crate::SemanticGeometry;

impl TableGeometry {
    /// Rebuild a table exactly as it was persisted.
    ///
    /// `margins` is the sparse override list the payload carries; the flat
    /// `CellMarginTable` it becomes is an in-memory detail, not a second stored
    /// shape. The caller (the ymodel payload reader) has already accepted the
    /// stored object.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_persisted(
        frame: TableFrame,
        columns: Vec<TableColumnGeometry>,
        rows: Vec<TableRowGeometry>,
        style: TableStyleValues,
        margins: Vec<((usize, usize), TableCellMargins)>,
        name: String,
        description: String,
        display: Vec<SemanticGeometry>,
        appearance: Vec<TableDisplayAppearance>,
    ) -> Self {
        let margins: CellMarginTable = margins.into_iter().collect();
        TableGeometry {
            frame,
            columns,
            rows,
            style,
            margins,
            name,
            description,
            display: super::display::TableDisplay::from_parts(display, appearance),
        }
    }

    /// The stored display primitives and their appearance, in lockstep.
    pub(crate) fn display_parts(&self) -> (&[SemanticGeometry], &[TableDisplayAppearance]) {
        self.display.parts()
    }
}
