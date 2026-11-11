//! The typed `IMAGEATTACH` / `IMAGECLIP` / `IMAGEADJUST` grammars and their
//! dispatch.
//!
//! These pin the textual grammar (a single-token file plus five numbers, or a
//! four-number clip / `OFF`), the admission of both selectors as argument forms
//! of their existing extended commands, and the payload path: a valid attach
//! creates a real image row, and a valid clip with no selection reaches the
//! command layer and is refused there by name (`imageclip.no_selection`), which
//! proves the mapping without duplicating `cad-app`'s semantics.

use super::super::super::*;
use super::super::test_support::*;

use crate::session::commands::{
    parse_imageadjust_argument, parse_imageattach_argument, parse_imageclip_argument,
    ARGUMENT_SELECTORS, DASH_IMAGEADJUST_SELECTOR, IMAGEADJUST_SELECTOR, IMAGEATTACH_SELECTOR,
    IMAGECLIP_SELECTOR,
};
use cad_app::{ExtendedCommandId, ImageAdjustCommand, ImageAttachCommand, ImageClipCommand};
use cad_domain::Point3;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn imageattach_argument_is_a_single_token_file_plus_five_numbers() {
    assert_eq!(
        parse_imageattach_argument("textures/logo.png 10 20 640 480 0.05"),
        Some(ImageAttachCommand {
            file: "textures/logo.png".into(),
            origin: Point3 {
                x: 10.0,
                y: 20.0,
                z: 0.0
            },
            pixels: [640.0, 480.0],
            units_per_pixel: 0.05,
        })
    );
    // Sign and fraction are legal on the point; the three sizes are positive.
    assert!(parse_imageattach_argument("logo.png -1 -2.5 1 1 2").is_some());
    // A path with spaces is not typable in v1: the file is one token.
    assert_eq!(parse_imageattach_argument("my logo.png 0 0 1 1 1"), None);
    for refused in [
        "",
        "logo.png",
        "logo.png 0 0 1 1",
        "logo.png 0 0 1 1 1 2",
        "logo.png a 0 1 1 1",
        "logo.png 0 0 0 1 1",
        "logo.png 0 0 -1 1 1",
        "logo.png 0 0 1 1 0",
        "logo.png 0 0 1 1 1e-2",
    ] {
        assert_eq!(parse_imageattach_argument(refused), None, "{refused:?}");
    }
}

#[test]
fn imageclip_argument_is_a_rectangle_a_polygon_or_off() {
    assert_eq!(
        parse_imageclip_argument("0 0 320 240"),
        Some(ImageClipCommand::Rect([0.0, 0.0, 320.0, 240.0]))
    );
    assert_eq!(parse_imageclip_argument("OFF"), Some(ImageClipCommand::Off));
    assert_eq!(parse_imageclip_argument("off"), Some(ImageClipCommand::Off));
    // Ordering is the command's `imageclip.invalid_rect` refusal, not syntax:
    // the four finite numbers are admitted and the command decides.
    assert_eq!(
        parse_imageclip_argument("10 0 0 10"),
        Some(ImageClipCommand::Rect([10.0, 0.0, 0.0, 10.0]))
    );
    // POLY: world-space pairs on the flat work plane.
    assert_eq!(
        parse_imageclip_argument("POLY 0 0 10 0 10 5"),
        Some(ImageClipCommand::Polygon(vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0
            },
            Point3 {
                x: 10.0,
                y: 0.0,
                z: 0.0
            },
            Point3 {
                x: 10.0,
                y: 5.0,
                z: 0.0
            },
        ]))
    );
    assert_eq!(
        parse_imageclip_argument("poly 0 0 10 0 10 5 0 5"),
        parse_imageclip_argument("POLY 0 0 10 0 10 5 0 5")
    );
    for refused in [
        "",
        "0 0 1",
        "0 0 1 1 1",
        "0 0 x 1",
        "OFF EXTRA",
        "NONE",
        // Fewer than three point pairs, an odd coordinate count and a
        // non-numeric coordinate are all outside the grammar.
        "POLY 0 0 10 0",
        "POLY 0 0 10 0 10",
        "POLY 0 0 10 0 x 5",
    ] {
        assert_eq!(parse_imageclip_argument(refused), None, "{refused:?}");
    }
}

#[test]
fn imageadjust_argument_is_three_percentages_plus_an_optional_word_or_reset() {
    let set = |brightness, contrast, fade, monochrome| ImageAdjustCommand::Set {
        brightness,
        contrast,
        fade,
        monochrome,
    };
    assert_eq!(
        parse_imageadjust_argument("30 70 20"),
        Some(set(30, 70, 20, None)),
        "an omitted MONO/COLOR keeps the stored state"
    );
    assert_eq!(
        parse_imageadjust_argument("30 70 20 MONO"),
        Some(set(30, 70, 20, Some(true)))
    );
    assert_eq!(
        parse_imageadjust_argument("30 70 20 mono"),
        Some(set(30, 70, 20, Some(true))),
        "the word is case-insensitive"
    );
    assert_eq!(
        parse_imageadjust_argument("0 0 0 COLOR"),
        Some(set(0, 0, 0, Some(false)))
    );
    assert_eq!(
        parse_imageadjust_argument("RESET"),
        Some(ImageAdjustCommand::Reset)
    );
    assert_eq!(
        parse_imageadjust_argument("reset"),
        Some(ImageAdjustCommand::Reset)
    );
    // 150 is admitted by the *grammar* (a u8 integer); the command refuses the
    // real 0..=100 range by name, which the dispatch test below pins.
    assert_eq!(
        parse_imageadjust_argument("150 50 0"),
        Some(set(150, 50, 0, None))
    );
    for refused in [
        "",
        "30",
        "30 70",
        "30 70 20 EXTRA",
        "30 70 20 MONO EXTRA",
        "-1 50 0",
        "30.5 70 20",
        "300 70 20",
        "a b c",
        "RESET EXTRA",
    ] {
        assert_eq!(parse_imageadjust_argument(refused), None, "{refused:?}");
    }
}

