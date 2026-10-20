//! MULTILEADER export: a real DXF `MULTILEADER` entity, one `MLEADERSTYLE`
//! object per distinct style name and the two `CLASSES` entries (ADR 0019
//! decision E).
//!
//! The entity is written from the domain definition (`style + text + roots +
//! lines + reserved`), **never** from the derived display: the display is what
//! this build renders, the `MULTILEADER` record is what AutoCAD recomputes.
//! The `90` property-override mask is written **verbatim** (our scaling refusal,
//! decision C.5, is what keeps mask and geometry consistent), and no computed
//! text layout is written.
//!
//! The honest deviations that make this a `Converted` mapping, never `Exact`:
//!
//! * `91`/`341`/`171` (leader colour / linetype / lineweight) are written from
//!   defaults because the import resolves them but does not apply them.
//! * The `MLEADERSTYLE` record carries this build's **resolved effective**
//!   snapshot (P7b-1 deviation 1), not the source record, so a field the `90`
//!   mask suppresses is not a verbatim round-trip — its value re-resolves from
//!   that snapshot on load.
//! * The parsed source groups with no model field — entity-tail text frame
//!   (`292`), annotation scale (`293`), text direction negative (`294`), text
//!   angle type (`174`), text align in IPE (`178`), extend leader to text
//!   (`295`) and context text boundary height (`44`), text height automatic
//!   (`293`) and word break (`295`) — are **normalised to acadrust defaults by
//!   design** and are not round-tripped. The report names them unconditionally
//!   because the writer never reads them and so cannot tell whether a source
//!   file set them.
//!
//! These named deviations are **exhaustive for what the writer knows about**,
//! not a claim that the file loses nothing: the writer can only report what the
//! domain model retains. The entity-level custom arrowhead block handle
//! (entity-tail `342`) is exactly such a case — the domain model never reads
//! it, so the writer cannot see it; the importer names that loss in the
//! migrated entity's `Completeness` instead. (Preserving `342` in `reserved` is
//! a declared follow-up, not a current capability.)
//!
//! Further reported deviations: MULTILEADER seed values (`MLEADERSTYLE` default
//! text `300`, the block-content transform matrix `47`) are not modelled and
//! are written empty/identity; a raw per-line arrowhead/linetype handle is
//! written verbatim even though the handle is not remapped to an exported
//! object; and `CONTEXT_DATA`'s text colour, flow direction, alignment and line
//! spacing have no model field, so they are written from defaults. acadrust's
//! `MLEADERSTYLE` writer also emits no style-level attachment
//! direction/top/bottom (`271`–`273`), which this build never modelled anyway.
//! The text normal and direction are derived from the plane frame and the
//! rotation because the model stores neither.
//!
//! "Never write computed text layout" (decision E) holds by construction: only
//! the string, its placement, the width boundary and the height are written.
//! The legacy leader-line break mirror fields (`segment_index`,
//! `break_info_count`) are re-derived by the writer from the break records, so
//! they are not written as independent source values.

use acadrust::classes::{DxfClass, ProxyFlags};
use acadrust::entities::multileader::LeaderLineBreakInfo;
use acadrust::entities::{
    LeaderLine as AcadLeaderLine, LeaderRoot as AcadLeaderRoot, MultiLeader as AcadMultiLeader,
    MultiLeaderAnnotContext, MultiLeaderPathType as AcadPathType,
    MultiLeaderPropertyOverrideFlags as AcadMask, StartEndPointPair, TextAttachmentDirectionType,
    TextAttachmentPointType, TextAttachmentType,
};
use acadrust::objects::{
    LeaderContentType as StyleContentType, MultiLeaderPathType as StylePathType, MultiLeaderStyle,
    ObjectType, TextAlignmentType as StyleTextAlignment, TextAngleType as StyleTextAngle,
    TextAttachmentType as StyleTextAttachment,
};
use acadrust::types::{Color, Handle, LineWeight, Vector3};
use acadrust::CadDocument;
use cad_db::DbEntity;
use cad_domain::*;

use super::entity::{common_for, v3, EntityContext, ExportAccumulator};
use super::report::MappingOutcome;
use super::tables::ensure_text_style;

