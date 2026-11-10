//! The typed `IMAGEATTACH` / `IMAGECLIP` grammars and their dispatch.
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
    parse_imageattach_argument, parse_imageclip_argument, ARGUMENT_SELECTORS, IMAGEATTACH_SELECTOR,
    IMAGECLIP_SELECTOR,
};
use cad_app::{ExtendedCommandId, ImageAttachCommand, ImageClipCommand};

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
fn imageclip_argument_is_a_rectangle_or_off() {
    assert_eq!(
        parse_imageclip_argument("0 0 320 240"),
        Some(ImageClipCommand {
            rect: Some([0.0, 0.0, 320.0, 240.0])
        })
    );
    assert_eq!(
        parse_imageclip_argument("OFF"),
        Some(ImageClipCommand { rect: None })
    );
    assert_eq!(
        parse_imageclip_argument("off"),
        Some(ImageClipCommand { rect: None })
    );
    // Ordering is the command's `imageclip.invalid_rect` refusal, not syntax:
    // the four finite numbers are admitted and the command decides.
    assert_eq!(
        parse_imageclip_argument("10 0 0 10"),
        Some(ImageClipCommand {
            rect: Some([10.0, 0.0, 0.0, 10.0])
        })
    );
    for refused in ["", "0 0 1", "0 0 1 1 1", "0 0 x 1", "OFF EXTRA", "NONE"] {
        assert_eq!(parse_imageclip_argument(refused), None, "{refused:?}");
    }
}

#[test]
fn both_selectors_are_admitted_argument_forms_of_their_extended_commands() {
    for selector in [IMAGEATTACH_SELECTOR, IMAGECLIP_SELECTOR] {
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
fn malformed_image_arguments_are_refused_before_dispatch() {
    let session = fresh_session();
    for line in [
        "IMAGEATTACH logo.png 0 0 1 1",
        "IMAGEATTACH logo.png 0 0 1 1 1 2",
        "IMAGECLIP 0 0 1",
        "IMAGECLIP NONE",
    ] {
        let error = session.command(line).expect_err(line);
        assert!(error.contains("unsupported CAD command"), "{line}: {error}");
    }
}
