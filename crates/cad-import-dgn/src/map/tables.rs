//! Layers, linetypes, text styles and block identity tables.

use cad_db::{BlockDefinition, Layer, LineType, LinetypePattern, Style};
use cad_domain::{BlockId, LayerId, LinetypeId, Point3, StyleId, UnitContext};

use super::{document_units_per_uor, domain_point, finite_point, Mapper};
use crate::{
    GEOMETRY_REJECTED, LEVEL_SYMBOLOGY_UNDECODED, LINETYPE_PATTERN_UNSCALED, UNITS_UNSUPPORTED,
};

impl Mapper<'_> {
    /// Map the unit-derived tables and the primary model's linetype scale.
    pub(crate) fn map_tables(&mut self) -> cad_domain::CadResult<()> {
        if let Some(codepage) = &self.doc.source.codepage {
            self.note(
                crate::TEXT_ENCODING,
                format!(
                    "8-bit DGN strings were decoded with the reader's '{codepage}' code page; \
                     the import exposes the reader's choice rather than re-decoding text"
                ),
                None,
            );
        }
        let model = self.primary_model();
        self.uor_per_document_unit = model.and_then(document_units_per_uor);
        self.units = self.unit_context();
        self.read_layers()?;
        self.read_linetypes()?;
        self.read_styles()?;
        self.read_blocks()?;
        Ok(())
    }

    fn unit_context(&mut self) -> UnitContext {
        let Some(meters) = self.doc.units.meters_per_unit() else {
            self.note(
                "import.dgn.units_unitless",
                "the DGN file declares no master-unit size; coordinates are reported as drawing \
                 units, never assumed to be millimetres"
                    .into(),
                None,
            );
            return UnitContext::drawing_units();
        };
        let close = |target: f64| (meters - target).abs() <= target * 1e-9;
        let unit = if close(1.0) {
            cad_domain::Unit::Meter
        } else if close(0.001) {
            cad_domain::Unit::Millimeter
        } else if close(0.0254) {
            cad_domain::Unit::Inch
        } else if close(0.3048) {
            cad_domain::Unit::Foot
        } else {
            self.units_partial = true;
            self.note(
                UNITS_UNSUPPORTED,
                format!(
                    "the DGN master unit is {meters} m per unit, which the domain's unit set \
                     (mm/m/in/ft) cannot represent; coordinates stay in the file's own numbers \
                     and are reported as drawing units"
                ),
                None,
            );
            return UnitContext::drawing_units();
        };
        let name = match unit {
            cad_domain::Unit::Meter => "meter",
            cad_domain::Unit::Millimeter => "millimeter",
            cad_domain::Unit::Inch => "inch",
            cad_domain::Unit::Foot => "foot",
            cad_domain::Unit::DrawingUnits => "drawing_units",
        };
        self.note(
            &format!("import.dgn.units_{name}"),
            format!(
                "the DGN master unit is {meters} m per drawing unit; stored coordinates are in \
                 that unit and the display unit follows it"
            ),
            None,
        );
        UnitContext {
            source: unit.clone(),
            display: unit,
            display_per_source: Some(1.0),
            decimal_places: 3,
        }
    }

    fn read_layers(&mut self) -> cad_domain::CadResult<()> {
        let doc = self.doc;
        for (index, layer) in doc.layers.iter().enumerate() {
            let id = LayerId(index as u128);
            if self.layer_names.contains_key(&id) || self.layers.contains_key(&layer.name) {
                continue;
            }
            self.builder.insert_layer(Layer {
                id,
                name: layer.name.clone(),
                visible: layer.visible && !layer.frozen,
            })?;
            self.layers.insert(layer.name.clone(), id);
            self.layer_keys
                .entry(layer.name.to_ascii_lowercase())
                .or_insert(id);
            self.layer_names.insert(id, layer.name.clone());
            self.next_layer = self.next_layer.max(index as u128 + 1);
        }
        // cadkit decodes no level colour, weight, linetype or freeze/lock bit;
        // the raw flags stay in the document's props, never in a fabricated
        // LayerAttributes row. Display state is therefore always partial.
        self.note(
            LEVEL_SYMBOLOGY_UNDECODED,
            format!(
                "{} DGN level row(s) were imported by name and visibility only; cadkit-dgn \
                 decodes no level colour, lineweight, linetype or freeze/lock flag, so no layer \
                 attribute row is written",
                doc.layers.len()
            ),
            None,
        );
        Ok(())
    }

    fn read_linetypes(&mut self) -> cad_domain::CadResult<()> {
        let doc = self.doc;
        for (index, linetype) in doc.linetypes.iter().enumerate() {
            let id = LinetypeId(index as u128);
            let key = linetype.name.to_ascii_lowercase();
            let pattern = match self.uor_per_document_unit {
                Some(uor) if uor.is_finite() && uor > 0.0 => LinetypePattern::from_elements(
                    linetype.pattern.iter().map(|element| element / uor),
                ),
                _ => {
                    if linetype.pattern.iter().any(|element| *element != 0.0) {
                        self.unscaled_linetypes += 1;
                        self.note(
                            LINETYPE_PATTERN_UNSCALED,
                            format!(
                                "linetype '{}' stores its dash pattern in UOR, but the file gives \
                                 no usable UOR-to-document-unit factor; the linetype was stored \
                                 continuous rather than at a guessed scale",
                                linetype.name
                            ),
                            None,
                        );
                    }
                    LinetypePattern::continuous()
                }
            };
            self.builder.insert_linetype(LineType {
                id,
                name: linetype.name.clone(),
                pattern: pattern.clone(),
                complex: false,
            })?;
            self.linetypes.insert(key.clone(), id);
            self.linetype_patterns.insert(key, pattern);
        }
        Ok(())
    }

    fn read_styles(&mut self) -> cad_domain::CadResult<()> {
        // The domain's text geometry stores a StyleId unconditionally; make the
        // zero row real so no stored text references a missing style.
        self.builder.insert_style(Style {
            id: StyleId(0),
            name: "Standard".into(),
            resource_keys: Vec::new(),
        })?;
        self.styles.insert("Standard".into(), StyleId(0));
        self.style_keys.insert("standard".into(), StyleId(0));
        self.style_fonts.insert("standard".into(), None);

        let doc = self.doc;
        for style in &doc.text_styles {
            let keys: Vec<String> = style.font.iter().cloned().collect();
            let id = if style.name == "Standard" {
                StyleId(0)
            } else {
                let id = StyleId(self.next_style);
                self.next_style += 1;
                id
            };
            self.builder.insert_style(Style {
                id,
                name: style.name.clone(),
                resource_keys: keys,
            })?;
            self.styles.insert(style.name.clone(), id);
            self.style_keys.insert(style.name.to_ascii_lowercase(), id);
            self.style_fonts
                .insert(style.name.to_ascii_lowercase(), style.font.clone());
        }
        Ok(())
    }

    fn read_blocks(&mut self) -> cad_domain::CadResult<()> {
        let doc = self.doc;
        for (index, block) in doc.blocks.iter().enumerate() {
            let id = BlockId(index as u128);
            let base = finite_point(domain_point(block.base_point)).unwrap_or_else(|| {
                self.rejected += 1;
                self.note(
                    GEOMETRY_REJECTED,
                    format!(
                        "shared cell definition '{}' has a non-finite base point; its members \
                         were placed from the origin",
                        block.name
                    ),
                    None,
                );
                Point3::default()
            });
            self.builder.insert_block(BlockDefinition {
                id,
                entities: Vec::new(),
                dynamic_visibility: None,
            })?;
            self.builder.set_block_metadata(id, &block.name, base)?;
            self.blocks.insert(block.name.clone(), id);
            self.block_keys
                .entry(block.name.to_ascii_lowercase())
                .or_insert(id);
            self.block_bases.insert(id, base);
        }
        self.compute_block_depths();
        Ok(())
    }

    fn compute_block_depths(&mut self) {
        let doc = self.doc;
        let mut memo = std::collections::BTreeMap::new();
        let mut names: Vec<String> = doc.blocks.iter().map(|block| block.name.clone()).collect();
        names.sort();
        names.dedup();
        let mut cycles = 0usize;
        let mut cyclic = std::collections::BTreeSet::new();
        for name in &names {
            let mut visiting = std::collections::BTreeSet::new();
            let depth = super::blocks::block_depth(
                doc,
                name,
                &mut visiting,
                &mut memo,
                &mut cycles,
                &mut cyclic,
            );
            if depth > self.max_depth {
                self.depth_limited += 1;
                if let Some(id) = self.blocks.get(name) {
                    self.block_unsafe.insert(*id);
                }
            }
        }
        for name in &cyclic {
            if let Some(id) = self.blocks.get(name) {
                self.block_unsafe.insert(*id);
            }
        }
        if cycles > 0 {
            self.depth_limited += 1;
            self.note(
                crate::DEPTH_LIMIT,
                format!(
                    "{cycles} shared cell definition reference cycle(s) (for example A -> B -> A)                      were found; the definitions are stored once and expanding them would recurse"
                ),
                None,
            );
        }
        if self.depth_limited > 0 {
            self.note(
                crate::DEPTH_LIMIT,
                format!(
                    "{} shared cell definition(s) nest deeper than the {} level import limit, or \
                     close a reference cycle; the definitions are stored once, not expanded, and \
                     expanding them may be refused by the representation",
                    self.depth_limited, self.max_depth
                ),
                None,
            );
        }
    }
}
