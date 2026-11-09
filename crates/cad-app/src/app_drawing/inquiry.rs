//! Read-only inquiry commands: `LIST`, `STATUS` and `FS`.
//!
//! All three are **report-only**: they open no transaction, bump no revision,
//! record no undo entry and never publish a `ChangeSet`. `LIST` and `STATUS`
//! also mutate no session state; `FS` mutates only the session selection (a
//! read-only union), so Viewer and Work both admit the three commands.
//!
//! Declared divergences from AutoCAD (pinned in `docs/autocad-2026-command-coverage.md`):
//!
//! * `LIST` reads the **current selection** instead of prompting "Select
//!   objects"; a non-addressable ref is reported per-ref (`list.not_addressable`)
//!   and does not abort the whole report. A directly addressable entity is
//!   reported in **any** space — model, paper and block definition rows all
//!   resolve through the same `drawing.entity(id)` path (block-instance
//!   *members* still need their own ref path and are reported, not expanded).
//!   The report names the entity's space and every display property this build
//!   models: canonical colour/linetype/linetype scale/lineweight/transparency
//!   (ByLayer/ByBlock or the explicit value), the baked hidden (visibility)
//!   flag, the geometry source, the annotative flag, the resolved TEXT style
//!   name, the DIMENSION style name and (in the geometry summary) the INSERT
//!   block name. Thickness and extrusion are **not modelled** by this build and
//!   the report says so; 3D data is out of scope build-wide;
//! * `STATUS` does not track drawing creation/edit time, memory usage, drawing
//!   limits or the base point (this build has no drawing-header model), and says
//!   so with explicit `status.not_tracked` diagnostics;
//! * `FS` is an Express tool (command-line only, no ribbon button) and implements
//!   AutoCAD's default `FSMODE=OFF`: it adds only **directly touching** entities
//!   — a crossing and a tangent contact both count — never a transitive chain.
//!   Chain mode (`FSMODE=ON`) is **NOT IMPLEMENTED**.
//!   It enumerates directly addressable model-space entities only — block
//!   instance members are not expanded, and hidden layers are skipped.

use crate::picking::ref_visibility;
use crate::selection::model_ref;
use cad_domain::TolerancePolicy;
use cad_geometry::{distance, DefaultGeometryEngine, GeometryEngine};

use super::*;

fn document_drawing(
    app: &Application,
    command: &Command,
) -> CadResult<Arc<cad_db::DrawingDatabase>> {
    let document = app
        .workspace
        .documents
        .get(&command.document)
        .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
    if document.id != command.document {
        return Err(CadError::StaleResult);
    }
    Ok(document.drawing.clone())
}

fn require_none_payload(command: &Command, name: &str) -> CadResult<()> {
    if matches!(command.payload, CommandPayload::None) {
        Ok(())
    } else {
        Err(CadError::InvalidInput(format!("{name} needs no payload")))
    }
}

fn fmt_point(p: Point3) -> String {
    format!("({:.6}, {:.6}, {:.6})", p.x, p.y, p.z)
}

fn space_label(space: &SpaceId) -> String {
    match space {
        SpaceId::Model => "Model".to_string(),
        SpaceId::Paper(layout) => format!("Paper({})", layout.0),
        SpaceId::Block(block) => format!("Block({})", block.0),
    }
}

fn layer_label(database: &cad_db::DrawingDatabase, layer: LayerId) -> String {
    match database.layer(layer) {
        Some(row) => row.name.clone(),
        None => format!("unresolved({})", layer.0),
    }
}

fn handle_label(entity: &cad_db::DbEntity) -> String {
    entity
        .object
        .source_handle
        .as_deref()
        .map(str::trim)
        .filter(|handle| !handle.is_empty())
        .unwrap_or("-")
        .to_string()
}