/// CLASSES group 3 for the entity class (U-5: unverified upstream, pinned by
/// the tests).
const MULTILEADER_APPLICATION: &str = "ACDB_MLEADER_CLASS";
/// CLASSES group 3 for the style class (U-5).
const MLEADERSTYLE_APPLICATION: &str = "ACDB_MLEADERSTYLE_CLASS";
/// CLASSES group 90 written when acadrust has no pre-registered entry.
///
/// acadrust 0.6.5's `CadDocument::new()` already registers both classes, so the
/// normal path never adds one. These are that table's own values (ADR 0019 U-5:
/// unverified upstream; the test pins them).
const MULTILEADER_PROXY_FLAGS: u16 = 1025; // ERASE_ALLOWED | DISABLES_PROXY_WARNING_DIALOG
const MLEADERSTYLE_PROXY_FLAGS: u16 = 4095; // every proxy capability bit

/// The right attachment written when the model carries none (it has no field).
const UNMODELLED_RIGHT_ATTACHMENT: TextAttachmentType = TextAttachmentType::MiddleOfText;
/// The style record's mirror of [`UNMODELLED_RIGHT_ATTACHMENT`].
const UNMODELLED_STYLE_RIGHT_ATTACHMENT: StyleTextAttachment = StyleTextAttachment::MiddleOfText;
/// The flow direction written when the model carries none (it has no field).
const UNMODELLED_FLOW_DIRECTION: acadrust::entities::FlowDirectionType =
    acadrust::entities::FlowDirectionType::Horizontal;

/// Named deviation: unapplied entity-level leader colour/linetype/lineweight.
const REASON_LINE_DEFAULTS: &str =
    "entity-level leader colour, linetype and lineweight (entity-tail 91/341/171) are written \
     as defaults; the import resolves but does not apply them";
/// Named deviation: the override mask is source data, not a derived value.
const REASON_MASK_VERBATIM: &str =
    "the 90 property-override mask is written verbatim and is never re-derived";
/// Named deviation: P7b-1 stores effective style values, not the source record.
const REASON_EFFECTIVE_STYLE: &str =
    "the MLEADERSTYLE record holds the resolved effective values snapshot, not the source \
     record; fields the 90 mask suppresses are not a verbatim round-trip (P7b-1 deviation)";
/// Named deviation: fields with no model home are written as their defaults.
const REASON_SEEDS: &str =
    "MULTILEADER seed values (MLEADERSTYLE default text 300, block-content transform matrix 47) \
     are not modelled and are written empty/identity";
/// Named deviation: `CONTEXT_DATA` properties the model does not carry.
const REASON_CONTEXT_DEFAULTS: &str =
    "CONTEXT_DATA text colour, flow direction, alignment, line spacing and the text \
     normal/direction are not modelled; they are written from defaults or derived values";
/// Standing named deviation: parsed source groups the model never reads.
const REASON_UNMODELLED_GROUPS: &str =
    "these parsed MULTILEADER groups have no model field, so they are written as acadrust \
     defaults and are not round-tripped: entity-tail text frame (292), annotation scale (293), \
     text direction negative (294), text angle type (174), text align in IPE (178) and extend \
     leader to text (295), plus context text boundary height (44), text height automatic (293) \
     and word break (295)";

