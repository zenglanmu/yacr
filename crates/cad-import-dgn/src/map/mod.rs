//! Mapping of a `cadkit_core::Document` (DGN V7/V8) into a `cad-db` database.
//!
//! This is a **vector** mapping, not a mesh one: layers (DGN levels), linetypes,
//! text styles, shared-cell definitions/blocks and the model's entities are
//! written as first-class database rows. Only the primary design model is
//! imported; every other model is reported, never guessed into model space.
//!
//! Nothing here invents state. `cadkit-dgn` decodes no level colour, weight,
//! freeze or lock bit, so layer rows carry only their name and visibility and
//! **no** `LayerAttributes` row is written (writing defaults would look like
//! real data). The same rule applies to tags, rasters, unknown elements and
//! unsupported `EntityKind`s: each is a stable diagnostic plus `Partial`, never
//! a silently dropped or fabricated value.

mod blocks;
mod entities;
mod report;
mod tables;

use std::collections::{BTreeMap, BTreeSet};

use cad_db::{DrawingDatabaseBuilder, EntityColor, EntityLineWeight, LinetypePattern, Style};
use cad_domain::{
    CadError, CadResult, Diagnostic, EntityId, LayerId, ObjectId, Point3, SemanticGeometry,
    StyleId, UnitContext,
};
use cadkit_core::{Document, Model, ModelKind, Value};

use crate::{NO_GEOMETRY, UNKNOWN_ELEMENTS_ONLY};

/// One resolved entity style after inheritance from a flattened group.
#[derive(Clone)]
pub(super) struct Inherit {
    pub(super) layer_name: Option<String>,
    pub(super) color: EntityColor,
    pub(super) linetype: LinetypeChoice,
    pub(super) lineweight: EntityLineWeight,
    pub(super) visible: bool,
}

impl Default for Inherit {
    fn default() -> Self {
        Inherit {
            layer_name: None,
            color: EntityColor::ByLayer,
            linetype: LinetypeChoice::ByLayer,
            lineweight: EntityLineWeight::ByLayer,
            visible: true,
        }
    }
}

#[derive(Clone)]
pub(super) enum LinetypeChoice {
    ByLayer,
    ByBlock,
    Named(String),
}

pub(super) struct Resolved {
    pub(super) layer: LayerId,
    pub(super) layer_name: String,
    pub(super) color: EntityColor,
    pub(super) linetype: LinetypeChoice,
    pub(super) lineweight: EntityLineWeight,
    pub(super) visible: bool,
}

impl Resolved {
    fn inherited(&self) -> Inherit {
        Inherit {
            layer_name: Some(self.layer_name.clone()),
            color: self.color,
            linetype: self.linetype.clone(),
            lineweight: self.lineweight,
            visible: self.visible,
        }
    }
}

pub(super) struct Mapped {
    pub(super) geometry: SemanticGeometry,
    pub(super) partial: bool,
    pub(super) type_key: &'static str,
}

/// Builds one imported DGN database.
pub(crate) struct Mapper<'a> {
    pub(super) doc: &'a Document,
    pub(super) cancelled: &'a dyn Fn() -> bool,
    pub(super) builder: DrawingDatabaseBuilder,

    pub(super) layers: BTreeMap<String, LayerId>,
    pub(super) layer_keys: BTreeMap<String, LayerId>,
    pub(super) layer_names: BTreeMap<LayerId, String>,
    pub(super) next_layer: u128,
    pub(super) fallback_layer: Option<LayerId>,

    pub(super) linetypes: BTreeMap<String, cad_domain::LinetypeId>,
    pub(super) linetype_patterns: BTreeMap<String, LinetypePattern>,

    pub(super) styles: BTreeMap<String, StyleId>,
    pub(super) style_keys: BTreeMap<String, StyleId>,
    pub(super) style_fonts: BTreeMap<String, Option<String>>,
    pub(super) next_style: u128,

    pub(super) blocks: BTreeMap<String, cad_domain::BlockId>,
    pub(super) block_keys: BTreeMap<String, cad_domain::BlockId>,
    pub(super) block_bases: BTreeMap<cad_domain::BlockId, Point3>,
    pub(super) active_blocks: BTreeSet<cad_domain::BlockId>,
    /// Definitions that must not be instantiated: their insert chain either
    /// exceeds `max_depth` or closes a reference cycle. Emitting an `Insert`
    /// into one would make the representation recurse (or refuse the row on the
    /// first edit), so the instance is skipped with an explicit diagnostic.
    pub(super) block_unsafe: BTreeSet<cad_domain::BlockId>,

    /// Shared `ObjectId`/`EntityId` counter; the two ids always share the
    /// numeric value, exactly as the other importers allocate them.
    pub(super) next_object: u128,
    pub(super) draw_order: i64,
    pub(super) inserted: usize,
    pub(super) hidden: Vec<EntityId>,
    pub(super) max_entities: usize,
    pub(super) max_depth: usize,

    pub(super) units: UnitContext,
    pub(super) uor_per_document_unit: Option<f64>,

    pub(super) diagnostics: Vec<Diagnostic>,
    pub(super) kinds: BTreeMap<&'static str, bool>,
    pub(super) unknown_types: BTreeMap<String, usize>,
    pub(super) unmodelled_types: BTreeMap<String, usize>,
    pub(super) unknown_linetypes: BTreeSet<String>,
    pub(super) unknown_layers: usize,
    pub(super) tags: usize,
    pub(super) rasters: usize,
    pub(super) groups_flattened: usize,
    pub(super) rejected: usize,
    pub(super) missing_blocks: usize,
    pub(super) mirrored_cells: usize,
    pub(super) depth_limited: usize,
    pub(super) text_align_losses: usize,
    pub(super) text_plane_losses: usize,
    pub(super) text_width_losses: usize,
    pub(super) mtext_flattened: usize,
    pub(super) unscaled_linetypes: usize,
    pub(super) units_partial: bool,
    pub(super) skipped_models: usize,
}