/// Canonical display-property projection for the `LIST` report.
///
/// It reads [`cad_db::DrawingDatabase::entity_render_attributes`] and the
/// entity's baked hidden flag through [`cad_db::DrawingDatabase::entity_hidden`]
/// (the authoritative imported display state for top-level rows), plus the
/// canonical round-trippable `SETENTITYPROPERTY` strings (`ByLayer`/`ByBlock`
/// stay symbolic; an explicit value is shown verbatim). `ltscale` is the
/// **effective** factor: [`cad_db::sanitize_linetype_scale`] maps a meaningless
/// non-finite/non-positive stored value to the identity `1.0` (the single rule
/// the importer and representation also use). Thickness and extrusion have no
/// stored model in this build, so the report says `not-modelled` instead of
/// inventing them.
fn display_summary(database: &cad_db::DrawingDatabase, entity: &cad_db::DbEntity) -> String {
    let attributes = database.entity_render_attributes(entity.id);
    let value = |key: &str| {
        crate::property_value::property_value(database, entity, key)
            .unwrap_or_else(|| "unresolved".to_string())
    };
    format!(
        "color={} linetype={} ltscale={:.6} lineweight={} transparency={} visible={} \
         source={:?} annotative={} thickness=not-modelled extrusion=not-modelled",
        value("color"),
        value("linetype"),
        cad_db::sanitize_linetype_scale(attributes.linetype_scale),
        value("lineweight"),
        value("transparency"),
        !database.entity_hidden(entity.id),
        attributes.geometry_source,
        attributes.annotative.annotative
    )
}

/// The type-specific names the report resolves from the drawing tables.
///
/// A TEXT keeps its `StyleId`; the name is resolved from the style table and
/// reported as `-` when the row is missing (never fabricated). A DIMENSION
/// carries its style **name** snapshot in the geometry, which is reported
/// verbatim. Every other kind returns an empty string so the stable prefix of
/// the report is unchanged.
fn type_summary(database: &cad_db::DrawingDatabase, geometry: &SemanticGeometry) -> String {
    match geometry {
        SemanticGeometry::Text { style, .. } => format!(
            " text_style={}",
            database
                .style(*style)
                .map(|row| row.name.as_str())
                .unwrap_or("-")
        ),
        SemanticGeometry::Dimension(dimension) => {
            format!(" dim_style={}", dimension.style_name())
        }
        _ => String::new(),
    }
}

