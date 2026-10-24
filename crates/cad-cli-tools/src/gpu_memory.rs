//! GPU-memory reporting, the `--gpu-budget-mb` policy override, and the
//! refusal exit-code contract for the native `render`/`plot` paths.
//!
//! # What this module does *not* do
//!
//! It never measures, estimates or re-derives GPU memory: every number it prints
//! is a field of [`GpuMemoryReport`] as returned by
//! [`Renderer::gpu_memory_report`](cad_render_wgpu::Renderer::gpu_memory_report),
//! and every refusal field is a field of
//! [`GpuMemoryRefusal`](cad_render_wgpu::GpuMemoryRefusal) as reported by the
//! renderer. This module only **names** things (`source=accounted`,
//! `origin=application-budget`, `none`) and decides the process exit code.
//!
//! # Where the lines go, and why
//!
//! Both the successful `gpu-memory:` line and the `gpu-memory-refused:` line go
//! to **stderr**, because this CLI's machine contract is that stdout carries
//! exactly the JSON result document (see the crate docs). The lines are stable
//! and machine-readable: fixed field order, `none` for an absent `Option<u64>`,
//! and no locale-dependent text.
//!
//! # The refusal contract
//!
//! A budget or device out-of-memory refusal is **never** a success and never
//! leaves a blank PNG behind:
//!
//! * the operation fails with the machine code
//!   [`error_code::GPU_MEMORY_REFUSED`](crate::error_code::GPU_MEMORY_REFUSED);
//! * the process exits [`EXIT_GPU_MEMORY_REFUSED`](crate::EXIT_GPU_MEMORY_REFUSED)
//!   (7), distinct from usage (2) and generic failure (1);
//! * exactly one `gpu-memory-refused:` line names the origin, the category, the
//!   accounting at the refusal, the request and the cap;
//! * the PNG writer is gated on a read-back frame, so a refused frame writes no
//!   file at all.
//!
//! A [`GpuMemoryRefusal`](cad_render_wgpu::GpuMemoryRefusal) has no public
//! constructor, so the CLI carries its fields in [`GpuMemoryRefusalFacts`] —
//! a plain copy, which is also what makes the exit-code mapping testable on a
//! host with no GPU adapter.

use cad_domain::CadError;
use cad_render_wgpu::{
    GpuBudget, GpuMemoryCategory, GpuMemoryRefusal, GpuMemoryReport, GpuMemorySource, RenderError,
    Renderer,
};

use crate::{error_code, invocation::GpuBudgetFlag, CliError, CliInvocation};

/// The stable token for a refusal decided by the renderer's own application
/// budget (`GpuMemoryRefusalOrigin::ApplicationBudget`).
pub(crate) const ORIGIN_APPLICATION_BUDGET: &str = "application-budget";

/// The stable token for a refusal decided by the **device** reporting
/// out-of-memory (`GpuMemoryRefusalOrigin::DeviceOutOfMemory`).
pub(crate) const ORIGIN_DEVICE_OUT_OF_MEMORY: &str = "device-out-of-memory";

/// Printed after the report line when the resolved policy carries **no** cap
/// (`--gpu-budget-mb 0` or `YACR_GPU_BUDGET_MB=0`).
///
/// The absence is stated explicitly: `cap=none` alone could read as "unknown",
/// while this line says the cap was deliberately switched off, so nothing but
/// the device or the driver can refuse an allocation.
pub(crate) const GPU_BUDGET_UNCAPPED_LINE: &str =
    "gpu-memory-budget: uncapped (--gpu-budget-mb 0 / \
     YACR_GPU_BUDGET_MB=0): the application budget is explicitly disabled; only the device or \
     the driver can refuse an allocation";

/// `Option<u64>` as the report line spells it: the number, or `none`.
fn bytes_or_none(bytes: Option<u64>) -> String {
    match bytes {
        Some(bytes) => bytes.to_string(),
        None => "none".to_string(),
    }
}

/// The stable token for [`GpuMemorySource`], so the line never depends on
/// `Debug` formatting.
fn source_token(source: GpuMemorySource) -> &'static str {
    match source {
        GpuMemorySource::Accounted => "accounted",
        GpuMemorySource::AllocatorReport => "allocator-report",
        GpuMemorySource::Estimate => "estimate",
    }
}

