use super::*;

#[test]
fn entity_flags_resolve_a_source_to_its_batch_key() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    renderer
        .upload(&scene(vec![quad(0.0, 0.0, 8.0, 0x123, false)]))
        .expect("upload");
    let key = renderer.pick_key_for_batch(0).expect("key");

    renderer
        .set_entity_flags(&[(source(0x123), FLAG_SELECTED)])
        .expect("set entity flags");
    assert_eq!(renderer.entity_flag_keys(), vec![key]);
    assert_eq!(renderer.pick_key_flags(key), FLAG_SELECTED);

    // The call replaces the whole entity set: an empty list clears it.
    renderer.set_entity_flags(&[]).expect("clear entity flags");
    assert!(renderer.entity_flag_keys().is_empty());
    assert_eq!(renderer.pick_key_flags(key), 0);
}

#[test]
fn an_insert_instance_flag_targets_only_that_placement() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let mut first = line_band(6.0, 3, 0);
    first.sources = vec![source_instance(20, &[10])];
    let mut second = line_band(-6.0, 3, 0);
    second.sources = vec![source_instance(20, &[20])];
    renderer
        .upload(&scene(vec![first, second]))
        .expect("upload");
    let first_key = renderer.pick_key_for_batch(0).expect("key");
    let second_key = renderer.pick_key_for_batch(1).expect("key");

    renderer
        .set_entity_flags(&[(source_instance(20, &[10]), FLAG_SELECTED)])
        .expect("set entity flags");
    assert_eq!(renderer.entity_flag_keys(), vec![first_key]);
    assert_eq!(renderer.pick_key_flags(first_key), FLAG_SELECTED);
    assert_eq!(
        renderer.pick_key_flags(second_key),
        0,
        "the sibling placement must stay unflagged"
    );
}

/// A merged multi-source batch is tinted as a whole: the static streams do not
/// record which fragment range belongs to which source, so a flag on one of its
/// sources necessarily tints the batch. This is the documented granularity limit,
/// not a silent per-fragment claim.
#[test]
fn a_multi_source_batch_is_tinted_at_batch_granularity() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let mut batch = line_band(6.0, 3, 0);
    batch.sources = vec![source(0x1000), source(0x2000)];
    renderer.upload(&scene(vec![batch])).expect("upload");
    let key = renderer.pick_key_for_batch(0).expect("key");

    // Flag only one of the batch's two sources.
    renderer
        .set_entity_flags(&[(source(0x1000), FLAG_SELECTED)])
        .expect("set entity flags");
    assert_eq!(
        renderer.entity_flag_keys(),
        vec![key],
        "the whole multi-source batch's key carries the flag"
    );
    assert_eq!(renderer.pick_key_flags(key), FLAG_SELECTED);
}

#[test]
fn entity_selected_and_hovered_are_visibly_tinted_and_hidden_is_not_drawn() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let target = RenderTarget::new(96, 96);
    renderer
        .upload(&scene(vec![quad(0.0, 0.0, 8.0, 0x55, false)]))
        .expect("upload");
    let plain = render(&mut renderer, &target);
    let (x, y) = world_to_pixel(&camera_2d(), 96, 96, [0.0, 0.0]);
    let i = (y as usize * 96 + x as usize) * 4;

    renderer.set_selection_flags_enabled(true);
    renderer
        .set_entity_flags(&[(source(0x55), FLAG_SELECTED)])
        .expect("selected");
    let selected = render(&mut renderer, &target);
    assert_ne!(selected, plain, "a selected entity must be tinted");
    assert!(
        selected[i] > 8 || selected[i + 1] > 8 || selected[i + 2] > 8,
        "the selected entity stays visible"
    );

    renderer
        .set_entity_flags(&[(source(0x55), FLAG_HOVERED)])
        .expect("hovered");
    let hovered = render(&mut renderer, &target);
    assert_ne!(hovered, plain, "a hovered entity must be tinted");
    assert_ne!(
        hovered, selected,
        "hover is a lighter/different tint of the same mechanism"
    );

    renderer
        .set_entity_flags(&[(source(0x55), FLAG_HIDDEN)])
        .expect("hidden");
    let hidden = render(&mut renderer, &target);
    assert!(
        hidden[i] <= 8 && hidden[i + 1] <= 8 && hidden[i + 2] <= 8,
        "a hidden entity must be discarded, got {:?}",
        &hidden[i..i + 4]
    );
}