#[test]
fn both_selectors_are_admitted_argument_forms_of_their_extended_commands() {
    for selector in [
        IMAGEATTACH_SELECTOR,
        IMAGECLIP_SELECTOR,
        IMAGEADJUST_SELECTOR,
        DASH_IMAGEADJUST_SELECTOR,
    ] {
        assert!(
            ARGUMENT_SELECTORS.contains(&selector),
            "{selector} must be an argument selector"
        );
    }
    assert_eq!(
        normalize_command("IMAGEATTACH"),
        Some(CommandId::Extended(ExtendedCommandId::ImageAttach))
    );
    assert_eq!(
        normalize_command("IMAGECLIP"),
        Some(CommandId::Extended(ExtendedCommandId::ImageClip))
    );
    assert_eq!(canonical_command_name("IMAGEATTACH"), Some("IMAGEATTACH"));
    assert_eq!(canonical_command_name("IMAGECLIP"), Some("IMAGECLIP"));
    assert_eq!(
        normalize_command("IMAGEADJUST"),
        Some(CommandId::Extended(ExtendedCommandId::ImageAdjust))
    );
    assert_eq!(
        normalize_command("-IMAGEADJUST"),
        Some(CommandId::Extended(ExtendedCommandId::DashImageAdjust))
    );
    assert_eq!(canonical_command_name("IMAGEADJUST"), Some("IMAGEADJUST"));
    assert_eq!(canonical_command_name("-IMAGEADJUST"), Some("-IMAGEADJUST"));
}

#[test]
fn typed_imageattach_dispatches_a_real_image_entity() {
    let session = fresh_session();
    session
        .command("IMAGEATTACH textures/logo.png 10 20 640 480 0.05")
        .expect("the typed attach creates one image");
    let c = session.core.controller.borrow();
    let drawing = c.drawing().expect("a drawing is open");
    let images: Vec<_> = drawing
        .model_space()
        .iter()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Image {
                origin,
                pixels,
                file,
                ..
            } => Some((*origin, *pixels, file.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(images.len(), 1, "exactly one image row");
    let (origin, pixels, file) = &images[0];
    assert_eq!(
        *origin,
        Point3 {
            x: 10.0,
            y: 20.0,
            z: 0.0
        }
    );
    assert_eq!(*pixels, [640.0, 480.0]);
    assert_eq!(file.as_deref(), Some("textures/logo.png"));
}

#[test]
fn typed_imageclip_reaches_the_command_layer_and_refuses_no_selection() {
    let session = fresh_session();
    let error = session
        .command("IMAGECLIP 0 0 320 240")
        .expect_err("nothing is selected");
    assert!(
        error.contains("imageclip.no_selection"),
        "the payload reached the command layer: {error}"
    );
}

#[test]
fn typed_imageadjust_reaches_the_command_layer_and_refuses_by_name() {
    let session = fresh_session();
    // Nothing is selected: the payload still reaches the command layer, which is
    // the authority for the selection refusal.
    for line in ["IMAGEADJUST 30 70 20", "-IMAGEADJUST 30 70 20 MONO"] {
        let error = session.command(line).expect_err(line);
        assert!(
            error.contains("imageadjust.no_selection"),
            "{line}: {error}"
        );
    }
    // 150 is out of the 0..=100 range: the command's own named refusal.
    let error = session
        .command("IMAGEADJUST 150 50 0")
        .expect_err("150 is out of range");
    assert!(error.contains("imageadjust.invalid_value"), "{error}");
    // A bare selector is admitted by name and refused by the command: the
    // grammar never defaults a transfer.
    for line in ["IMAGEADJUST", "-IMAGEADJUST"] {
        let error = session.command(line).expect_err(line);
        assert!(error.contains("imageadjust.payload"), "{line}: {error}");
    }
}

#[test]
fn malformed_image_arguments_are_refused_before_dispatch() {
    let session = fresh_session();
    for (line, code) in [
        ("IMAGEATTACH logo.png 0 0 1 1", "imageattach.argument"),
        ("IMAGEATTACH logo.png 0 0 1 1 1 2", "imageattach.argument"),
        ("IMAGECLIP 0 0 1", "imageclip.argument"),
        ("IMAGECLIP NONE", "imageclip.argument"),
        ("IMAGEADJUST 30 70", "imageadjust.argument"),
        ("IMAGEADJUST 30 70 20 EXTRA", "imageadjust.argument"),
        ("-IMAGEADJUST 30 70 20 MONO EXTRA", "imageadjust.argument"),
    ] {
        let error = session.command(line).expect_err(line);
        assert!(error.starts_with(code), "{line}: {error}");
        assert!(error.contains("nothing was changed"), "{line}: {error}");
    }
}
