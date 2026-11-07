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
//!   and does not abort the whole report. It reports the entity's space and its
//!   canonical colour/linetype/lineweight/transparency (ByLayer/ByBlock or the
//!   explicit value); thickness and extrusion are **not modelled** by this build
//!   and the report says so;
//! * `STATUS` does not track drawing creation/edit time, memory usage, drawing
//!   limits or the base point (this build has no drawing-header model), and says
//!   so with explicit `status.not_tracked` diagnostics;
//! * `FS` is an Express tool (command-line only, no ribbon button) and implements
//!   AutoCAD's default `FSMODE=OFF`: it adds only **directly touching** entities,
//!   never a transitive chain. Chain mode (`FSMODE=ON`) is **NOT IMPLEMENTED**.
//!   It enumerates directly addressable model-space entities only — block
//!   instance members are not expanded, and hidden layers are skipped.

use cad_geometry::distance;

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
/// The four values are the same round-trippable strings `SETENTITYPROPERTY`
/// accepts (`ByLayer`/`ByBlock` stay symbolic; an explicit value is shown
/// verbatim). Thickness and extrusion have no stored model in this build, so the
/// report says `not-modelled` instead of inventing them.
fn display_summary(database: &cad_db::DrawingDatabase, entity: &cad_db::DbEntity) -> String {
    let value = |key: &str| {
        crate::property_value::property_value(database, entity, key)
            .unwrap_or_else(|| "unresolved".to_string())
    };
    format!(
        "color={} linetype={} lineweight={} transparency={} thickness=not-modelled \
         extrusion=not-modelled",
        value("color"),
        value("linetype"),
        value("lineweight"),
        value("transparency")
    )
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
                    "LIST id={} handle={} type={} layer={} space={} draw_order={} {} {}",
                    entity.id.0,
                    handle_label(entity),
                    entity.object.type_key,
                    layer_label(&drawing, entity.layer),
                    space_label(&entity.space),
                    entity.draw_order,
                    display_summary(&drawing, entity),
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

}
