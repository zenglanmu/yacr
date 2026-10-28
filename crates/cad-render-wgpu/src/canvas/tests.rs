//! Pure unit tests for the canvas presenter policy: no GPU, no browser, no wgpu
//! object is ever created here. The GPU-touching paths ([`CanvasPresenter`](super::CanvasPresenter))
//! cannot be exercised without a WebView and a WebGL2 context, so they have no
//! test in this crate at all rather than an ignored one that pretends to exist.

use super::policy::{
    check_physical_size, check_present_format, check_present_size, check_present_usage,
    preferred_present_format, CanvasError, CanvasTargetKind, SurfaceStatus,
};
use crate::wgpu;

const ALL_STATUSES: [SurfaceStatus; 7] = [
    SurfaceStatus::Success,
    SurfaceStatus::Suboptimal,
    SurfaceStatus::Timeout,
    SurfaceStatus::Occluded,
    SurfaceStatus::Outdated,
    SurfaceStatus::Lost,
    SurfaceStatus::Validation,
];

#[test]
fn zero_physical_size_is_rejected_instead_of_panicking() {
    for size in [(0, 0), (0, 720), (1280, 0)] {
        let error = check_physical_size(size, 8192).expect_err("zero dimension must be rejected");
        match error {
            CanvasError::InvalidSize {
                width,
                height,
                detail,
            } => {
                assert_eq!((width, height), size);
                assert!(
                    detail.contains("devicePixelRatio"),
                    "the message must state the physical-pixel contract: {detail}"
                );
            }
            other => panic!("expected InvalidSize, got {other:?}"),
        }
    }
}

#[test]
fn oversized_physical_size_is_rejected_with_the_adapter_limit_named() {
    let error = check_physical_size((4096, 3000), 2048).expect_err("oversized must be rejected");
    let message = error.to_string();
    assert!(
        message.contains("max_texture_dimension_2d (2048)"),
        "the message must name the limit: {message}"
    );
    assert!(
        message.contains("4096x3000"),
        "the message must name the size: {message}"
    );
}

#[test]
fn physical_size_at_the_limit_is_accepted() {
    assert_eq!(check_physical_size((2048, 2048), 2048), Ok((2048, 2048)));
    assert_eq!(check_physical_size((1, 1), 1), Ok((1, 1)));
}

#[test]
fn present_format_prefers_a_linear_surface_and_never_assumes_bgra() {
    // The real WebGL2 capability order: linear RGBA first, sRGB second.
    let webgl2 = [
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Rgba8UnormSrgb,
    ];
    assert_eq!(
        preferred_present_format(&webgl2),
        Some(wgpu::TextureFormat::Rgba8Unorm),
        "a linear surface is required: only that present path is a plain \
         blitFramebuffer copy, and the blit stage owns the sRGB encode"
    );
    // An sRGB-only surface list still resolves (to the linear sibling when it is
    // advertised, otherwise to what is offered).
    assert_eq!(
        preferred_present_format(&[wgpu::TextureFormat::Rgba8UnormSrgb]),
        Some(wgpu::TextureFormat::Rgba8UnormSrgb)
    );
    assert_eq!(
        preferred_present_format(&[
            wgpu::TextureFormat::Bgra8UnormSrgb,
            wgpu::TextureFormat::Bgra8Unorm,
        ]),
        Some(wgpu::TextureFormat::Bgra8Unorm),
        "the linear sibling is picked whatever the channel order is"
    );
    // No sRGB offered: the first advertised format is used, whatever it is.
    assert_eq!(
        preferred_present_format(&[wgpu::TextureFormat::Bgra8Unorm]),
        Some(wgpu::TextureFormat::Bgra8Unorm)
    );
    assert_eq!(preferred_present_format(&[]), None);
}

#[test]
fn acquire_status_frame_and_reconfigure_tables_are_exact() {
    for status in ALL_STATUSES {
        assert_eq!(
            status.is_frame(),
            matches!(status, SurfaceStatus::Success | SurfaceStatus::Suboptimal),
            "{status:?} frame classification"
        );
        assert_eq!(
            status.needs_reconfigure(),
            matches!(status, SurfaceStatus::Outdated | SurfaceStatus::Lost),
            "{status:?} reconfigure classification"
        );
    }
}

#[test]
fn every_non_frame_status_maps_to_its_own_error_naming_the_step() {
    let mut messages = Vec::new();
    for status in ALL_STATUSES {
        let error = status.acquire_error();
        if status.is_frame() {
            assert!(error.is_none(), "{status:?} carries a frame, not an error");
            continue;
        }
        let error = error.expect("a non-frame status must be reported");
        let message = error.to_string();
        assert!(
            message.starts_with("canvas: get_current_texture"),
            "{status:?} must name the failed step: {message}"
        );
        messages.push(message);
    }
    messages.sort();
    let unique = messages.len();
    messages.dedup();
    assert_eq!(
        messages.len(),
        unique,
        "each status needs a distinguishable message: {messages:?}"
    );
}

