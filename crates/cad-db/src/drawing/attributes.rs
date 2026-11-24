//! Block attribute helpers (ADR-0055).
//!
//! Definitions live as ordinary member entities of a block (their geometry is
//! [`SemanticGeometry::AttributeDefinition`]); the values a reference carries
//! live in [`SemanticGeometry::Insert::attributes`]. This module owns the one
//! lookup that turns a block's definitions into the default value list a fresh
//! `INSERT` starts from. No command or importer is wired to it in this slice.

use cad_domain::*;

use super::DrawingDatabase;

impl DrawingDatabase {
    /// Materialize the default values for `block`'s attribute definitions.
    ///
    /// Every `ATTDEF` member contributes one [`AttributeValue`] whose tag, flags
    /// and text placement are copied from the definition and whose value starts
    /// as the definition's default. Values are ordered by [`AttributeDefinition::order`];
    /// equal orders keep the block's member order (stable sort), so the result is
    /// deterministic without inventing a secondary key.
    ///
    /// An unknown block, or a block with no definitions, returns an empty list —
    /// never a fabricated attribute. The definition's layer is used for the
    /// value (the DXF `ATTRIB` inherits the `ATTDEF`'s layer until edited).
    pub fn default_attribute_values(&self, block: BlockId) -> Vec<AttributeValue> {
        let Some(definition) = self.blocks.get(&block) else {
            return Vec::new();
        };
        let mut values: Vec<(u32, AttributeValue)> = definition
            .entities
            .iter()
            .filter_map(|member| {
                let entity = self.entities.get(member)?;
                let SemanticGeometry::AttributeDefinition(attribute) = &entity.geometry else {
                    return None;
                };
                Some((
                    attribute.order,
                    AttributeValue::from_definition(attribute, entity.layer),
                ))
            })
            .collect();
        values.sort_by_key(|(order, _)| *order);
        values.into_iter().map(|(_, value)| value).collect()
    }
}

#[cfg(test)]
mod tests {
    use cad_domain::{
        AttributeDefinition, AttributeFlags, AttributeText, Point3, SemanticGeometry, StyleId,
        TextAlignH, TextAlignV, TextLineSpacing,
    };

    use crate::builder::DrawingDatabaseBuilder;
    use crate::entity::{DbEntity, DbObject};
    use crate::tables::{BlockDefinition, Layer};
    use cad_domain::{BlockId, EntityId, LayerId, ObjectId, Revision, SpaceId};

    fn definition(tag: &str, order: u32, default_value: &str) -> SemanticGeometry {
        SemanticGeometry::AttributeDefinition(Box::new(AttributeDefinition {
            tag: tag.to_string(),
            prompt: format!("{tag}?"),
            default_value: default_value.to_string(),
            flags: AttributeFlags::default(),
            order,
            text: AttributeText {
                position: Point3 {
                    x: 1.0,
                    y: 2.0,
                    z: 0.0,
                },
                height: 2.5,
                rotation: 0.0,
                style: StyleId(0),
                width_factor: 1.0,
                h_align: TextAlignH::Left,
                v_align: TextAlignV::Baseline,
                wrap_width: None,
                line_spacing: TextLineSpacing::default(),
                font: None,
                oblique: 0.0,
            },
            source_handle: None,
        }))
    }

    fn member(id: u128, block: BlockId, geometry: SemanticGeometry) -> DbEntity {
        DbEntity {
            object: DbObject {
                id: ObjectId(id),
                type_key: "AcDbAttributeDefinition".to_string(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(id),
            layer: LayerId(1),
            space: SpaceId::Block(block),
            geometry,
            draw_order: 0,
        }
    }

    #[test]
    fn defaults_follow_definition_order_and_keep_the_tag_value() {
        let block = BlockId(1);
        let mut builder = DrawingDatabaseBuilder::new(cad_domain::DatabaseId(1));
        builder
            .insert_layer(Layer {
                id: LayerId(1),
                name: "0".to_string(),
                visible: true,
            })
            .expect("layer");
        builder
            .insert_block(BlockDefinition {
                id: block,
                entities: vec![EntityId(2), EntityId(1)],
                dynamic_visibility: None,
            })
            .expect("block");
        builder
            .insert_entity(member(2, block, definition("SECOND", 2, "b")))
            .expect("second member");
        builder
            .insert_entity(member(1, block, definition("FIRST", 1, "a")))
            .expect("first member");
        let db = builder.finish().expect("database");

        let values = db.default_attribute_values(block);
        assert_eq!(
            values
                .iter()
                .map(|value| (value.tag.as_str(), value.value.as_str()))
                .collect::<Vec<_>>(),
            vec![("FIRST", "a"), ("SECOND", "b")]
        );
        assert!(values.iter().all(|value| value.layer == LayerId(1)));

        // An unknown block is an empty list, never a fabrication.
        assert!(db.default_attribute_values(BlockId(99)).is_empty());
    }
}