#[test]
fn entity_flags_tint_an_instanced_line() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let target = RenderTarget::new(96, 96);
    renderer
        .upload(&scene(vec![line_band(6.0, 3, 0xAB)]))
        .expect("upload");
    let plain = render(&mut renderer, &target);
    let (x, y) = first_drawn_pixel(&plain, 96, 96).expect("drawn");
    let i = (y as usize * 96 + x as usize) * 4;

    renderer.set_selection_flags_enabled(true);
    renderer
        .set_entity_flags(&[(source(0xAB), FLAG_SELECTED)])
        .expect("selected");
    let tinted = render(&mut renderer, &target);
    assert_ne!(
        &tinted[i..i + 4],
        &plain[i..i + 4],
        "an instanced line entity must be tinted through the in-stream flags"
    );
}

#[test]
fn a_selection_change_uploads_only_flags_data() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let target = RenderTarget::new(96, 96);
    renderer
        .upload(&scene(vec![
            quad(-12.0, 0.0, 5.0, 0x0A, false),
            line_band(6.0, 3, 0x0B),
        ]))
        .expect("upload");
    let _ = render(&mut renderer, &target);
    let streams = renderer.instance_stream_uploads();
    let chunks = renderer.mesh_chunk_uploads();
    let flags_before = renderer.flags_upload_count();

    // A selection change is only a flags update: no stream/chunk re-upload.
    renderer
        .set_entity_flags(&[(source(0x0A), FLAG_SELECTED)])
        .expect("select A");
    renderer
        .set_entity_flags(&[(source(0x0B), FLAG_SELECTED)])
        .expect("select B");
    assert!(
        renderer.flags_upload_count() > flags_before,
        "a selection change must upload flags"
    );
    assert_eq!(
        renderer.instance_stream_uploads(),
        streams,
        "a selection change must not re-upload the static instance streams"
    );
    assert_eq!(
        renderer.mesh_chunk_uploads(),
        chunks,
        "a selection change must not re-upload the static merged chunks"
    );
}

#[test]
fn flags_enabled_with_no_entity_flags_is_pixel_identical() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let target = RenderTarget::new(96, 96);
    renderer
        .upload(&scene(vec![quad(0.0, 0.0, 8.0, 0x55, false)]))
        .expect("upload");
    let before = render(&mut renderer, &target);

    // The production session enables the toggle; with an empty entity-flag set
    // the output must be byte-identical to the pre-flags frame.
    renderer.set_selection_flags_enabled(true);
    renderer.set_entity_flags(&[]).expect("no flags");
    let after = render(&mut renderer, &target);
    assert_eq!(
        after, before,
        "flags enabled but empty must be identical to before"
    );
}

/// B2: an image entity flag must survive a base-change image re-upload onto the
/// **current** key. `upload_images` allocates fresh keys per call and rebuilds the
/// pick table, so a flag pushed before the re-upload would otherwise stay on a
/// retired key and the flagged image would flash back to unflagged after an edit.
#[test]
fn an_image_entity_flag_survives_a_re_upload_onto_the_current_key() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let mut cache = DecodedImageCache::new();
    cache
        .insert(
            ResourceKey("red".into()),
            solid_image(2, 2, [255, 0, 0, 255]),
            &ResourceLimits::default(),
        )
        .expect("insert red image");
    renderer
        .upload_images(&[image_batch("red", 0xE0)], &cache)
        .expect("image");
    let first = renderer.pick_key_for_image(0).expect("image key");
    renderer
        .set_entity_flags(&[(source(0xE0), FLAG_SELECTED)])
        .expect("select image");
    assert_eq!(renderer.pick_key_flags(first), FLAG_SELECTED);

    // A base change re-uploads the image: a fresh key, a rebuilt pick table.
    renderer
        .upload_images(&[image_batch("red", 0xE0)], &cache)
        .expect("re-upload");
    let current = renderer.pick_key_for_image(0).expect("image key");
    assert_ne!(current, first, "the re-upload allocates a fresh key");
    assert_eq!(
        renderer.pick_key_flags(current),
        FLAG_SELECTED,
        "the current image key must carry the flag after the re-upload (B2)"
    );
}