/// Write one MULTILEADER entity plus the style record and class entries it
/// needs (ADR 0019 decision E).
///
/// Identity or rigid insert placement writes a real `MULTILEADER`; a
/// scaling/mirroring insert placement cannot be baked (decision C.5), so the
/// entity is **flattened to its display primitives** through the same path the
/// `Compound` arm uses — never dropped, and never silently. Each child records
/// its own outcome.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_multileader(
    doc: &mut CadDocument,
    entity: &DbEntity,
    geometry: &MultiLeaderGeometry,
    transform: &Transform3,
    depth: usize,
    ctx: &EntityContext,
    acc: &mut ExportAccumulator,
    stack: &mut Vec<BlockId>,
) -> CadResult<()> {
    let object = Some(entity.object.id);
    let placed;
    let geometry = if transform == &Transform3::identity() {
        geometry
    } else {
        match geometry.transformed(transform) {
            Ok(transformed) => {
                placed = transformed;
                &placed
            }
            Err(_) => {
                // Same fallback the pre-P7b `Compound` path gave these entities:
                // the annotations survive as primitives with the insert
                // transform composed in, and the report says the MULTILEADER
                // identity is gone on round trip.
                super::entity::flatten_children(
                    doc,
                    entity,
                    geometry.display(),
                    transform,
                    depth,
                    ctx,
                    acc,
                    stack,
                )?;
                acc.record(
                    object,
                    "multileader_flattened",
                    MappingOutcome::Converted(vec![
                        "a MULTILEADER cannot be written under a scaling/mirroring insert \
                         transform; flattened to its display primitives and no longer a \
                         MULTILEADER on round trip"
                            .into(),
                    ]),
                );
                return Ok(());
            }
        }
    };

    ensure_classes(doc);

    let style = geometry.style();
    let text = geometry.text();
    let mut reasons = vec![
        REASON_LINE_DEFAULTS.to_string(),
        REASON_MASK_VERBATIM.to_string(),
        REASON_EFFECTIVE_STYLE.to_string(),
        REASON_SEEDS.to_string(),
        REASON_CONTEXT_DEFAULTS.to_string(),
        REASON_UNMODELLED_GROUPS.to_string(),
    ];
    let (style_name, name_reasons) = resolve_style_name(geometry);
    reasons.extend(name_reasons);
    let (text_style_name, text_reasons) =
        ensure_text_style(doc, &style.text_style_name, style.text_font.as_deref());
    reasons.extend(text_reasons);
    let text_style_handle = ensure_style_handle(doc, &text_style_name);
    let (style_handle, style_reasons) =
        ensure_style_record(doc, acc, &style_name, geometry, text_style_handle);
    reasons.extend(style_reasons);

    let (common, common_reasons) = common_for(doc, entity, ctx);
    reasons.extend(common_reasons);

    let mut ml = AcadMultiLeader::new();
    ml.common = common;
    ml.style_handle = Some(style_handle);
    ml.content_type = content_type(geometry.content());
    ml.property_override_flags = AcadMask::from_bits_retain(geometry.override_mask());
    ml.path_type = acad_path_type(style.path_type);
    ml.enable_landing = style.enable_landing;
    ml.enable_dogleg = style.enable_dogleg;
    ml.dogleg_length = style.landing_distance;
    ml.arrowhead_size = style.arrowhead_size;
    ml.text_style_handle = Some(text_style_handle);
    ml.text_height = style.text_height;
    ml.text_left_attachment = TextAttachmentType::from(text.left_attachment);
    ml.text_attachment_point = TextAttachmentPointType::from(text.attachment_point);
    ml.text_attachment_direction = reserved_root(geometry, 0)
        .map(|extras| attachment_direction(extras.text_attachment_direction))
        .unwrap_or(TextAttachmentDirectionType::Horizontal);
    ml.scale_factor = style.scale_factor;

    let mut context = build_context(geometry, style, text, text_style_handle);
    context.leader_roots = build_roots(geometry);
    ml.context = context;

    if geometry.content() == MLeaderContent::Text && text.string.is_empty() {
        reasons.push(
            "empty text content is written as MText content; a reload treats an empty string as \
             no content"
                .into(),
        );
    }
    if geometry.reserved().background.is_some() {
        reasons.push(
            "multileader background-fill parameters are written verbatim from the preserved \
             source data but are not rendered"
                .into(),
        );
    }
    if geometry.reserved().columns.is_some() {
        reasons.push(
            "multileader column-layout parameters are written verbatim from the preserved \
             source data but are not rendered"
                .into(),
        );
    }
    if has_breaks(geometry) {
        reasons.push(
            "multileader leader-line break data is written verbatim from the preserved source \
             data but is not rendered"
                .into(),
        );
    }
    if let Some(handle) = geometry
        .lines()
        .iter()
        .find_map(|line| line.arrowhead_handle.filter(|handle| *handle != 0))
    {
        reasons.push(format!(
            "per-line arrowhead block handle {handle:X} is written verbatim; the referenced \
             block is not exported, so the handle dangles"
        ));
    }
    if geometry
        .reserved()
        .lines
        .iter()
        .any(|line| line.line_type_handle.is_some_and(|handle| handle != 0))
    {
        reasons.push(
            "per-line linetype handles are written verbatim from the source and are not remapped \
             to an exported LINETYPE record"
                .into(),
        );
    }

    acc.push(acadrust::EntityType::MultiLeader(Box::new(ml)), ctx)?;
    acc.record(object, "multileader", MappingOutcome::Converted(reasons));
    Ok(())
}

