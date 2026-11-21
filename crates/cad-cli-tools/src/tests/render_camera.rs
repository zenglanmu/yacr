//! `render` 3D camera/view contracts.
//!
//! Two layers, both crate-local:
//!
//! * flag parsing and refusal contracts (`--view`, `--azimuth`, `--elevation`,
//!   `--fov`, `--legacy-shading`) that must be usable errors rather than silent
//!   ignores;
//! * one real headless frame proving the projection is genuinely 3D and
//!   angle-dependent, driven through `ops::render::render_document` on a
//!   document installed in memory (a wedge with real z extent).
//!
//! The GPU case binds a real adapter and auto-skips when there is none
//! (`SKIP ... NOT a pass`), exactly like the other contracts here. A software
//! adapter's pixels are layout evidence only, never a GPU claim.

use super::*;

use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};
use cad_domain::{
    DatabaseId, DocumentIdentity, EntityId, LayerId, Mesh, ObjectId, Point3, Revision,
    SemanticGeometry, SpaceId, UnitContext,
};

/// No adapter means this machine cannot answer the question; skip rather than
/// report a failure that is not about the code under test.
#[cfg(not(target_arch = "wasm32"))]
fn adapter_available() -> bool {
    use cad_render_wgpu::headless::enumerate_adapters;
    use cad_render_wgpu::BackendPreference;

    let available = !enumerate_adapters(BackendPreference::Auto).is_empty();
    if !available {
        eprintln!("no wgpu adapter; skipping 3D render contract — NOT a pass");
    }
    available
}

/// A real solid-ish shape: a base triangle at `z = 0` and a slanted face rising
/// to `z = 2`, sharing the edge `(3,0,0)-(0,1,0)`.
///
/// Deliberately asymmetric in x/y (a symmetric wedge would make azimuth 0 and 90
/// mirror images) and genuinely three-dimensional (the flat `z = 0` triangle used
/// by the texture contracts cannot tell a perspective view from the plan view).
#[cfg(not(target_arch = "wasm32"))]
fn wedge() -> Mesh {
    let vertices = vec![
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 3.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
        Point3 {
            x: 3.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 2.0,
        },
    ];
    // Base face `+Z`, slanted face `normalize((2, 6, 3))` (outward), both wound
    // CCW so the renderer's back-face culling keeps them.
    let normals = vec![
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
        Point3 {
            x: 2.0 / 7.0,
            y: 6.0 / 7.0,
            z: 3.0 / 7.0,
        },
        Point3 {
            x: 2.0 / 7.0,
            y: 6.0 / 7.0,
            z: 3.0 / 7.0,
        },
        Point3 {
            x: 2.0 / 7.0,
            y: 6.0 / 7.0,
            z: 3.0 / 7.0,
        },
    ];
    Mesh {
        vertices,
        triangles: vec![[0, 1, 2], [3, 4, 5]],
        normals,
        face_sources: Vec::new(),
        colors: Vec::new(),
        uvs: Vec::new(),
        texture: None,
        pbr: None,
    }
}

/// A controller whose document is one wedge mesh in model space.
#[cfg(not(target_arch = "wasm32"))]
fn controller() -> HostController {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .expect("layer");
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "AcDbMesh".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Mesh(wedge()),
            draw_order: 0,
        })
        .expect("mesh entity");
    let database = builder.finish().expect("database");
    let snapshot = database
        .snapshot(
            UnitContext::drawing_units(),
            DocumentIdentity::Temporary(1),
            Vec::new(),
            Revision(0),
        )
        .expect("snapshot");

    let mut controller = HostController::with_demo_document([64.0, 64.0]).unwrap();
    controller
        .install_snapshot(
            &snapshot,
            cad_app::PersistenceState::untitled(),
            Some("wedge".into()),
        )
        .expect("the wedge snapshot installs");
    controller
}

