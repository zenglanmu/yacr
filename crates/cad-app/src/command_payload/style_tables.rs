//! Typed input of the named **style-table** commands (`STYLE`, `-STYLE`,
//! `MLEADERSTYLE`) after import.
//!
//! The importer owns the initial rows; these commands are the post-import
//! writer's input. One payload describes exactly one operation on **one** style
//! table, and the command layer applies it as one transaction / one undo step.
//! The database owns name validation, duplicate detection and delete safety
//! (`cad-db`'s `drawing::style_edit`), so a payload never repairs a name.
//!
//! `DIMSTYLE` keeps its own payload
//! ([`crate::DimensionBatchPayload::Style`] / [`crate::DimStyleCommand`]) because
//! its `LIST`/`CURRENT` reports predate this module; the write variants share
//! the same database path.

/// Which named style table an operation addresses.
///
/// The three tables are separate symbol tables in the drawing, so the same name
/// may exist in more than one of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleTableKind {
    /// `DB.STYLE` (the text-style table).
    TextStyle,
    /// `DB.DIMSTYLE` (the dimension-style table).
    DimStyle,
    /// `DB.MLEADERSTYLE` (the multileader-style table).
    MLeaderStyle,
}

impl StyleTableKind {
    /// The stable refusal prefix for this table (`style`, `dimstyle`,
    /// `mleaderstyle`).
    pub fn key(self) -> &'static str {
        match self {
            Self::TextStyle => "style",
            Self::DimStyle => "dimstyle",
            Self::MLeaderStyle => "mleaderstyle",
        }
    }

    /// The user-visible table name (`STYLE`, `DIMSTYLE`, `MLEADERSTYLE`).
    pub fn label(self) -> &'static str {
        match self {
            Self::TextStyle => "STYLE",
            Self::DimStyle => "DIMSTYLE",
            Self::MLeaderStyle => "MLEADERSTYLE",
        }
    }
}

/// One typed style-table operation.
///
/// Every variant is applied by the same database batch, so create/rename/delete/
/// modify are all one revision and one undo step. Values are kept verbatim here;
/// the application parses each table's field grammar and refuses an unknown
/// field or an invalid value by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StyleTableOp {
    /// Report every stored row (read-only; no payload write).
    List,
    /// Create a row named `name`; `from` copies the fields of an existing row
    /// (font keys for a text style, every modelled variable for a dim/mleader
    /// style) when present.
    Create { name: String, from: Option<String> },
    /// Rename `old` to `new`.
    Rename { old: String, new: String },
    /// Delete the row named `name`.
    Delete { name: String },
    /// Make `name` the session's current row (`DIMSTYLE`/`MLEADERSTYLE` only;
    /// a text style has no current-style variable in this build).
    Set { name: String },
    /// Write one or more `<field> <value>` pairs of the row named `name`.
    Modify {
        name: String,
        assignments: Vec<(String, String)>,
    },
}

/// The payload of one style-table command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleTableCommand {
    pub kind: StyleTableKind,
    pub op: StyleTableOp,
}