/// The stable token for a refusal origin.
fn origin_token(refusal: &GpuMemoryRefusal) -> &'static str {
    if refusal.is_device_out_of_memory() {
        ORIGIN_DEVICE_OUT_OF_MEMORY
    } else {
        // `is_budget_refusal()` is the positive test for the only other origin.
        ORIGIN_APPLICATION_BUDGET
    }
}

/// One line, fixed field order, every number read straight off `report`.
///
/// `budget` is the effective threshold the budget gate compares against
/// (`cap × refuse_at`) and `cap` is the configured cap; both are the renderer's
/// **application estimate**, never the card's VRAM. `none` means the field is
/// absent (an uncapped budget, or a backend without an allocator report).
pub(crate) fn gpu_memory_line(report: &GpuMemoryReport) -> String {
    format!(
        "gpu-memory: total={} buffer={} texture={} peak={} budget={} cap={} source={} \
         allocator_total={} reserved={}",
        report.total_bytes,
        report.buffer_bytes,
        report.texture_bytes,
        report.peak_bytes,
        bytes_or_none(report.budget_bytes),
        bytes_or_none(report.cap_bytes),
        source_token(report.source),
        bytes_or_none(report.allocator_total_bytes),
        bytes_or_none(report.reserved_bytes),
    )
}

/// The per-category breakdown (`--gpu-memory-detail`), in
/// [`GpuMemoryCategory::ALL`] order so the field order is stable.
///
/// Zero-byte categories are printed too: a category silently vanishing from the
/// breakdown would look like it is not accounted for at all.
pub(crate) fn gpu_memory_detail_line(report: &GpuMemoryReport) -> String {
    use std::fmt::Write as _;

    let mut line = String::from("gpu-memory-detail:");
    for category in GpuMemoryCategory::ALL {
        // Writing into a `String` cannot fail; the result is ignored on purpose.
        let _ = write!(line, " {}={}", category.label(), report.category(category));
    }
    line
}

/// The structured fields of one refusal, as the CLI reports them.
///
/// A plain copy of the renderer's refusal: nothing is inferred, and no field is
/// computed here (the threshold arithmetic lives in `cad-render-wgpu`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GpuMemoryRefusalFacts<'a> {
    /// `application-budget` or `device-out-of-memory`.
    pub origin: &'static str,
    /// The refused allocation's owner class (`vertex`, `index`, …).
    pub category: &'static str,
    /// Renderer-accounted device bytes at the moment of the refusal.
    pub used_bytes: u64,
    /// Bytes the refused request would have added.
    pub requested_bytes: u64,
    /// The configured application cap, when one is configured.
    pub cap_bytes: Option<u64>,
    /// The threshold the verdict was taken against (`cap × refuse_at`).
    pub limit_bytes: Option<u64>,
    /// The device's own message, for a device out-of-memory refusal only.
    pub device_message: Option<&'a str>,
}

impl<'a> GpuMemoryRefusalFacts<'a> {
    /// Copy the fields of the renderer's refusal.
    pub(crate) fn from_refusal(refusal: &'a GpuMemoryRefusal) -> Self {
        Self {
            origin: origin_token(refusal),
            category: refusal.category.label(),
            used_bytes: refusal.used_bytes,
            requested_bytes: refusal.requested_bytes,
            cap_bytes: refusal.cap_bytes,
            limit_bytes: refusal.limit_bytes,
            device_message: refusal.device_message.as_deref(),
        }
    }
}

/// The one stable refusal line: origin, category, used, requested, cap, limit.
///
/// The device's own message is **not** in the line (it is free text that would
/// make the line unstable); it travels in the structured error context.
pub(crate) fn gpu_memory_refusal_line(facts: &GpuMemoryRefusalFacts<'_>) -> String {
    format!(
        "gpu-memory-refused: origin={} category={} used={} requested={} cap={} limit={}",
        facts.origin,
        facts.category,
        facts.used_bytes,
        facts.requested_bytes,
        bytes_or_none(facts.cap_bytes),
        bytes_or_none(facts.limit_bytes),
    )
}