/// A single-line, locale-neutral truncation of a text entity's stored string.
fn truncate_text(text: &str) -> String {
    const LIMIT: usize = 40;
    let mut out = String::new();
    for (index, ch) in text.chars().enumerate() {
        if index >= LIMIT {
            out.push_str("...");
            break;
        }
        match ch {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

/// Compact geometry parameters for the `LIST` report.
///
/// Every arm reports only values the entity really stores; a value this build
/// cannot resolve is spelled out (`unresolved(...)`, `-`) rather than invented.
fn geometry_summary(database: &cad_db::DrawingDatabase, geometry: &SemanticGeometry) -> String {
    match geometry {
        SemanticGeometry::Line { start, end } => format!(
            "line start={} end={} length={:.6}",
            fmt_point(*start),
            fmt_point(*end),
            distance(*start, *end)
        ),
        SemanticGeometry::Circle {
            center,
            normal,
            radius,
        } => format!(
            "circle center={} normal={} radius={radius:.6}",
            fmt_point(*center),
            fmt_point(*normal)
        ),
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => format!(
            "arc center={} normal={} radius={radius:.6} start={start:.6} sweep={sweep:.6}",
            fmt_point(*center),
            fmt_point(*normal)
        ),
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start,
            sweep,
        } => format!(
            "ellipse center={} normal={} major_axis={} ratio={ratio:.6} start={start:.6} \
             sweep={sweep:.6}",
            fmt_point(*center),
            fmt_point(*normal),
            fmt_point(*major_axis)
        ),
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => format!(
            "polyline vertices={} bulges={} closed={closed}",
            points.len(),
            bulges.len()
        ),
        SemanticGeometry::Spline {
            degree,
            control_points,
            ..
        } => format!(
            "spline degree={degree} control_points={}",
            control_points.len()
        ),
        SemanticGeometry::Point(position) => format!("point position={}", fmt_point(*position)),
        SemanticGeometry::Mesh(mesh) => format!(
            "mesh vertices={} triangles={}",
            mesh.vertices.len(),
            mesh.triangle_count()
        ),
        SemanticGeometry::Insert { block, .. } => format!(
            "insert block={} name={}",
            block.0,
            database.block_name(*block).unwrap_or("-")
        ),
        SemanticGeometry::Text {
            text,
            position,
            height,
            ..
        } => format!(
            "text \"{}\" position={} height={height:.6}",
            truncate_text(text),
            fmt_point(*position)
        ),
        SemanticGeometry::Shape {
            shape_name,
            code,
            position,
            size,
            ..
        } => format!(
            "shape name={} code={code} position={} size={size:.6}",
            if shape_name.is_empty() {
                "-"
            } else {
                shape_name.as_str()
            },
            fmt_point(*position)
        ),
        SemanticGeometry::Opaque {
            type_key,
            version,
            payload,
        } => format!(
            "opaque type={type_key} version={version} payload_bytes={}",
            payload.len()
        ),
        SemanticGeometry::Image {
            pixels,
            file,
            visible,
            ..
        } => format!(
            "image file={} pixels={}x{} visible={visible}",
            file.as_deref().unwrap_or("-"),
            pixels[0],
            pixels[1]
        ),
        SemanticGeometry::Mask { boundary, inverted } => {
            format!("mask boundary={} inverted={inverted}", boundary.len())
        }
        SemanticGeometry::Compound(children) => {
            format!("compound children={}", children.len())
        }
        SemanticGeometry::Dimension(dimension) => format!(
            "dimension kind={:?} measurement={:.6} display_items={}",
            dimension.kind(),
            dimension.measurement(),
            dimension.display().len()
        ),
        SemanticGeometry::Hatch(hatch) => {
            format!(
                "hatch loops={} display_items={}",
                hatch.loops().len(),
                hatch.display().len()
            )
        }
        SemanticGeometry::MultiLeader(leader) => {
            let content = match leader.content() {
                MLeaderContent::None => "none",
                MLeaderContent::Text => "text",
            };
            format!(
                "multileader style={} content={content} roots={} lines={}",
                leader.style_name(),
                leader.roots().len(),
                leader.lines().len()
            )
        }
    }
}

impl Application {
    /// `LIST`: report every selected object's stored property data.
    pub(crate) fn list_objects(
        &self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, "LIST")?;
        if session.selection.is_empty() {
            return Err(CadError::InvalidInput(
                "list.selection: select at least one object first".into(),
            ));
        }
        let drawing = document_drawing(self, command)?;
        let mut diagnostics = Vec::with_capacity(session.selection.len() + 1);
        for reference in session.selection.refs() {
            if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
                diagnostics.push(Diagnostic {
                    object: Some(ObjectId(reference.entity.0)),
                    code: "list.not_addressable".into(),
                    message: format!(
                        "LIST: selection ref {} is a block-instance member or sub-element and is \
                         not directly addressable; no entity data was read",
                        SelectionSet::identity_label(reference)
                    ),
                });
                continue;
            }
            let Some(entity) = drawing.entity(reference.entity) else {
                diagnostics.push(Diagnostic {
                    object: Some(ObjectId(reference.entity.0)),
                    code: "list.not_addressable".into(),
                    message: format!(
                        "LIST: entity {} is not present in the drawing (deleted or foreign); no \
                         entity data was read",
                        reference.entity.0
                    ),
                });
                continue;
            };
            diagnostics.push(Diagnostic {
                object: Some(ObjectId(entity.id.0)),
                code: "list.object".into(),
                message: format!(
                    "LIST id={} handle={} type={} layer={} space={} draw_order={} {}{} {}",
                    entity.id.0,
                    handle_label(entity),
                    entity.object.type_key,
                    layer_label(&drawing, entity.layer),
                    space_label(&entity.space),
                    entity.draw_order,
                    display_summary(&drawing, entity),
                    type_summary(&drawing, &entity.geometry),
                    geometry_summary(&drawing, &entity.geometry)
                ),
            });
        }
        diagnostics.push(Diagnostic {
            object: None,
            code: "list.summary".into(),
            message: format!(
                "LIST: {} selected object(s); drawing revision {}",
                session.selection.len(),
                drawing.revision().0
            ),
        });
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }

    /// `STATUS`: report drawing statistics, modes and extents.
    pub(crate) fn drawing_status(
        &self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, "STATUS")?;
        let drawing = document_drawing(self, command)?;

        let mut model = 0usize;
        let mut paper = 0usize;
        let mut block = 0usize;
        for entity in drawing.entities() {
            match entity.space {
                SpaceId::Model => model += 1,
                SpaceId::Paper(_) => paper += 1,
                SpaceId::Block(_) => block += 1,
            }
        }
        let total = drawing.entity_count();

        let mut layers = 0usize;
        let mut off = 0usize;
        let mut frozen = 0usize;
        let mut locked = 0usize;
        for layer in drawing.layers() {
            layers += 1;
            let attributes = drawing.layer_attributes(layer.id);
            off += usize::from(attributes.off);
            frozen += usize::from(attributes.frozen);
            locked += usize::from(attributes.locked);
        }

        let bounds = match drawing.bounds() {
            Some((min, max)) => format!("min={} max={}", fmt_point(min), fmt_point(max)),
            None => "empty".to_string(),
        };
        let active_layer = layer_label(&drawing, session.active_layer);
        let zoom_history = self
            .workspace
            .viewports
            .get(&command.viewport)
            .map(|viewport| viewport.zoom_history_len().to_string())
            .unwrap_or_else(|| "unavailable".to_string());

        let diagnostics = vec![
            Diagnostic {
                object: None,
                code: "status.summary".into(),
                message: format!(
                    "STATUS drawing={} database={} revision={} entities={total} (model={model} \
                     paper={paper} block={block})",
                    command.document.0,
                    drawing.id().0,
                    drawing.revision().0
                ),
            },
            Diagnostic {
                object: None,
                code: "status.layers".into(),
                message: format!(
                    "STATUS layers={layers} off={off} frozen={frozen} locked={locked}"
                ),
            },
            Diagnostic {
                object: None,
                code: "status.tables".into(),
                message: format!(
                    "STATUS blocks={} linetypes={} text_styles={} dimstyles={} \
                     mleader_styles={} layouts={}",
                    drawing.blocks().count(),
                    drawing.linetypes().count(),
                    drawing.styles().count(),
                    drawing.dim_styles().count(),
                    drawing.mleader_styles().count(),
                    drawing.layouts().count()
                ),
            },
            Diagnostic {
                object: None,
                code: "status.bounds".into(),
                message: format!("STATUS model-space bounds: {bounds}"),
            },
            Diagnostic {
                object: None,
                code: "status.active_layer".into(),
                message: format!("STATUS current layer={active_layer}"),
            },
            Diagnostic {
                object: None,
                code: "status.session".into(),
                message: format!(
                    "STATUS session mode={:?} grid={} snap={} snap_grid={} ortho={} \
                     object_snap={}",
                    session.mode(),
                    session.grid_enabled(),
                    // AutoCAD `SNAP` (F9) is this build's grid snap; object snap
                    // is the separate `OSNAP` (F3) switch.
                    session.snap_grid_enabled(),
                    session.snap_grid_enabled(),
                    session.ortho_enabled(),
                    session.snap_enabled()
                ),
            },
            Diagnostic {
                object: None,
                code: "status.zoom_history".into(),
                message: format!(
                    "STATUS zoom-history depth (current viewport)={zoom_history} \
                     layer_overrides={}",
                    session.layer_overrides.len()
                ),
            },
            Diagnostic {
                object: None,
                code: "status.not_tracked".into(),
                message: "STATUS drawing creation/edit time: not tracked by this build".into(),
            },
            Diagnostic {
                object: None,
                code: "status.not_tracked".into(),
                message: "STATUS drawing limits and base point: not tracked by this build (there \
                          is no drawing-header model)"
                    .into(),
            },
            Diagnostic {
                object: None,
                code: "status.not_tracked".into(),
                message: "STATUS memory usage: not tracked by this build".into(),
            },
        ];
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }

    /// `FS`: add every visible model-space entity that touches the selected seed.
    pub(crate) fn selection_touching(
        &self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, "FS")?;
        if session.selection.len() != 1 {
            return Err(CadError::InvalidInput(format!(
                "fs.selection: FS needs exactly one selected seed object, got {}",
                session.selection.len()
            )));
        }
        let seed_ref = session.selection.refs()[0].clone();
        if !seed_ref.instance.0.is_empty() || seed_ref.sub_element.is_some() {
            return Err(CadError::InvalidInput(
                "fs.not_addressable: the FS seed must be a directly addressable entity, not a \
                 block-instance member or sub-element"
                    .into(),
            ));
        }
        let drawing = document_drawing(self, command)?;
        let seed = drawing.entity(seed_ref.entity).ok_or_else(|| {
            CadError::InvalidInput(
                "fs.seed_missing: the selected seed entity is not present in the drawing".into(),
            )
        })?;
        if seed.space != SpaceId::Model {
            return Err(CadError::InvalidInput(
                "fs.seed_not_model_space: the FS seed must be a model-space entity".into(),
            ));
        }
        if !fs_supported(&seed.geometry) {
            return Err(CadError::Unsupported(format!(
                "fs.seed_geometry: FS does not intersect {:?} geometry",
                std::mem::discriminant(&seed.geometry)
            )));
        }

        let engine = DefaultGeometryEngine;
        let policy = TolerancePolicy::default();
        let seed_endpoints = curve_endpoints(&engine, &seed.geometry, &policy)?;
        let root = drawing.root_layer_scope();
        let mut touching = Vec::new();
        for candidate in drawing.model_space() {
            if candidate.id == seed.id || !fs_supported(&candidate.geometry) {
                continue;
            }
            let reference = model_ref(command.document, candidate.id);
            if !ref_visibility(
                &drawing,
                &session.layer_overrides,
                &session.hidden_entities,
                root,
                &reference,
            )
            .unwrap_or(false)
            {
                continue;
            }
            let mut hit = match engine.intersect_local(&seed.geometry, &candidate.geometry, &policy)
            {
                Ok(points) => !points.is_empty(),
                Err(error) => {
                    return Err(CadError::Unsupported(format!(
                        "fs.intersect: entity {} could not be intersected: {error}",
                        candidate.id.0
                    )))
                }
            };
            if !hit && !seed_endpoints.is_empty() {
                let candidate_endpoints = curve_endpoints(&engine, &candidate.geometry, &policy)?;
                hit = endpoints_touch(&seed_endpoints, &candidate_endpoints, &policy);
            }
            if hit {
                touching.push(reference);
            }
        }

        let mut added = 0usize;
        for reference in touching {
            if session.selection.insert(reference) {
                added += 1;
            }
        }
        let total = session.selection.len();
        let (code, message) = if added == 0 {
            (
                "fs.none",
                format!(
                    "FS: nothing touches the selected entity {}; the selection is unchanged",
                    seed.id.0
                ),
            )
        } else {
            (
                "fs.applied",
                format!("FS: added {added} touching object(s) to the selection (total {total})"),
            )
        };
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: code.into(),
                message,
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// The geometry kinds FS intersects. Other kinds (text, hatch, dimensions,
/// blocks, images, opaque data, meshes, compounds) are not curve-touch candidates
/// and are skipped, never silently truncated inside a supported kind.
fn fs_supported(geometry: &SemanticGeometry) -> bool {
    matches!(
        geometry,
        SemanticGeometry::Line { .. }
            | SemanticGeometry::Circle { .. }
            | SemanticGeometry::Arc { .. }
            | SemanticGeometry::Ellipse { .. }
            | SemanticGeometry::Polyline { .. }
            | SemanticGeometry::Spline { .. }
    )
}

fn curve_endpoints(
    engine: &DefaultGeometryEngine,
    geometry: &SemanticGeometry,
    policy: &TolerancePolicy,
) -> CadResult<Vec<Point3>> {
    match geometry {
        SemanticGeometry::Line { start, end } => Ok(vec![*start, *end]),
        SemanticGeometry::Arc { .. }
        | SemanticGeometry::Polyline { .. }
        | SemanticGeometry::Spline { .. } => {
            let points = engine.tessellate_curve(geometry, policy)?;
            let mut endpoints = Vec::new();
            if let Some(first) = points.first() {
                endpoints.push(*first);
            }
            if let Some(last) = points.last() {
                if endpoints.first() != Some(last) {
                    endpoints.push(*last);
                }
            }
            Ok(endpoints)
        }
        _ => Ok(Vec::new()),
    }
}

fn endpoints_touch(a: &[Point3], b: &[Point3], policy: &TolerancePolicy) -> bool {
    let tolerance = policy.computation_world.max(1e-9);
    a.iter()
        .any(|p| b.iter().any(|q| distance(*p, *q) <= tolerance))
}