/// S2: an entity-level flag must not tint the sub-element overlay batch. The face
/// overlay's source is `E#face-a`; matching it by whole-entity identity would
/// double-tint the face and let a future `FLAG_HIDDEN` discard the face highlight.
#[test]
fn an_entity_flag_does_not_tint_a_sub_element_overlay_batch() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let mut face = quad(0.0, 12.0, 4.0, 0xE1, false);
    face.sources = vec![SelectionRef {
        document: DocumentId(1),
        entity: EntityId(0xE1),
        instance: InstancePath::default(),
        sub_element: Some(SubElementId {
            source_key: "face-a".into(),
            topology_revision: Revision(0),
        }),
    }];
    renderer
        .upload(&scene(vec![quad(0.0, -12.0, 4.0, 0xE1, false), face]))
        .expect("upload");
    let base_key = renderer.pick_key_for_batch(0).expect("base key");
    let face_key = renderer.pick_key_for_batch(1).expect("face key");

    // The combined `E` + `E#face-a` selection resolves the entity flag for `E`
    // only; the face batch must stay unflagged.
    renderer
        .set_entity_flags(&[(source(0xE1), FLAG_SELECTED)])
        .expect("select E");
    assert_eq!(
        renderer.entity_flag_keys(),
        vec![base_key],
        "only the whole-entity batch carries the entity flag"
    );
    assert_eq!(renderer.pick_key_flags(base_key), FLAG_SELECTED);
    assert_eq!(
        renderer.pick_key_flags(face_key),
        0,
        "the sub-element overlay batch must not be tinted by an entity-level flag (S2)"
    );
}

/// S1: a requested ref that resolves in the database but matches no resident pick
/// key must be reported, not silently dropped (iron rule #2).
#[test]
fn an_entity_flag_with_no_resident_key_is_reported_not_silently_dropped() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    renderer
        .upload(&scene(vec![quad(0.0, 0.0, 4.0, 0x01, false)]))
        .expect("upload");
    let report = renderer
        .set_entity_flags(&[(source(0xDEAD), FLAG_SELECTED)])
        .expect("set flags");
    assert_eq!(report.matched, 0);
    assert_eq!(
        report.unmatched,
        vec![source(0xDEAD)],
        "the unmatched ref must be surfaced for the caller to report"
    );
    assert!(renderer.entity_flag_keys().is_empty());
}

/// A flag change must invalidate a *completed* progressive frame. Otherwise the
/// next `render` with an unchanged frame key draws nothing and keeps the
/// pre-flag pixels, so a selection would not tint until the camera moved — the
/// exact bug the production session (progressive rendering on) exposed.
#[test]
fn a_flag_change_redraws_a_completed_progressive_frame() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu, None);
    let target = RenderTarget::new(96, 96);
    renderer.set_progressive_rendering(true);
    renderer
        .upload(&scene(vec![quad(0.0, 0.0, 8.0, 0x55, false)]))
        .expect("upload");
    renderer.set_selection_flags_enabled(true);
    let plain = render(&mut renderer, &target);

    renderer
        .set_entity_flags(&[(source(0x55), FLAG_SELECTED)])
        .expect("select");
    let tinted = render(&mut renderer, &target);
    assert_ne!(
        plain, tinted,
        "a flags change must redraw the completed progressive frame"
    );
}
