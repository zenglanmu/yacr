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
//! * `STATUS` prints the drawing's stored `$INSBASE`/`$LIMMIN`/`$LIMMAX` values
//!   (`status.limits_base`) and names an absent one instead of defaulting it; it
//!   does **not** track drawing creation/edit time or memory usage and says so
//!   with explicit `status.not_tracked` diagnostics;
//! * `FS` is an Express tool (command-line only, no ribbon button). `FSMODE=OFF`
//!   (AutoCAD's initial value) adds only **directly touching** entities — a
//!   crossing and a tangent contact both count — never a transitive chain;
//!   `FSMODE=ON` follows the transitive **tangent** chain and refuses by name what
//!   it cannot chain (`fs.chain_nontangent`, `fs.chain_ambiguous`,
//!   `fs.chain_bound`; see [`super::fs_chain`]). It enumerates directly addressable
//!   model-space entities only — block instance members are not expanded, and
//!   hidden layers are skipped.

use cad_domain::{MLeaderContentKind, TolerancePolicy};
use cad_geometry::{distance, DefaultGeometryEngine};

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
            ..
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
            let content = match leader.content().kind() {
                MLeaderContentKind::None => "none",
                MLeaderContentKind::Text => "text",
                MLeaderContentKind::Block => "block",
                MLeaderContentKind::Tolerance => "tolerance",
            };
            format!(
                "multileader style={} content={content} roots={} lines={}",
                leader.style_name(),
                leader.roots().len(),
                leader.lines().len()
            )
        }
        SemanticGeometry::Table(table) => format!(
            "table style={} name={} rows={} columns={} display_items={}",
            if table.style_name().is_empty() {
                "-"
            } else {
                table.style_name()
            },
            if table.name().is_empty() {
                "-"
            } else {
                table.name()
            },
            table.rows().len(),
            table.columns().len(),
            table.display().len()
        ),
        // A body reports its stored topology class, whether the exact B-rep is
        // present, and the display cache's own counts; no volume, area or
        // bounding box is derived from the tessellation.
        SemanticGeometry::Body3d(body) => {
            let exact = match &body.exact {
                BodyExact::Exact(_) => "brep".to_string(),
                BodyExact::Unavailable { reason } => format!("unavailable({reason})"),
            };
            format!(
                "body kind={:?} exact={exact} mesh_vertices={} mesh_triangles={} edges={}",
                body.kind,
                body.display.mesh.vertices.len(),
                body.display.mesh.triangle_count(),
                body.display.edges.len()
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

        let mut diagnostics = vec![
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
            // The two vector header values are real now (`$INSBASE`,
            // `$LIMMIN`/`$LIMMAX`, written by `BASE`/`LIMITS`), so the report
            // prints the stored values and names only the ones this database does
            // not hold — never an origin or a zero rectangle as if it were
            // stored.
            match (drawing.insertion_base(), drawing.model_limits()) {
                (None, None) => Diagnostic {
                    object: None,
                    code: "status.not_stored".into(),
                    message: "STATUS drawing limits and base point: this drawing stores no \
                              $INSBASE, $LIMMIN or $LIMMAX value (the importer of this build does \
                              not read them); set them with `BASE`/`LIMITS`. Creation/edit time \
                              and memory usage stay not tracked."
                        .into(),
                },
                (base, limits) => Diagnostic {
                    object: None,
                    code: "status.limits_base".into(),
                    message: format!(
                        "STATUS drawing base point={} limits={}",
                        match base {
                            Some(base) => format!("({}, {}, {})", base.x, base.y, base.z),
                            None => "not stored".to_string(),
                        },
                        match limits {
                            Some((min, max)) =>
                                format!("({}, {}) to ({}, {})", min[0], min[1], max[0], max[1]),
                            None => "not stored".to_string(),
                        }
                    ),
                },
            },
            Diagnostic {
                object: None,
                code: "status.not_tracked".into(),
                message: "STATUS memory usage: not tracked by this build".into(),
            },
        ];
        // The extra measured statistics (current space, per-layout counts, tracked
        // modes, session counts) live beside the other read-only reports.
        diagnostics.extend(crate::app_reports_ext::status_extended(
            self, session, command, &drawing,
        ));
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }

    /// `FS`: add every visible model-space entity that touches the selected seed.
    ///
    /// `FSMODE=OFF` (the default) adds the **direct** contacts only; `FSMODE=ON`
    /// (`session.drafting_extras`) follows the transitive tangent chain. Both passes
    /// live in [`super::fs_chain`], which also owns the named refusals. Read-only:
    /// the pass mutates the session selection and nothing else, so there is no
    /// transaction, no revision bump and no undo entry.
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
        if !super::fs_chain::fs_supported(&seed.geometry) {
            return Err(CadError::Unsupported(format!(
                "fs.seed_geometry: FS does not intersect {:?} geometry",
                std::mem::discriminant(&seed.geometry)
            )));
        }

        let engine = DefaultGeometryEngine;
        let policy = TolerancePolicy::default();
        let root = drawing.root_layer_scope();
        let chain_mode = session.drafting_extras.fs_mode();
        let pass = if chain_mode {
            super::fs_chain::tangent_chain(
                &drawing,
                session,
                command.document,
                root,
                seed,
                &engine,
                &policy,
            )?
        } else {
            super::fs_chain::direct_touching(
                &drawing,
                session,
                command.document,
                root,
                seed,
                &engine,
                &policy,
            )?
        };

        let mut added = 0usize;
        for reference in pass.refs {
            if session.selection.insert(reference) {
                added += 1;
            }
        }
        let total = session.selection.len();
        let mode = if chain_mode { "ON" } else { "OFF" };
        let (code, message) = if added == 0 {
            (
                "fs.none",
                format!(
                    "FS (FSMODE={mode}): nothing touches the selected entity {}; the selection \
                     is unchanged",
                    seed.id.0
                ),
            )
        } else {
            (
                "fs.applied",
                format!("FS: added {added} touching object(s) to the selection (total {total})"),
            )
        };
        let mut diagnostics = vec![Diagnostic {
            object: None,
            code: code.into(),
            message,
        }];
        if added > 0 {
            let detail = if chain_mode {
                format!(
                    "FSMODE=ON: tangent-chain pass over {} visible model-space curve(s) including \
                     the seed, {} tangent contact(s); FS adds to the session selection only",
                    pass.entities, pass.contacts
                )
            } else {
                format!(
                    "FSMODE=OFF: direct-contact pass; {} touching object(s) beside the seed ({} \
                     visible model-space curve(s) including it); no transitive chain — set \
                     FSMODE=ON for the tangent chain",
                    pass.contacts, pass.entities
                )
            };
            diagnostics.push(Diagnostic {
                object: None,
                code: "fs.mode".into(),
                message: detail,
            });
        }
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }
}