/// A 64x64 render invocation, so a contract frame is cheap on a CPU adapter.
#[cfg(not(target_arch = "wasm32"))]
fn invocation() -> CliInvocation {
    let mut invocation = CliInvocation::new(CliOperation::FixedViewportRender, "wedge.dwg");
    invocation.render_width = 64;
    invocation.render_height = 64;
    invocation.locale = Locale::En;
    invocation
}

/// A plain 2D render invocation (no view flags) for the parse contracts.
fn plain() -> CliInvocation {
    CliInvocation::new(CliOperation::FixedViewportRender, "x.dwg")
}

#[test]
fn every_standard_view_preset_parses_case_insensitively() {
    for view in cad_app::StandardView::ALL {
        assert_eq!(parse_view_preset(view.key()), Some(view), "{view:?}");
        assert_eq!(
            parse_view_preset(&view.key().to_ascii_uppercase()),
            Some(view),
            "{view:?}"
        );
        assert_eq!(
            parse_view_preset(&format!("  {}  ", view.key())),
            Some(view),
            "{view:?}"
        );
    }
    assert_eq!(parse_view_preset("bogus"), None);
    assert_eq!(parse_view_preset(""), None);
}

#[test]
fn view_flag_combinations_resolve_or_refuse_as_usage_errors() {
    // None of the view flags: the unchanged 2D plan path.
    assert_eq!(plain().requested_view().unwrap(), None);

    let preset = CliInvocation {
        view_preset: Some(cad_app::StandardView::Isometric),
        ..plain()
    };
    assert_eq!(
        preset.requested_view().unwrap(),
        Some(RequestedView::Preset(cad_app::StandardView::Isometric))
    );

    let angles = CliInvocation {
        view_azimuth_degrees: Some(0.0),
        view_elevation_degrees: Some(20.0),
        ..plain()
    };
    assert_eq!(
        angles.requested_view().unwrap(),
        Some(RequestedView::Angles {
            azimuth_degrees: 0.0,
            elevation_degrees: 20.0,
        })
    );

    // Each inconsistent combination is a usage error (exit code 2), never a
    // silent ignore.
    let azimuth_only = CliInvocation {
        view_azimuth_degrees: Some(0.0),
        ..plain()
    };
    let elevation_only = CliInvocation {
        view_elevation_degrees: Some(10.0),
        ..plain()
    };
    let view_and_angles = CliInvocation {
        view_preset: Some(cad_app::StandardView::Top),
        view_azimuth_degrees: Some(0.0),
        view_elevation_degrees: Some(10.0),
        ..plain()
    };
    for (label, invocation, needle) in [
        ("azimuth only", azimuth_only, "together"),
        ("elevation only", elevation_only, "together"),
        ("view + angles", view_and_angles, "mutually exclusive"),
    ] {
        let error = invocation.requested_view().unwrap_err();
        assert_eq!(error.code, error_code::USAGE, "{label}");
        assert_eq!(error.exit_code(), 2, "{label}");
        assert!(error.message.contains(needle), "{label}: {}", error.message);
    }

    // `--legacy-shading` is a plain boolean flag, off by default.
    assert!(!plain().legacy_shading);
}

