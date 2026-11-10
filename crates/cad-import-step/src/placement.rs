//! Occurrence-tree placement resolution for STEP bodies.
//!
//! The tessellator returns every face mesh in **local body coordinates**. A STEP
//! assembly records where each body's product definition is used in
//! `NEXT_ASSEMBLY_USAGE_OCCURRENCE` records; `cadmpeg-ir` resolves those into a
//! canonical occurrence tree (`Model::occurrences`), each entry carrying the
//! placement relative to its parent. Editing that tree was verified against the
//! real `cadmpeg-ir` 0.6.11 source, not guessed:
//!
//! * `Occurrence::prototype` names the local `ProductDefinition` it instances;
//! * `ProductDefinition::bodies` lists the shape bodies owned by that definition;
//! * `Occurrence::transform` is the placement relative to the direct container
//!   (`OccurrenceParent`), and `Occurrence::scale` is a per-axis instance scale;
//! * `Body::transform` is a **world** placement produced by standalone
//!   `MAPPED_ITEM` records, so it is applied after the occurrence placement.
//!
//! A body that no occurrence and no product definition owns is already in the
//! model frame: its IR coordinates are used unchanged (identity placement).
//! That is why a single-representation file with several bodies does not need an
//! assembly tree, and why a file with a tree is never stacked at the origin.

use std::collections::BTreeMap;

use cadmpeg_ir::document::CadIr;
use cadmpeg_ir::products::{Occurrence, OccurrenceParent, PrototypeReference};

/// Row-major affine placement (`3` rows of `[m00 m01 m02 tx]`), finite.
pub(crate) type Affine = [[f64; 4]; 3];

/// The resolved placement of every body in one document.
pub(crate) struct PlacementPlan {
    /// World placements per body identity, in occurrence order. A body with no
    /// owned occurrence is placed by identity, so every body has at least one
    /// entry here.
    pub(crate) placements: BTreeMap<String, Vec<Affine>>,
    /// Bodies owned by a product definition that no occurrence instantiates.
    /// The adapter still imports them at identity, but reports the ambiguity.
    pub(crate) uninstantiated: Vec<String>,
}

/// Resolve every body's world placement from the occurrence tree.
///
/// A cycle, a missing parent, duplicate occurrence identity, or a placement that
/// overflows the finite range is refused: the caller turns that into the stable
/// `import.step.assembly_placement_unsupported` refusal instead of shipping
/// geometry at a guessed location.
pub(crate) fn resolve_body_placements(ir: &CadIr) -> Result<PlacementPlan, String> {
    let mut occurrences = BTreeMap::<&str, &Occurrence>::new();
    for occurrence in &ir.model.occurrences {
        if occurrences
            .insert(occurrence.id.as_str(), occurrence)
            .is_some()
        {
            return Err(format!(
                "duplicate occurrence identity {}",
                occurrence.id.as_str()
            ));
        }
    }

    let mut memo = BTreeMap::<String, Affine>::new();
    let mut visiting = Vec::<String>::new();
    for occurrence in &ir.model.occurrences {
        world_transform(
            occurrence.id.as_str(),
            &occurrences,
            &mut memo,
            &mut visiting,
        )?;
    }

    let mut placements = BTreeMap::<String, Vec<Affine>>::new();
    let mut uninstantiated = Vec::new();
    for definition in &ir.model.product_definitions {
        let mut owners = Vec::<&Occurrence>::new();
        for occurrence in &ir.model.occurrences {
            if let PrototypeReference::Local {
                definition: prototype,
            } = &occurrence.prototype
            {
                if prototype == &definition.id {
                    owners.push(occurrence);
                }
            }
        }
        for body in &definition.bodies {
            let key = body.as_str().to_string();
            if owners.is_empty() {
                uninstantiated.push(key.clone());
                placements.entry(key).or_insert_with(|| vec![identity()]);
                continue;
            }
            let entries = placements.entry(key).or_default();
            for owner in &owners {
                let world = memo
                    .get(owner.id.as_str())
                    .copied()
                    .ok_or_else(|| format!("occurrence {} was not resolved", owner.id.as_str()))?;
                entries.push(world);
            }
        }
    }

    for body in &ir.model.bodies {
        let key = body.id.as_str().to_string();
        let mut entries = placements.remove(&key).unwrap_or_default();
        if entries.is_empty() {
            entries.push(identity());
        } else if let Some(world) = body.transform {
            // `Body::transform` is a **world** placement produced by a
            // standalone MAPPED_ITEM. With no occurrence (or an identity root
            // occurrence) it is the only placement. Combining it with a real
            // occurrence transform would place the body twice, which the
            // decoder gives no way to undo honestly, so refuse instead.
            let body_rows = world.affine_rows();
            if entries.iter().any(|entry| *entry != identity()) {
                return Err(format!(
                    "body {key} carries both an occurrence placement and a standalone \
                     MAPPED_ITEM world transform; composing them would place it twice"
                ));
            }
            for entry in &mut entries {
                *entry = compose(*entry, body_rows)?;
            }
        }
        placements.insert(key, entries);
    }

    uninstantiated.sort();
    uninstantiated.dedup();
    Ok(PlacementPlan {
        placements,
        uninstantiated,
    })
}

