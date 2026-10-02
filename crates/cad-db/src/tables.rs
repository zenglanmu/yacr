//! Layer, block, layout and style tables.

use crate::entity::LinetypePattern;
use cad_domain::*;

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
}

/// A named linetype table entry (spec §3.2 / §7.1).
///
/// `pattern` is the resolved dash pattern; `complex` records whether the source
/// linetype carried shape/text elements that this build cannot draw. A complex
/// linetype still stores its dash segments so the line is at least dashed, and
/// the importer reports the omitted glyphs as a `Partial` reason.
#[derive(Debug, Clone, PartialEq)]
pub struct LineType {
    pub id: LinetypeId,
    pub name: String,
    pub pattern: LinetypePattern,
    /// `true` when the source linetype had embedded shape/text content.
    pub complex: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockDefinition {
    pub id: BlockId,
    pub entities: Vec<EntityId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub id: LayoutId,
    pub name: String,
    pub viewports: Vec<PaperViewport>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PaperViewport {
    pub clip: Vec<Point3>,
    pub model_to_paper: Transform3,
    pub completeness: Completeness,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Style {
    pub id: StyleId,
    pub name: String,
    pub resource_keys: Vec<String>,
}