#[test]
fn fov_and_angle_values_refuse_out_of_range_instead_of_clamping() {
    assert_eq!(parse_fov_degrees("45"), Ok(45.0));
    assert_eq!(parse_fov_degrees("1"), Ok(1.0));
    assert_eq!(parse_fov_degrees("179"), Ok(179.0));
    for bad in ["0", "200", "-5", "NaN", "inf", "abc"] {
        assert!(parse_fov_degrees(bad).is_err(), "--fov {bad}");
    }

    assert_eq!(parse_azimuth_degrees("-360"), Ok(-360.0));
    assert_eq!(parse_azimuth_degrees("360"), Ok(360.0));
    for bad in ["-361", "361", "NaN", "inf", "abc"] {
        assert!(parse_azimuth_degrees(bad).is_err(), "--azimuth {bad}");
    }

    assert_eq!(parse_elevation_degrees("-90"), Ok(-90.0));
    assert_eq!(parse_elevation_degrees("90"), Ok(90.0));
    for bad in ["-91", "91", "NaN", "inf", "abc"] {
        assert!(parse_elevation_degrees(bad).is_err(), "--elevation {bad}");
    }

    // `--fov` without any 3D view is a usage error, not a silent ignore.
    let fov_only = CliInvocation {
        view_fov_degrees: Some(45.0),
        ..plain()
    };
    let error = fov_only.requested_view().unwrap_err();
    assert_eq!(error.code, error_code::USAGE);
    assert!(error.message.contains("--fov"), "{}", error.message);

    // ... and is accepted with either way of asking for a 3D view.
    let with_preset = CliInvocation {
        view_preset: Some(cad_app::StandardView::Front),
        view_fov_degrees: Some(60.0),
        ..plain()
    };
    assert!(with_preset.requested_view().unwrap().is_some());
    let with_angles = CliInvocation {
        view_azimuth_degrees: Some(45.0),
        view_elevation_degrees: Some(10.0),
        view_fov_degrees: Some(60.0),
        ..plain()
    };
    assert!(with_angles.requested_view().unwrap().is_some());

    // A programmatically supplied out-of-range value is refused too.
    let bad_angle = CliInvocation {
        view_azimuth_degrees: Some(400.0),
        view_elevation_degrees: Some(10.0),
        ..plain()
    };
    assert_eq!(
        bad_angle.requested_view().unwrap_err().code,
        error_code::USAGE
    );
}

#[test]
fn angle_direction_and_up_hint_use_the_cad_z_up_frame() {
    let horizontal = RequestedView::Angles {
        azimuth_degrees: 0.0,
        elevation_degrees: 0.0,
    };
    let direction = horizontal.direction();
    assert!((direction.x - 1.0).abs() < 1e-12);
    assert!(direction.y.abs() < 1e-12);
    assert!(direction.z.abs() < 1e-12);
    assert_eq!(
        horizontal.up_hint(),
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        }
    );

    let tilted = RequestedView::Angles {
        azimuth_degrees: 90.0,
        elevation_degrees: 20.0,
    };
    let direction = tilted.direction();
    assert!(direction.x.abs() < 1e-12);
    assert!((direction.y - 20.0f64.to_radians().cos()).abs() < 1e-12);
    assert!((direction.z - 20.0f64.to_radians().sin()).abs() < 1e-12);

    // Near-vertical directions fall back to world +Y so the basis stays valid.
    let vertical = RequestedView::Angles {
        azimuth_degrees: 0.0,
        elevation_degrees: 90.0,
    };
    assert_eq!(
        vertical.up_hint(),
        Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        }
    );

    for view in cad_app::StandardView::ALL {
        let requested = RequestedView::Preset(view);
        assert_eq!(requested.direction(), view.eye_offset());
        assert_eq!(requested.up_hint(), view.up_hint());
        assert_eq!(requested.preset_key(), Some(view.key()));
    }
    assert_eq!(tilted.preset_key(), None);
}