/// Insert the two `CLASSES` entries (ADR 0019 E / U-5).
///
/// Group 1 is `MULTILEADER` for the entity class — a real file does **not** use
/// `MLEADER` here — and `MLEADERSTYLE` for the object class. `ProxyFlags`,
/// `instance_count` and `was_zombie` are unverified upstream.
///
/// acadrust 0.6.5's `CadDocument::new()` already registers both classes with
/// these exact names; the insert below is the defensive path for a document
/// whose class table lacks them (ADR 0019 §0.8 assumed an empty table, which is
/// not true for this version). Either way the written values are pinned by the
/// tests, not chosen silently.
fn ensure_classes(doc: &mut CadDocument) {
    if doc.classes.get_by_name("MULTILEADER").is_none() {
        let mut entity_class = DxfClass::new_entity("MULTILEADER", "AcDbMLeader");
        entity_class.application_name = MULTILEADER_APPLICATION.to_string();
        entity_class.proxy_flags = ProxyFlags(MULTILEADER_PROXY_FLAGS);
        entity_class.instance_count = 0;
        entity_class.was_zombie = false;
        doc.classes.add_or_update(entity_class);
    }
    if doc.classes.get_by_name("MLEADERSTYLE").is_none() {
        let mut style_class = DxfClass::new("MLEADERSTYLE", "AcDbMLeaderStyle");
        style_class.application_name = MLEADERSTYLE_APPLICATION.to_string();
        style_class.proxy_flags = ProxyFlags(MLEADERSTYLE_PROXY_FLAGS);
        style_class.instance_count = 0;
        style_class.was_zombie = false;
        style_class.is_an_entity = false;
        doc.classes.add_or_update(style_class);
    }
}

/// The style name to write, normalising an empty one to `Standard`.
fn resolve_style_name(geometry: &MultiLeaderGeometry) -> (String, Vec<String>) {
    let name = geometry.style_name().trim();
    if name.is_empty() {
        (
            "Standard".to_string(),
            vec!["multileader has no style name; written with 'Standard'".to_string()],
        )
    } else {
        (name.to_string(), Vec::new())
    }
}

/// The handle of a text style entry, allocating one when the table entry has
/// none (the DXF writer only fills null table-entry handles on its own copy, so
/// a `340`/`342`/`343` reference needs a stable value here).
fn ensure_style_handle(doc: &mut CadDocument, name: &str) -> Handle {
    let needs_handle = doc
        .text_styles
        .get(name)
        .map(|entry| entry.handle.is_null())
        .unwrap_or(false);
    if needs_handle {
        let handle = doc.allocate_handle();
        if let Some(entry) = doc.text_styles.get_mut(name) {
            entry.handle = handle;
        }
    }
    doc.text_styles
        .get(name)
        .map(|entry| entry.handle)
        .unwrap_or(Handle::NULL)
}

/// The ACAD_MLEADERSTYLE dictionary handle.
fn style_dictionary(doc: &CadDocument) -> Option<Handle> {
    let root = doc.header.named_objects_dict_handle;
    match doc.objects.get(&root) {
        Some(ObjectType::Dictionary(dictionary)) => dictionary
            .entries
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("ACAD_MLEADERSTYLE"))
            .map(|(_, handle)| *handle),
        _ => None,
    }
}

/// The handle of the style object already carrying `name`, if any.
fn existing_style_handle(doc: &CadDocument, name: &str) -> Option<Handle> {
    doc.objects
        .iter()
        .find_map(|(handle, object)| match object {
            ObjectType::MultiLeaderStyle(style) if style.name.eq_ignore_ascii_case(name) => {
                Some(*handle)
            }
            _ => None,
        })
}