/// Placements for one body, or a single identity placement.
pub(crate) fn placements_of(plan: &PlacementPlan, body: &str) -> Vec<Affine> {
    plan.placements
        .get(body)
        .cloned()
        .unwrap_or_else(|| vec![identity()])
}

/// The identity placement.
pub(crate) const fn identity() -> Affine {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
    ]
}

/// Apply an affine placement to a point.
pub(crate) fn transform_point(matrix: &Affine, point: [f64; 3]) -> Option<[f64; 3]> {
    let [x, y, z] = point;
    let out = [
        matrix[0][0] * x + matrix[0][1] * y + matrix[0][2] * z + matrix[0][3],
        matrix[1][0] * x + matrix[1][1] * y + matrix[1][2] * z + matrix[1][3],
        matrix[2][0] * x + matrix[2][1] * y + matrix[2][2] * z + matrix[2][3],
    ];
    out.iter().all(|value| value.is_finite()).then_some(out)
}

/// The inverse-transpose linear part used for normals, or `None` when singular.
pub(crate) fn normal_matrix(matrix: &Affine) -> Option<[[f64; 3]; 3]> {
    let a = [
        [matrix[0][0], matrix[0][1], matrix[0][2]],
        [matrix[1][0], matrix[1][1], matrix[1][2]],
        [matrix[2][0], matrix[2][1], matrix[2][2]],
    ];
    let det = a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1])
        - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
        + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0]);
    if !det.is_finite() || det.abs() <= f64::EPSILON {
        return None;
    }
    let inv_det = 1.0 / det;
    // Cofactor matrix divided by the determinant is (M^-1)^T.
    let cofactor = [
        [
            (a[1][1] * a[2][2] - a[1][2] * a[2][1]) * inv_det,
            (a[1][2] * a[2][0] - a[1][0] * a[2][2]) * inv_det,
            (a[1][0] * a[2][1] - a[1][1] * a[2][0]) * inv_det,
        ],
        [
            (a[0][2] * a[2][1] - a[0][1] * a[2][2]) * inv_det,
            (a[0][0] * a[2][2] - a[0][2] * a[2][0]) * inv_det,
            (a[0][1] * a[2][0] - a[0][0] * a[2][1]) * inv_det,
        ],
        [
            (a[0][1] * a[1][2] - a[0][2] * a[1][1]) * inv_det,
            (a[0][2] * a[1][0] - a[0][0] * a[1][2]) * inv_det,
            (a[0][0] * a[1][1] - a[0][1] * a[1][0]) * inv_det,
        ],
    ];
    let finite = cofactor.iter().flatten().all(|value| value.is_finite());
    finite.then_some(cofactor)
}

/// Multiply two affine placements (`left * right`, column vectors).
fn compose(left: Affine, right: Affine) -> Result<Affine, String> {
    let mut out = [[0.0; 4]; 3];
    for row in 0..3 {
        for column in 0..4 {
            let value = left[row][0] * right[0][column]
                + left[row][1] * right[1][column]
                + left[row][2] * right[2][column];
            let value = if column == 3 {
                value + left[row][3]
            } else {
                value
            };
            if !value.is_finite() {
                return Err("an occurrence placement overflows the finite range".into());
            }
            out[row][column] = value;
        }
    }
    Ok(out)
}

fn world_transform(
    id: &str,
    occurrences: &BTreeMap<&str, &Occurrence>,
    memo: &mut BTreeMap<String, Affine>,
    visiting: &mut Vec<String>,
) -> Result<Affine, String> {
    if let Some(rows) = memo.get(id) {
        return Ok(*rows);
    }
    if visiting.iter().any(|active| active == id) {
        return Err(format!("occurrence {id} closes a placement cycle"));
    }
    let occurrence = occurrences
        .get(id)
        .ok_or_else(|| format!("occurrence {id} is missing"))?;
    visiting.push(id.to_string());
    let local = local_matrix(occurrence)?;
    let world = match &occurrence.parent {
        OccurrenceParent::Root {} => local,
        OccurrenceParent::Occurrence { occurrence: parent } => {
            let parent = world_transform(parent.as_str(), occurrences, memo, visiting)?;
            compose(parent, local)?
        }
    };
    visiting.pop();
    memo.insert(id.to_string(), world);
    Ok(world)
}

fn local_matrix(occurrence: &Occurrence) -> Result<Affine, String> {
    let transform = occurrence
        .effective_transform()
        .map_err(|error| format!("occurrence {}: {error}", occurrence.id.as_str()))?;
    let mut rows = transform.affine_rows();
    let scale = occurrence.scale.map(|value| value.get());
    if scale.iter().all(|value| *value == 1.0) {
        return Ok(rows);
    }
    if scale
        .iter()
        .any(|value| !value.is_finite() || *value == 0.0)
    {
        return Err(format!(
            "occurrence {} has an unusable scale",
            occurrence.id.as_str()
        ));
    }
    for row in &mut rows {
        row[0] *= scale[0];
        row[1] *= scale[1];
        row[2] *= scale[2];
    }
    if rows.iter().flatten().all(|value| value.is_finite()) {
        Ok(rows)
    } else {
        Err(format!(
            "occurrence {} scale overflows the finite range",
            occurrence.id.as_str()
        ))
    }
}
