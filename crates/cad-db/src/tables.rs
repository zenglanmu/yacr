//! Layer, block, layout and style tables.

use cad_domain::*;

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
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