/// Write (or update) the `MLEADERSTYLE` record for `style_name` and return its
/// handle plus the deviations this entity contributes.
///
/// One record per distinct name (ADR 0019 D). When two entities share a name
/// with different snapshots the **first** one wins and the second is reported:
/// with the mask verbatim, its masked-out fields re-resolve to the first
/// snapshot on load.
fn ensure_style_record(
    doc: &mut CadDocument,
    acc: &mut ExportAccumulator,
    style_name: &str,
    geometry: &MultiLeaderGeometry,
    text_style_handle: Handle,
) -> (Handle, Vec<String>) {
    let key = style_name.to_ascii_lowercase();
    if let Some((_, first_values)) = acc.mleader_styles.get(&key) {
        let mut reasons = Vec::new();
        if first_values != geometry.style() {
            reasons.push(format!(
                "MLEADERSTYLE record '{style_name}' was written from the first entity's \
                 snapshot; this entity's style values differ, so the fields its 90 mask does \
                 not override re-resolve to the first snapshot on reload"
            ));
        }
        let handle = existing_style_handle(doc, style_name).unwrap_or(Handle::NULL);
        return (handle, reasons);
    }

    let record = style_record(style_name, geometry, text_style_handle);
    let mut reasons = Vec::new();

    let handle = match existing_style_handle(doc, style_name) {
        Some(handle) => {
            // The document already carries this style (the initial `Standard`
            // object): keep its identity and replace only the values, so the
            // name still has exactly one record.
            let owner = doc
                .objects
                .get(&handle)
                .and_then(|object| match object {
                    ObjectType::MultiLeaderStyle(style) => Some(style.owner_handle),
                    _ => None,
                })
                .or_else(|| style_dictionary(doc))
                .unwrap_or(Handle::NULL);
            let mut record = record;
            record.handle = handle;
            record.owner_handle = owner;
            doc.objects
                .insert(handle, ObjectType::MultiLeaderStyle(record));
            handle
        }
        None => {
            let handle = doc.allocate_handle();
            let mut record = record;
            record.handle = handle;
            match style_dictionary(doc) {
                Some(owner) => {
                    record.owner_handle = owner;
                    if let Some(ObjectType::Dictionary(dictionary)) = doc.objects.get_mut(&owner) {
                        dictionary.add_entry(style_name.to_string(), handle);
                    }
                }
                None => {
                    reasons.push(
                        "the ACAD_MLEADERSTYLE dictionary is missing; the style record is written \
                         with a null owner"
                            .into(),
                    );
                }
            }
            doc.objects
                .insert(handle, ObjectType::MultiLeaderStyle(record));
            handle
        }
    };

    acc.mleader_styles
        .insert(key, (style_name.to_string(), geometry.style().clone()));
    (handle, reasons)
}

/// Build the `MLEADERSTYLE` object from the resolved snapshot.
///
/// Values that the entity also writes are derived from the same placeholders as
/// the entity, so a mask-unset field re-resolves to the value the entity shows.
fn style_record(
    name: &str,
    geometry: &MultiLeaderGeometry,
    text_style_handle: Handle,
) -> MultiLeaderStyle {
    let style = geometry.style();
    let text = geometry.text();
    let mut record = MultiLeaderStyle::new(name);
    record.path_type = style_path_type(style.path_type);
    // `91`/`340`/`92` are the style's own colour/linetype/lineweight, which the
    // model does not carry; the entity writes the same defaults.
    record.line_color = Color::ByBlock;
    record.line_type_handle = None;
    record.line_weight = LineWeight::ByLayer;
    record.enable_landing = style.enable_landing;
    record.enable_dogleg = style.enable_dogleg;
    record.landing_distance = style.landing_distance;
    record.landing_gap = style.landing_gap;
    record.arrowhead_size = style.arrowhead_size;
    record.content_type = style_content_type(geometry.content());
    record.text_style_handle = Some(text_style_handle);
    record.text_left_attachment = StyleTextAttachment::from(text.left_attachment);
    record.text_right_attachment = UNMODELLED_STYLE_RIGHT_ATTACHMENT;
    record.text_angle_type = StyleTextAngle::Horizontal;
    record.text_alignment = StyleTextAlignment::Left;
    record.text_color = Color::ByBlock;
    // The effective text height the entity shows (CONTEXT_DATA 41) wins over the
    // snapshot field so the record and the entity never disagree when the mask
    // leaves TEXT_HEIGHT to the style.
    record.text_height = text.height;
    // The style's 142 scale is not in the snapshot (only the entity's tail `45`
    // is); leave the acadrust default rather than invent a second value.
    record.scale_factor = 1.0;
    record
}

fn content_type(content: MLeaderContent) -> acadrust::entities::LeaderContentType {
    match content {
        MLeaderContent::None => acadrust::entities::LeaderContentType::None,
        MLeaderContent::Text => acadrust::entities::LeaderContentType::MText,
    }
}

fn style_content_type(content: MLeaderContent) -> StyleContentType {
    match content {
        MLeaderContent::None => StyleContentType::None,
        MLeaderContent::Text => StyleContentType::MText,
    }
}