impl<'a> Mapper<'a> {
    pub(crate) fn new(
        database: cad_domain::DatabaseId,
        doc: &'a Document,
        limits: &cad_import_core::ImportLimits,
        cancelled: &'a dyn Fn() -> bool,
    ) -> Self {
        Mapper {
            doc,
            cancelled,
            builder: DrawingDatabaseBuilder::new(database),
            layers: BTreeMap::new(),
            layer_keys: BTreeMap::new(),
            layer_names: BTreeMap::new(),
            next_layer: 0,
            fallback_layer: None,
            linetypes: BTreeMap::new(),
            linetype_patterns: BTreeMap::new(),
            styles: BTreeMap::new(),
            style_keys: BTreeMap::new(),
            style_fonts: BTreeMap::new(),
            next_style: 1,
            blocks: BTreeMap::new(),
            block_keys: BTreeMap::new(),
            block_bases: BTreeMap::new(),
            active_blocks: BTreeSet::new(),
            block_unsafe: BTreeSet::new(),
            next_object: 1,
            draw_order: 0,
            inserted: 0,
            hidden: Vec::new(),
            max_entities: limits.max_entities,
            max_depth: limits.max_block_depth,
            units: UnitContext::drawing_units(),
            uor_per_document_unit: None,
            diagnostics: Vec::new(),
            kinds: BTreeMap::new(),
            unknown_types: BTreeMap::new(),
            unmodelled_types: BTreeMap::new(),
            unknown_linetypes: BTreeSet::new(),
            unknown_layers: 0,
            tags: 0,
            rasters: 0,
            groups_flattened: 0,
            rejected: 0,
            missing_blocks: 0,
            mirrored_cells: 0,
            depth_limited: 0,
            text_align_losses: 0,
            text_plane_losses: 0,
            text_width_losses: 0,
            mtext_flattened: 0,
            unscaled_linetypes: 0,
            units_partial: false,
            skipped_models: 0,
        }
    }

    pub(super) fn note(&mut self, code: &str, message: String, object: Option<ObjectId>) {
        self.diagnostics.push(Diagnostic {
            object,
            code: code.to_string(),
            message,
        });
    }

    /// Entities inserted so far (model + block members), for progress ticks.
    pub(crate) fn inserted_count(&self) -> usize {
        self.inserted
    }

