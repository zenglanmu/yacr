//! Block-attribute command payloads (ADR-0055 M3).
//!
//! These are the **typed inputs** of the attribute command family. They carry
//! data only: [`crate::app_drawing::attribute_commands`] validates every field
//! against the authoritative drawing and then writes or refuses. The
//! definitions/values themselves live in `cad-domain`; this module never
//! duplicates their shape.
//!
//! The grammar that fills these fields is `cad-ui-core`'s typed-line parser
//! (`session::commands::attribute`). A host may also construct a payload
//! directly (tests and IPC do), and the command layer re-validates it either
//! way.

use cad_domain::{AttributeFlags, Point3};

/// What one `ATTDEF`/`ATTREDEF` invocation defines.
///
/// `order: None` appends after the block's current highest order; `Some` stores
/// the given key verbatim (equal keys are legal — the stable member order then
/// breaks the tie, exactly like [`cad_db::DrawingDatabase::default_attribute_values`]).
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeDefinitionCommand {
    /// The target block **name**; resolved by the command layer with the
    /// database's one name rule (exact, then ASCII case-insensitive).
    pub block: String,
    pub tag: String,
    pub prompt: String,
    pub default_value: String,
    pub flags: AttributeFlags,
    pub order: Option<u32>,
    pub position: Point3,
    pub height: f64,
    pub rotation: f64,
    pub width_factor: f64,
    pub wrap_width: Option<f64>,
    pub oblique: f64,
}

impl Default for AttributeDefinitionCommand {
    fn default() -> Self {
        AttributeDefinitionCommand {
            block: String::new(),
            tag: String::new(),
            prompt: String::new(),
            default_value: String::new(),
            flags: AttributeFlags::default(),
            order: None,
            position: Point3::default(),
            height: 1.0,
            rotation: 0.0,
            width_factor: 1.0,
            wrap_width: None,
            oblique: 0.0,
        }
    }
}

/// Which references a value edit reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeEditScope {
    /// The current selection; every selected object must be a direct reference
    /// to the named block. The individual commands additionally require exactly
    /// one reference.
    Selection,
    /// Every `INSERT` of the block in the document, nested references included
    /// (`GATTE`, `-ATTEDIT`).
    AllInstances,
}

/// One `TAG <new value>` assignment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeAssignment {
    pub tag: String,
    pub value: String,
}

/// What one `ATTEDIT`/`-ATTEDIT`/`EATTEDIT`/`ATTIPEDIT`/`GATTE` invocation edits.
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeEditCommand {
    pub block: String,
    pub assignments: Vec<AttributeAssignment>,
    pub scope: AttributeEditScope,
    /// `EATTEDIT` also lets the value's text placement move; `None` leaves the
    /// stored placement untouched. A definition flagged `locked_position`
    /// refuses a non-`None` position (`attribute.locked_position`) before any
    /// write.
    pub position: Option<Point3>,
    /// `EATTEDIT` text height override; `None` keeps the stored height.
    pub height: Option<f64>,
    /// `EATTEDIT` text rotation in radians; `None` keeps the stored rotation.
    pub rotation: Option<f64>,
}

/// `ATTSYNC`: the block whose definition is propagated to its references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeSyncCommand {
    pub block: String,
}

/// `BATTORDER`: the block and its complete new tag order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeOrderCommand {
    pub block: String,
    pub tags: Vec<String>,
}

/// The `ATTDISP` display mode (AutoCAD's `Normal`/`On`/`Off`).
///
/// This is a **session display setting**, not document data: it never writes the
/// drawing, bumps no revision and records no undo step. `Normal` draws visible
/// values and honours a definition's `invisible` flag; `On` draws every value;
/// `Off` hides every value. The renderer lane (ADR-0055 M2) consumes this mode;
/// the command layer only stores it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeDisplayMode {
    Normal,
    On,
    Off,
}

impl AttributeDisplayMode {
    /// The stable uppercase token used by the command line and diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            AttributeDisplayMode::Normal => "NORMAL",
            AttributeDisplayMode::On => "ON",
            AttributeDisplayMode::Off => "OFF",
        }
    }
}

/// One block-attribute command payload (ADR-0055 M3).
///
/// The selectors keep their own `ExtendedCommandId`, but their inputs share one
/// payload family so the command layer can route one typed value and the
/// host-facing enum gains exactly one variant.
#[derive(Debug, Clone, PartialEq)]
pub enum AttributeCommand {
    /// `ATTDEF` / `-ATTDEF` / `ATTREDEF`.
    Definition(AttributeDefinitionCommand),
    /// `ATTEDIT` / `-ATTEDIT` / `EATTEDIT` / `ATTIPEDIT` / `GATTE`.
    Edit(AttributeEditCommand),
    /// `ATTSYNC`.
    Sync(AttributeSyncCommand),
    /// `BATTORDER`.
    Order(AttributeOrderCommand),
    /// `ATTDISP`.
    Display(AttributeDisplayMode),
}