fn acad_path_type(path_type: MLeaderPathType) -> AcadPathType {
    match path_type {
        MLeaderPathType::Invisible => AcadPathType::Invisible,
        MLeaderPathType::StraightLineSegments => AcadPathType::StraightLineSegments,
        MLeaderPathType::Spline => AcadPathType::Spline,
    }
}

fn style_path_type(path_type: MLeaderPathType) -> StylePathType {
    match path_type {
        MLeaderPathType::Invisible => StylePathType::Invisible,
        MLeaderPathType::StraightLineSegments => StylePathType::StraightLineSegments,
        MLeaderPathType::Spline => StylePathType::Spline,
    }
}

fn attachment_direction(value: i16) -> TextAttachmentDirectionType {
    TextAttachmentDirectionType::from(value)
}

/// The raw `reserved` colour as an acadrust colour.
fn acad_color(color: &MLeaderColor) -> Color {
    if let Some(rgb) = color.true_color {
        Color::from_true_color_value(rgb)
    } else if let Some(index) = color.index {
        Color::from_index(index as i16)
    } else {
        Color::ByBlock
    }
}

fn reserved_root(geometry: &MultiLeaderGeometry, root_index: usize) -> Option<&MLeaderRootExtras> {
    geometry
        .reserved()
        .roots
        .iter()
        .find(|extras| extras.root_index == root_index)
}

fn has_breaks(geometry: &MultiLeaderGeometry) -> bool {
    let reserved = geometry.reserved();
    reserved.roots.iter().any(|root| !root.breaks.is_empty())
        || reserved
            .lines
            .iter()
            .any(|line| !line.breaks.is_empty() || !line.legacy_breaks.is_empty())
}

fn pair(points: &MLeaderBreak) -> StartEndPointPair {
    StartEndPointPair::new(v3(points.start), v3(points.end))
}

/// The plane's normal: the text normal is not stored, so it is derived from the
/// frame axes rather than written as a contradictory `+Z`.
fn plane_normal(plane: &PlaneFrame) -> Vector3 {
    let normal = Vector3::new(
        plane.u.y * plane.v.z - plane.u.z * plane.v.y,
        plane.u.z * plane.v.x - plane.u.x * plane.v.z,
        plane.u.x * plane.v.y - plane.u.y * plane.v.x,
    );
    if normal.length() > 1e-12 {
        normal.normalize()
    } else {
        Vector3::UNIT_Z
    }
}

/// The in-plane text direction for a rotation angle (`u cos + v sin`).
fn text_direction(plane: &PlaneFrame, rotation: f64) -> Vector3 {
    let (sin, cos) = rotation.sin_cos();
    let direction = Vector3::new(
        plane.u.x * cos + plane.v.x * sin,
        plane.u.y * cos + plane.v.y * sin,
        plane.u.z * cos + plane.v.z * sin,
    );
    if direction.length() > 1e-12 {
        direction.normalize()
    } else {
        Vector3::UNIT_X
    }
}

/// Build `CONTEXT_DATA` from the definition (decision E).
fn build_context(
    geometry: &MultiLeaderGeometry,
    style: &MLeaderStyleValues,
    text: &MLeaderText,
    text_style_handle: Handle,
) -> MultiLeaderAnnotContext {
    let plane = geometry.plane();
    let mut context = MultiLeaderAnnotContext::new();
    context.scale_factor = style.scale_factor;
    context.content_base_point = v3(text.location);
    context.text_height = text.height;
    context.arrowhead_size = style.arrowhead_size;
    context.landing_gap = style.landing_gap;
    context.text_left_attachment = TextAttachmentType::from(text.left_attachment);
    context.text_right_attachment = UNMODELLED_RIGHT_ATTACHMENT;
    context.text_alignment = acadrust::entities::TextAlignmentType::Left;
    context.has_text_contents = geometry.content() == MLeaderContent::Text;
    context.text_string = text.string.clone();
    context.text_normal = plane_normal(&plane);
    context.text_location = v3(text.location);
    context.text_direction = text_direction(&plane, text.rotation);
    context.text_rotation = text.rotation;
    context.text_width = text.width;
    // Line spacing and the background/column block are not modelled beyond the
    // preserved `reserved` data; the defaults are written when the source had
    // none, so a reload sees the same values.
    context.line_spacing_factor = 1.0;
    context.line_spacing_style = acadrust::entities::LineSpacingStyle::AtLeast;
    context.text_color = Color::ByBlock;
    context.text_attachment_point = TextAttachmentPointType::from(text.attachment_point);
    context.text_flow_direction = UNMODELLED_FLOW_DIRECTION;
    context.text_top_attachment = TextAttachmentType::CenterOfText;
    context.text_bottom_attachment = TextAttachmentType::CenterOfText;
    context.text_style_handle = Some(text_style_handle);
    if let Some(background) = &geometry.reserved().background {
        context.background_fill_enabled = background.fill_enabled;
        context.background_mask_fill_on = background.mask_fill_on;
        context.background_fill_color = acad_color(&background.fill_color);
        context.background_scale_factor = background.scale_factor;
        context.background_transparency = background.transparency;
    }
    if let Some(columns) = &geometry.reserved().columns {
        context.column_type = columns.column_type;
        context.column_width = columns.width;
        context.column_gutter = columns.gutter;
        context.column_flow_reversed = columns.flow_reversed;
        context.column_sizes = columns.sizes.clone();
    }
    context.base_point = v3(plane.origin);
    context.base_direction = v3(plane.u);
    context.base_vertical = v3(plane.v);
    context.normal_reversed = plane.normal_reversed;
    context
}