/// The structured CLI error for a refusal: code, exit 7, and the facts as
/// machine context (the human message repeats the origin, category and numbers).
pub(crate) fn gpu_memory_refusal_error(facts: &GpuMemoryRefusalFacts<'_>) -> CliError {
    let message = format!(
        "GPU memory refused ({}): a {}-byte {} allocation on top of {}-byte used exceeds the {} \
         cap (limit {}); nothing was allocated, the resident scene is unchanged and no PNG was \
         written",
        facts.origin,
        facts.requested_bytes,
        facts.category,
        facts.used_bytes,
        bytes_or_none(facts.cap_bytes),
        bytes_or_none(facts.limit_bytes),
    );
    CliError::new(error_code::GPU_MEMORY_REFUSED, message).with_context(serde_json::json!({
        "origin": facts.origin,
        "category": facts.category,
        "used_bytes": facts.used_bytes,
        "requested_bytes": facts.requested_bytes,
        "cap_bytes": facts.cap_bytes,
        "limit_bytes": facts.limit_bytes,
        "device_message": facts.device_message,
    }))
}

/// Map one refusal to its CLI error, printing the single stable line first.
///
/// The line is printed here because this is the only place that holds the
/// structured facts; the generic error reporter only sees code, message and
/// context, and re-deriving the line from those would be a second source of
/// truth.
fn refusal_error(refusal: &GpuMemoryRefusal) -> CliError {
    let facts = GpuMemoryRefusalFacts::from_refusal(refusal);
    eprintln!("{}", gpu_memory_refusal_line(&facts));
    gpu_memory_refusal_error(&facts)
}

/// Map a renderer failure, keeping a GPU-memory refusal distinct (exit 7).
///
/// Every other variant stays a `gpu_failure` (exit 1), exactly as before.
pub(crate) fn render_error(error: RenderError) -> CliError {
    match error.gpu_memory_refusal() {
        Some(refusal) => refusal_error(refusal),
        None => CliError::new(error_code::GPU_FAILURE, error.message().to_string()),
    }
}

/// Map a staging failure from the `CadResult` path.
///
/// `Renderer::upload`/`upload_images` cannot widen `CadError`, so the crate
/// reports the same refusal on
/// [`Renderer::gpu_budget_refusal`](cad_render_wgpu::Renderer::gpu_budget_refusal)
/// instead of in the error value. The renderer clears that record when a staging
/// attempt succeeds, so at this point it belongs to the attempt that just failed;
/// the error-kind guard additionally keeps a non-memory failure (an invariant, a
/// device loss) from being relabelled as a memory refusal.
pub(crate) fn staging_error(renderer: &Renderer, error: CadError) -> CliError {
    match (&error, renderer.gpu_budget_refusal()) {
        (CadError::GpuFailure(_), Some(refusal)) => refusal_error(&refusal),
        _ => CliError::from(error),
    }
}

/// Print the successful-run GPU memory report to stderr.
///
/// Called only after a frame was actually read back; a refused or failed render
/// prints the refusal line instead (see [`refusal_error`]).
pub(crate) fn print_gpu_memory_report(renderer: &Renderer, invocation: &CliInvocation) {
    let report = renderer.gpu_memory_report();
    eprintln!("{}", gpu_memory_line(&report));
    if invocation.gpu_memory_detail {
        eprintln!("{}", gpu_memory_detail_line(&report));
    }
    if report.cap_bytes.is_none() {
        eprintln!("{GPU_BUDGET_UNCAPPED_LINE}");
    }
}

/// Resolve the budget the renderer must enforce: **flag > env > default**.
///
/// `base` is the policy the renderer already resolved when it was constructed
/// ([`GpuBudget::from_env`](cad_render_wgpu::GpuBudget::from_env): the
/// `YACR_GPU_BUDGET_MB` override, else the 512 MiB default estimate), so the env
/// override is parsed exactly once, by the renderer's own rules. An explicit
/// `--gpu-budget-mb` replaces **only the cap** — `refuse_at` stays whatever the
/// base policy says — and `--gpu-budget-mb 0` replaces it with an explicit
/// "uncapped".
pub(crate) fn apply_gpu_budget_flag(base: GpuBudget, flag: GpuBudgetFlag) -> GpuBudget {
    match flag {
        GpuBudgetFlag::Absent => base,
        GpuBudgetFlag::Uncapped => base.with_cap(None),
        GpuBudgetFlag::Capped(bytes) => base.with_cap(Some(bytes)),
    }
}

/// Apply the resolved policy to a freshly constructed renderer.
pub(crate) fn apply_resolved_gpu_budget(renderer: &mut Renderer, invocation: &CliInvocation) {
    let budget = apply_gpu_budget_flag(renderer.gpu_budget(), invocation.gpu_budget_flag);
    renderer.set_gpu_budget(budget);
}