    pub(super) fn primary_model(&self) -> Option<&'a Model> {
        let index = self.primary_model_index()?;
        let doc: &'a Document = self.doc;
        doc.models.get(index)
    }

    pub(super) fn primary_model_index(&self) -> Option<usize> {
        if self.doc.models.is_empty() {
            return None;
        }
        Some(
            self.doc
                .models
                .iter()
                .position(|model| model.kind == ModelKind::Model)
                .unwrap_or(0),
        )
    }

    pub(super) fn layer_id(&mut self, name: Option<&str>) -> CadResult<LayerId> {
        if let Some(name) = name {
            if let Some(id) = self
                .layers
                .get(name)
                .or_else(|| self.layer_keys.get(&name.to_ascii_lowercase()))
            {
                return Ok(*id);
            }
            self.unknown_layers += 1;
        }
        self.fallback_layer()
    }

    fn fallback_layer(&mut self) -> CadResult<LayerId> {
        if let Some(id) = self.fallback_layer {
            return Ok(id);
        }
        if let Some(id) = self
            .layers
            .get("0")
            .or_else(|| self.layer_keys.get("0"))
            .copied()
        {
            self.fallback_layer = Some(id);
            return Ok(id);
        }
        let id = LayerId(self.next_layer);
        self.next_layer += 1;
        self.builder.insert_layer(cad_db::Layer {
            id,
            name: "0".into(),
            visible: true,
        })?;
        self.layers.insert("0".into(), id);
        self.layer_keys.insert("0".into(), id);
        self.layer_names.insert(id, "0".into());
        self.fallback_layer = Some(id);
        Ok(id)
    }

    pub(super) fn style_id(&mut self, name: Option<&str>) -> CadResult<StyleId> {
        let Some(name) = name.filter(|name| !name.trim().is_empty()) else {
            return Ok(StyleId(0));
        };
        if let Some(id) = self
            .styles
            .get(name)
            .or_else(|| self.style_keys.get(&name.to_ascii_lowercase()))
        {
            return Ok(*id);
        }
        let id = StyleId(self.next_style);
        self.next_style += 1;
        self.builder.insert_style(Style {
            id,
            name: name.to_string(),
            resource_keys: Vec::new(),
        })?;
        self.styles.insert(name.to_string(), id);
        self.style_keys.insert(name.to_ascii_lowercase(), id);
        self.style_fonts.insert(name.to_ascii_lowercase(), None);
        Ok(id)
    }

    pub(super) fn style_font(&self, name: Option<&str>) -> Option<String> {
        let name = name?;
        self.style_fonts
            .get(&name.to_ascii_lowercase())
            .cloned()
            .flatten()
    }

    pub(super) fn block(&self, name: &str) -> Option<(cad_domain::BlockId, Point3)> {
        let id = self
            .blocks
            .get(name)
            .or_else(|| self.block_keys.get(&name.to_ascii_lowercase()))
            .copied()?;
        let base = self.block_bases.get(&id).copied().unwrap_or_default();
        Some((id, base))
    }
}

pub(super) fn finite_point(point: Point3) -> Option<Point3> {
    (point.x.is_finite() && point.y.is_finite() && point.z.is_finite()).then_some(point)
}

/// Document units per UOR for a model.
///
/// `dgn.rescaled_to_document_units` is the effective factor after the reader
/// scaled another model into the document's units; `dgn.uor_per_master` is the
/// model's own UOR count.
pub(super) fn document_units_per_uor(model: &Model) -> Option<f64> {
    let value = model
        .props
        .get("dgn.rescaled_to_document_units")
        .or_else(|| model.props.get("dgn.uor_per_master"))
        .and_then(Value::as_f64)?;
    (value.is_finite() && value > 0.0).then_some(value)
}

/// `cadkit` and `cad-domain` have separate `Point3`/`Vec3` types; convert
/// explicitly so no coordinate is reinterpreted by a bit cast.
pub(super) fn domain_point(point: cadkit_core::Point3) -> Point3 {
    Point3 {
        x: point.x,
        y: point.y,
        z: point.z,
    }
}

pub(super) fn domain_vec(vector: cadkit_core::Vec3) -> Point3 {
    Point3 {
        x: vector.x,
        y: vector.y,
        z: vector.z,
    }
}

pub(super) fn summarize(items: &[&str]) -> String {
    const LIMIT: usize = 6;
    if items.len() <= LIMIT {
        return items.join(", ");
    }
    format!(
        "{} and {} more",
        items[..LIMIT].join(", "),
        items.len() - LIMIT
    )
}

pub(super) fn no_geometry_message(mapper: &Mapper<'_>) -> String {
    format!(
        "{NO_GEOMETRY}: the DGN file was read but produced no drawable entity ({} model entity \
         row(s) were seen)",
        mapper
            .primary_model()
            .map(|model| model.entities.len())
            .unwrap_or(0)
    )
}

pub(super) fn unknown_only_message(names: &[String], unsupported: usize) -> String {
    format!(
        "{UNKNOWN_ELEMENTS_ONLY}: the DGN file holds {unsupported} element(s) of types this \
         build does not model ({}) and nothing drawable; supported vector types are point, line, \
         polyline, circle, arc, ellipse, spline, text, shared cell and raster placement",
        names.join(", ")
    )
}

pub(super) fn invariant(message: &str) -> CadError {
    CadError::Invariant(message.to_string())
}