/// Build the `LEADER{}` roots and their `LEADER_LINE{}` members.
///
/// Vertices are written one by one, the per-line override word and arrowhead
/// fields are the model's own values, and unmodelled root/line data comes from
/// the verbatim `reserved` bag (each source group is written once). Break pairs
/// stored as complete records are written through `break_infos`; the legacy
/// `break_points` mirror is only written when no complete record exists, so the
/// same pair is never written twice — the writer derives `segment_index` and
/// `break_info_count` from what it writes.
fn build_roots(geometry: &MultiLeaderGeometry) -> Vec<AcadLeaderRoot> {
    let reserved = geometry.reserved();
    let mut roots = Vec::with_capacity(geometry.roots().len());
    for (root_index, root) in geometry.roots().iter().enumerate() {
        let extras = reserved_root(geometry, root_index);
        let mut leader = AcadLeaderRoot::new(root.leader_index);
        leader.content_valid = extras.map(|extras| extras.content_valid).unwrap_or(true);
        leader.unknown = extras.map(|extras| extras.unknown).unwrap_or(true);
        leader.connection_point = v3(root.landing_point);
        leader.direction = v3(root.dogleg_vector);
        leader.landing_distance = root.dogleg_length;
        leader.text_attachment_direction = extras
            .map(|extras| attachment_direction(extras.text_attachment_direction))
            .unwrap_or(TextAttachmentDirectionType::Horizontal);
        leader.break_points = extras
            .map(|extras| extras.breaks.iter().map(pair).collect())
            .unwrap_or_default();

        for (line_index, line) in geometry
            .lines()
            .iter()
            .enumerate()
            .filter(|(_, line)| line.root_index == root_index)
        {
            let line_extras = reserved
                .lines
                .iter()
                .find(|extras| extras.root_index == root_index && extras.line_index == line_index);
            let points = line.points.iter().map(|point| v3(*point)).collect();
            let mut written = AcadLeaderLine::from_points(line.index, points);
            written.path_type = acad_path_type(line.path_type);
            written.override_flags =
                acadrust::entities::LeaderLinePropertyOverrideFlags::from_bits_retain(
                    line.override_flags,
                );
            written.arrowhead_size = line.arrowhead_size;
            written.arrowhead_handle = line.arrowhead_handle.map(Handle::from);
            if let Some(extras) = line_extras {
                written.segment_index = extras.segment_index;
                written.break_info_count = extras.break_info_count;
                written.break_infos = extras
                    .breaks
                    .iter()
                    .map(|record| LeaderLineBreakInfo {
                        segment_index: record.segment_index,
                        break_points: record.points.iter().map(pair).collect(),
                    })
                    .collect();
                written.break_points = extras.legacy_breaks.iter().map(pair).collect();
                written.line_color = acad_color(&extras.line_color);
                written.line_type_handle = extras.line_type_handle.map(Handle::from);
                written.line_weight = LineWeight::from_value(extras.line_weight);
            }
            leader.lines.push(written);
        }
        roots.push(leader);
    }
    roots
}