#[test]
fn outdated_and_lost_errors_record_the_reconfigure() {
    let outdated = SurfaceStatus::Outdated
        .acquire_error()
        .expect("Outdated is an error");
    assert_eq!(outdated, CanvasError::SurfaceOutdated);
    assert!(outdated.to_string().contains("Outdated"));
    assert!(
        outdated.to_string().contains("reconfigured once"),
        "the message must state that a reconfigure was issued: {outdated}"
    );

    let lost = SurfaceStatus::Lost
        .acquire_error()
        .expect("Lost is an error");
    assert_eq!(lost, CanvasError::SurfaceLost);
    assert!(lost.to_string().contains("Lost"));
}

#[test]
fn setup_errors_name_the_failed_call() {
    let cases = [
        (
            CanvasError::AdapterUnavailable("no adapter".into()),
            "request_adapter",
        ),
        (
            CanvasError::DeviceUnavailable("no device".into()),
            "request_device",
        ),
        (
            CanvasError::SurfaceCreate("refused".into()),
            "create_surface",
        ),
        (CanvasError::NoSurfaceConfig, "get_default_config"),
        (CanvasError::SurfaceValidation("bad".into()), "validation"),
        (CanvasError::Blit("bad".into()), "blit"),
        (
            CanvasError::InvalidSize {
                width: 0,
                height: 0,
                detail: "zero".into(),
            },
            "invalid physical size",
        ),
    ];
    for (error, step) in cases {
        let message = error.to_string();
        assert!(
            message.contains(step),
            "{error:?} must name `{step}`: {message}"
        );
    }
}

#[test]
fn present_size_mismatch_is_refused_and_names_both_sizes() {
    assert_eq!(check_present_size((100, 50), (100, 50)), Ok(()));
    let error = check_present_size((100, 50), (128, 72)).expect_err("a mismatch must be refused");
    let message = error.to_string();
    assert!(message.contains("100x50"), "source size missing: {message}");
    assert!(
        message.contains("128x72"),
        "surface size missing: {message}"
    );
    assert!(message.contains("stretch"), "reason missing: {message}");
}

#[test]
fn present_format_accepts_the_srgb_pair_and_refuses_a_decode() {
    // The renderer's frame is sRGB; both surface kinds it can be configured with
    // present the same bytes, so both are accepted.
    assert_eq!(
        check_present_format(
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureFormat::Rgba8UnormSrgb
        ),
        Ok(()),
        "an sRGB surface is the hardware-encoded path"
    );
    assert_eq!(
        check_present_format(
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureFormat::Rgba8Unorm
        ),
        Ok(()),
        "the linear sibling is the shader-encoded path"
    );

    // A different base format would need a channel swizzle the blit does not do.
    for (source, surface) in [
        (
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureFormat::Bgra8Unorm,
        ),
        // A linear source into an sRGB surface would need a *decode*.
        (
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        ),
    ] {
        let error = check_present_format(source, surface)
            .expect_err("an unsupported pairing must be refused");
        let message = error.to_string();
        assert!(message.contains(&format!("{source:?}")), "{message}");
        assert!(message.contains(&format!("{surface:?}")), "{message}");
        assert!(message.contains("colour shift"), "{message}");
    }
}

#[test]
fn present_usage_requires_texture_binding() {
    assert_eq!(
        check_present_usage(wgpu::TextureUsages::TEXTURE_BINDING),
        Ok(())
    );
    assert_eq!(
        check_present_usage(
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING
        ),
        Ok(())
    );
    let error = check_present_usage(wgpu::TextureUsages::RENDER_ATTACHMENT)
        .expect_err("a non-sampleable frame must be refused");
    assert!(error.to_string().contains("TEXTURE_BINDING"));
}

#[test]
fn canvas_target_kind_tokens_are_stable_and_distinct() {
    assert_eq!(CanvasTargetKind::HtmlCanvas.as_str(), "html-canvas");
    assert_eq!(
        CanvasTargetKind::OffscreenCanvas.as_str(),
        "offscreen-canvas"
    );
    // The tokens are a host-visible vocabulary (`target=` in a status line, the
    // `create_surface(..)` diagnostic): they must never collide or be empty.
    let mut tokens: Vec<&str> = CanvasTargetKind::ALL
        .iter()
        .map(|kind| kind.as_str())
        .collect();
    let count = tokens.len();
    tokens.sort_unstable();
    tokens.dedup();
    assert_eq!(
        tokens.len(),
        count,
        "target tokens must be unique: {tokens:?}"
    );
    for kind in CanvasTargetKind::ALL {
        let token = kind.as_str();
        assert!(!token.is_empty(), "{kind:?} has no token");
        assert_eq!(
            token,
            kind.to_string(),
            "{kind:?} Display must equal its status token"
        );
    }
}

#[test]
fn only_the_offscreen_target_reports_a_worker_isolation() {
    // This mapping is the *only* source of the `isolation` token a host reports: an
    // `OffscreenCanvas` reaches a wasm module only through a Worker, a page
    // `<canvas>` only through the main thread. A regression here would let a
    // main-thread session claim `isolation: worker`.
    for kind in CanvasTargetKind::ALL {
        assert_eq!(
            kind.is_worker(),
            matches!(kind, CanvasTargetKind::OffscreenCanvas),
            "{kind:?} isolation classification"
        );
    }
    assert!(!CanvasTargetKind::HtmlCanvas.is_worker());
    assert!(CanvasTargetKind::OffscreenCanvas.is_worker());
}