/// The real contract: two different azimuths must produce different perspective
/// frames, and the 3D frame must differ from the unchanged 2D plan frame.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn oblique_views_render_a_real_angle_dependent_3d_projection() {
    // This test owns a device: see `gpu_test_guard`.
    let _gpu = gpu_test_guard();
    if !adapter_available() {
        return;
    }

    let controller = controller();
    let dir = plot_scratch_dir("render-3d");
    let render = |angles: Option<(f64, f64)>, preset: Option<cad_app::StandardView>, name: &str| {
        let mut invocation = invocation();
        if let Some((azimuth, elevation)) = angles {
            invocation.view_azimuth_degrees = Some(azimuth);
            invocation.view_elevation_degrees = Some(elevation);
        }
        invocation.view_preset = preset;
        let png = dir.join(name);
        invocation.png = Some(png.clone());
        let result = render_document(&invocation, &controller).expect("render frame");
        let bytes = std::fs::read(&png).expect("a frame-gated PNG was written");
        (result, bytes)
    };

    let (plan, plan_png) = render(None, None, "plan.png");
    let (azimuth_0, azimuth_0_png) = render(Some((0.0, 20.0)), None, "az0.png");
    let (azimuth_90, azimuth_90_png) = render(Some((90.0, 20.0)), None, "az90.png");
    let (isometric, isometric_png) =
        render(None, Some(cad_app::StandardView::Isometric), "iso.png");

    // 2D stays the plan path, with no 3D fields invented.
    assert_eq!(plan["camera"]["mode"], "2d");
    assert!(plan["camera"]["view"].is_null());
    assert!(plan["camera"]["fov_deg"].is_null());

    // Both angle frames and the preset frame report a 3D camera.
    for frame in [&azimuth_0, &azimuth_90, &isometric] {
        assert_eq!(frame["camera"]["mode"], "3d", "{frame}");
        assert_eq!(frame["camera"]["fov_deg"], 45.0, "{frame}");
        let eye = frame["camera"]["eye"].as_array().expect("eye array");
        let target = frame["camera"]["target"].as_array().expect("target array");
        assert_eq!(eye.len(), 3);
        assert_eq!(target.len(), 3);
        assert!(
            eye.iter()
                .chain(target)
                .all(|value| value.as_f64().is_some()),
            "camera points are finite numbers: {frame}"
        );
        assert!(
            frame["pixels"]["non_background"].as_u64().unwrap() > 0,
            "a 3D frame with geometry is not blank: {frame}"
        );
    }
    assert!(
        azimuth_0["camera"]["view"].is_null(),
        "angles have no preset name"
    );

    // The preset name and its exact direction reach the camera.
    assert_eq!(isometric["camera"]["view"], "isometric");
    let eye = isometric["camera"]["eye"].as_array().unwrap();
    let target = isometric["camera"]["target"].as_array().unwrap();
    let offset = [
        eye[0].as_f64().unwrap() - target[0].as_f64().unwrap(),
        eye[1].as_f64().unwrap() - target[1].as_f64().unwrap(),
        eye[2].as_f64().unwrap() - target[2].as_f64().unwrap(),
    ];
    let length = (offset[0] * offset[0] + offset[1] * offset[1] + offset[2] * offset[2]).sqrt();
    let unit = cad_app::StandardView::Isometric.eye_offset();
    assert!((offset[0] / length - unit.x).abs() < 1e-9);
    assert!((offset[1] / length - unit.y).abs() < 1e-9);
    assert!((offset[2] / length - unit.z).abs() < 1e-9);

    // The projection is genuinely 3D: rotating the eye changes the frame, and
    // no oblique frame coincides with the unchanged plan frame.
    assert_ne!(
        azimuth_0_png, azimuth_90_png,
        "azimuth must change the rendered 3D frame"
    );
    assert_ne!(
        plan_png, azimuth_0_png,
        "the 3D frame must differ from the 2D plan frame"
    );
    assert_ne!(
        plan_png, isometric_png,
        "the preset 3D frame must differ from the 2D plan frame"
    );

    // `--legacy-shading` is accepted on a 3D frame and must not change the
    // camera; only the PBR lighting path differs.
    let mut legacy = invocation();
    legacy.view_preset = Some(cad_app::StandardView::Isometric);
    legacy.legacy_shading = true;
    legacy.png = Some(dir.join("iso-legacy.png"));
    let legacy_frame = render_document(&legacy, &controller).expect("legacy-shading frame");
    assert_eq!(legacy_frame["camera"], isometric["camera"]);

    std::fs::remove_dir_all(dir).unwrap();
}
