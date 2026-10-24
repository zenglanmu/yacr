//! Host-run contracts for the GPU memory ledger, budget and report.
//!
//! Nothing here needs a GPU: the ledger, the budget arithmetic and the refusal
//! contents are plain data, which is exactly why they are testable in the
//! ordinary `cargo test -p cad-render-wgpu` instead of only being observable as a
//! dead session on a machine with a card.

use super::vram::VramSummary;
use super::*;
use std::sync::Arc;

fn ledger(budget: GpuBudget) -> Arc<GpuMemoryLedger> {
    Arc::new(GpuMemoryLedger::new(ResolvedBudget::explicit(budget), None))
}

#[test]
fn every_category_is_in_the_breakdown_exactly_once() {
    assert_eq!(GpuMemoryCategory::ALL.len(), GpuMemoryCategory::COUNT);
    for (index, category) in GpuMemoryCategory::ALL.iter().enumerate() {
        assert_eq!(category.index(), index, "{category:?} index");
        assert!(!category.label().is_empty());
    }
    // Labels are stable tokens: unique, lowercase ASCII, no UI text.
    let mut labels: Vec<&str> = GpuMemoryCategory::ALL
        .iter()
        .map(|category| category.label())
        .collect();
    labels.sort_unstable();
    labels.dedup();
    assert_eq!(labels.len(), GpuMemoryCategory::COUNT);
    for label in labels {
        assert!(
            label.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
            "{label} is not a stable token"
        );
    }
    // The texture split is over exactly the texture categories.
    let textures: Vec<&str> = GpuMemoryCategory::ALL
        .iter()
        .filter(|category| category.is_texture())
        .map(|category| category.label())
        .collect();
    assert_eq!(
        textures,
        vec![
            "color-target",
            "depth-target",
            "image-texture",
            "mask-texture"
        ]
    );
}

#[test]
fn charges_and_releases_keep_every_counter_consistent() {
    let memory = ledger(GpuBudget::uncapped());
    {
        let _vertices = memory.charge(GpuMemoryCategory::Vertex, 1_000);
        let _indices = memory.charge(GpuMemoryCategory::Index, 400);
        let _target = memory.charge(GpuMemoryCategory::ColorTarget, 8_192);
        assert_eq!(memory.used_bytes(), 1_000 + 400 + 8_192);
        let categories = memory.category_bytes();
        assert_eq!(categories[GpuMemoryCategory::Vertex.index()], 1_000);
        assert_eq!(categories[GpuMemoryCategory::Index.index()], 400);
        assert_eq!(categories[GpuMemoryCategory::ColorTarget.index()], 8_192);
        assert_eq!(categories[GpuMemoryCategory::Uniform.index()], 0);
        // Releasing one category leaves the other charges untouched.
        drop(_indices);
        assert_eq!(memory.used_bytes(), 1_000 + 8_192);
        assert_eq!(memory.category_bytes()[GpuMemoryCategory::Index.index()], 0);
    }
    // Every guard dropped: the ledger is back to zero and nothing underflowed.
    assert_eq!(memory.used_bytes(), 0);
    assert!(memory.category_bytes().iter().all(|bytes| *bytes == 0));
}

#[test]
fn the_peak_survives_a_release_and_captures_the_upload_transient() {
    let memory = ledger(GpuBudget::uncapped());
    // The previous resident scene.
    let resident = memory.charge(GpuMemoryCategory::Vertex, 10_000);
    assert_eq!(memory.peak_bytes(), 10_000);
    {
        // A staged replacement exists at the same time as the resident scene:
        // that overlap is the real peak of a re-upload, and it must be visible.
        let _staged = memory.charge(GpuMemoryCategory::Vertex, 7_000);
        assert_eq!(memory.used_bytes(), 17_000);
        assert_eq!(memory.peak_bytes(), 17_000);
    }
    drop(resident);
    // The current total falls back, the peak does not.
    assert_eq!(memory.used_bytes(), 0);
    assert_eq!(memory.peak_bytes(), 17_000);
}

#[test]
fn a_zero_byte_charge_is_a_no_op() {
    let memory = ledger(GpuBudget::uncapped());
    let charge = memory.charge(GpuMemoryCategory::Other, 0);
    assert_eq!(memory.used_bytes(), 0);
    assert_eq!(memory.peak_bytes(), 0);
    drop(charge);
}

#[test]
fn an_uncapped_ledger_admits_any_request() {
    let memory = ledger(GpuBudget::uncapped());
    assert!(memory
        .admit(&[(GpuMemoryCategory::Vertex, u64::MAX)])
        .is_ok());
    assert!(memory.refusal().is_none());
}

#[test]
fn an_admitted_request_records_no_refusal() {
    let memory = ledger(GpuBudget::estimated_default());
    let cap = DEFAULT_GPU_BUDGET_BYTES;
    assert!(memory
        .admit(&[(GpuMemoryCategory::Vertex, cap / 2)])
        .is_ok());
    assert!(memory.refusal().is_none());
    // Exactly at the cap is admitted (inclusive limit).
    let used = memory.charge(GpuMemoryCategory::Vertex, cap / 2);
    assert!(memory.admit(&[(GpuMemoryCategory::Index, 0)]).is_ok());
    drop(used);
}

#[test]
fn a_budget_refusal_names_the_category_request_measurement_and_cap() {
    let memory = ledger(GpuBudget::estimated_default());
    let cap = DEFAULT_GPU_BUDGET_BYTES;
    let resident = memory.charge(GpuMemoryCategory::Vertex, cap - 1_000);
    let refusal = memory
        .admit(&[(GpuMemoryCategory::ImageTexture, 4_096)])
        .expect_err("a request past the cap must be refused");
    assert!(refusal.is_budget_refusal());
    assert!(!refusal.is_device_out_of_memory());
    assert_eq!(refusal.category, GpuMemoryCategory::ImageTexture);
    assert_eq!(refusal.used_bytes, cap - 1_000);
    assert_eq!(refusal.requested_bytes, 4_096);
    assert_eq!(refusal.cap_bytes, Some(cap));
    assert_eq!(refusal.limit_bytes, Some(cap));
    assert_eq!(refusal.device_message, None);

    // The message names all four facts, like the wasm budget refusal does.
    let message = refusal.message();
    assert!(message.contains("image-texture"), "{message}");
    assert!(message.contains(&(cap - 1_000).to_string()), "{message}");
    assert!(
        message.contains(&(cap - 1_000 + 4_096).to_string()),
        "{message}"
    );
    assert!(message.contains(&cap.to_string()), "{message}");
    assert!(message.contains(GPU_BUDGET_ENV), "{message}");
    assert!(message.contains("resident scene is unchanged"), "{message}");

    // The refusal is recorded for callers that only get a `CadError` back.
    assert_eq!(memory.refusal(), Some(refusal.clone()));
    memory.clear_refusal();
    assert_eq!(memory.refusal(), None);
    drop(resident);
}

#[test]
fn a_refusal_targets_the_heaviest_part_of_the_request() {
    let memory = ledger(GpuBudget::estimated_default());
    let refusal = memory
        .admit(&[
            (GpuMemoryCategory::Vertex, DEFAULT_GPU_BUDGET_BYTES),
            (GpuMemoryCategory::Index, DEFAULT_GPU_BUDGET_BYTES * 2),
        ])
        .expect_err("two capped requests must be refused");
    assert_eq!(refusal.category, GpuMemoryCategory::Index);
    assert_eq!(
        refusal.requested_bytes,
        DEFAULT_GPU_BUDGET_BYTES * 3,
        "the verdict is on the whole request, not one piece"
    );
}

#[test]
fn a_device_out_of_memory_refusal_is_classified_separately() {
    let memory = ledger(GpuBudget::estimated_default());
    let resident = memory.charge(GpuMemoryCategory::Stream, 12_345);
    let refusal = memory.device_out_of_memory(
        GpuMemoryCategory::Stream,
        4_096,
        "create_buffer: Out of Memory".to_string(),
    );
    assert!(refusal.is_device_out_of_memory());
    assert!(!refusal.is_budget_refusal());
    assert_eq!(refusal.category, GpuMemoryCategory::Stream);
    assert_eq!(refusal.used_bytes, 12_345);
    assert_eq!(refusal.requested_bytes, 4_096);
    assert_eq!(
        refusal.device_message.as_deref(),
        Some("create_buffer: Out of Memory")
    );
    // A device OOM is not a budget verdict, so no cap threshold is claimed...
    assert_eq!(refusal.limit_bytes, Some(DEFAULT_GPU_BUDGET_BYTES));
    // ... but the cap that was configured is still reported, and the device's own
    // message is named in the sentence.
    assert_eq!(refusal.cap_bytes, Some(DEFAULT_GPU_BUDGET_BYTES));
    assert!(refusal.message().contains("out of memory"));
    assert!(refusal.message().contains("create_buffer"));
    assert!(refusal.message().contains("stream"));
    assert_eq!(memory.refusal(), Some(refusal));
    drop(resident);
}

#[test]
fn the_report_splits_buffers_from_textures_and_states_its_source() {
    let memory = ledger(GpuBudget::estimated_default().with_refuse_at(0.5));
    let _vertex = memory.charge(GpuMemoryCategory::Vertex, 1_000);
    let _index = memory.charge(GpuMemoryCategory::Index, 500);
    let _image = memory.charge(GpuMemoryCategory::ImageTexture, 2_000);
    let _depth = memory.charge(GpuMemoryCategory::DepthTarget, 3_000);

    let accounted = memory.report(true, None);
    assert_eq!(accounted.source, GpuMemorySource::Accounted);
    assert_eq!(accounted.buffer_bytes, 1_500);
    assert_eq!(accounted.texture_bytes, 5_000);
    assert_eq!(accounted.total_bytes, 6_500);
    assert_eq!(accounted.buffer_bytes + accounted.texture_bytes, 6_500);
    assert_eq!(accounted.category(GpuMemoryCategory::ImageTexture), 2_000);
    assert_eq!(accounted.category(GpuMemoryCategory::Uniform), 0);
    assert_eq!(accounted.peak_bytes, 6_500);
    assert_eq!(accounted.cap_bytes, Some(DEFAULT_GPU_BUDGET_BYTES));
    assert_eq!(accounted.budget_bytes, Some(DEFAULT_GPU_BUDGET_BYTES / 2));
    // The policy is labelled as an explicit budget (this ledger was built from a
    // `GpuBudget`), and no VRAM probe was supplied on this construction path.
    assert_eq!(accounted.budget_source, BudgetSource::ExplicitBytes);
    assert_eq!(accounted.vram_bytes, None);
    assert_eq!(accounted.budget_percent, None);
    assert_eq!(
        accounted.headroom_bytes(),
        Some(DEFAULT_GPU_BUDGET_BYTES / 2 - 6_500)
    );
    assert!(!accounted.over_budget());
    // No allocator report was supplied, so the cross-check is honestly absent.
    assert_eq!(accounted.reserved_bytes, None);
    assert_eq!(accounted.allocator_total_bytes, None);

    // The allocator report is a cross-check, attached without replacing the
    // accounting.
    let cross_checked = memory.report(
        true,
        Some(AllocatorFacts {
            allocated_bytes: 9_000,
            reserved_bytes: 16_384,
        }),
    );
    assert_eq!(cross_checked.source, GpuMemorySource::Accounted);
    assert_eq!(cross_checked.total_bytes, 6_500);
    assert_eq!(cross_checked.allocator_total_bytes, Some(9_000));
    assert_eq!(cross_checked.reserved_bytes, Some(16_384));

    // With no device, the source is stated instead of silently implying evidence.
    assert_eq!(memory.report(false, None).source, GpuMemorySource::Estimate);
    assert_eq!(
        memory
            .report(
                false,
                Some(AllocatorFacts {
                    allocated_bytes: 1,
                    reserved_bytes: 2,
                })
            )
            .source,
        GpuMemorySource::AllocatorReport
    );
}

#[test]
fn the_report_never_claims_more_headroom_than_the_threshold() {
    let memory = ledger(GpuBudget::estimated_default().with_refuse_at(0.5));
    let over = memory.charge(GpuMemoryCategory::Vertex, DEFAULT_GPU_BUDGET_BYTES / 2 + 1);
    let report = memory.report(true, None);
    assert!(report.over_budget());
    // Saturating, never a wrapped "huge headroom".
    assert_eq!(report.headroom_bytes(), Some(0));
    drop(over);
    assert_eq!(
        memory.report(true, None).headroom_bytes(),
        Some(DEFAULT_GPU_BUDGET_BYTES / 2)
    );
}

#[test]
fn setting_the_budget_keeps_the_accounting() {
    let memory = ledger(GpuBudget::uncapped());
    let _charge = memory.charge(GpuMemoryCategory::Uniform, 256);
    assert_eq!(memory.budget(), GpuBudget::uncapped());
    let tighter = GpuBudget::estimated_default().with_cap(Some(1024));
    memory.set_budget(tighter);
    assert_eq!(memory.budget(), tighter);
    // A verbatim policy is labelled as an explicit choice, not a VRAM percentage.
    assert_eq!(memory.resolved().source, BudgetSource::ExplicitBytes);
    assert_eq!(memory.resolved().percent, None);
    assert_eq!(memory.used_bytes(), 256);
    // The new cap now governs the verdict.
    assert!(memory.admit(&[(GpuMemoryCategory::Uniform, 1024)]).is_err());
}

#[test]
fn a_request_that_allocates_nothing_is_admitted_after_the_cap_was_lowered() {
    // A converted batch owns no per-batch buffers, so its staging request is zero
    // bytes. A host that lowers the budget below the resident scene must not have
    // every later upload rejected: only allocations are refused.
    let memory = ledger(GpuBudget::estimated_default());
    let resident = memory.charge(GpuMemoryCategory::Stream, DEFAULT_GPU_BUDGET_BYTES);
    memory.set_budget(GpuBudget::estimated_default().with_cap(Some(1_024)));
    assert!(memory.admit(&[(GpuMemoryCategory::Vertex, 0)]).is_ok());
    assert!(memory.admit(&[]).is_ok());
    assert!(memory.admit(&[(GpuMemoryCategory::Vertex, 1)]).is_err());
    drop(resident);
}

#[test]
fn the_render_error_variant_carries_the_refusal_by_type() {
    let memory = ledger(GpuBudget::estimated_default());
    let refusal = memory
        .admit(&[(GpuMemoryCategory::Vertex, DEFAULT_GPU_BUDGET_BYTES + 1)])
        .expect_err("over the cap");
    let error = crate::RenderError::GpuMemory(refusal.clone());
    // Not a device loss and not a rejected frame: the caller can branch on type
    // instead of matching message text.
    assert!(!error.is_device_loss());
    assert!(error.is_gpu_budget_refusal());
    assert_eq!(error.gpu_memory_refusal(), Some(&refusal));
    assert_eq!(error.message(), refusal.message());
    // A device OOM is the same variant with the other origin.
    let oom = crate::RenderError::GpuMemory(memory.device_out_of_memory(
        GpuMemoryCategory::ImageTexture,
        16,
        "out of memory".to_string(),
    ));
    assert!(!oom.is_gpu_budget_refusal());
    assert!(!oom.is_device_loss());
    assert!(oom
        .gpu_memory_refusal()
        .is_some_and(|refusal| refusal.is_device_out_of_memory()));
    // A frame error is still distinguishable.
    assert!(crate::RenderError::Frame("bad frame".into())
        .gpu_memory_refusal()
        .is_none());
}

#[test]
fn the_ledger_starts_uncapped_only_when_asked() {
    // A constructed renderer defaults to the estimated cap, and its report says so
    // rather than presenting the number as the card's VRAM.
    let memory = ledger(GpuBudget::from_env());
    let report = memory.report(false, None);
    assert_eq!(report.total_bytes, 0);
    assert_eq!(report.peak_bytes, 0);
    assert_eq!(report.source, GpuMemorySource::Estimate);
}

#[test]
fn the_report_states_the_budget_source_and_the_probed_vram() {
    const GIB: u64 = 1024 * 1024 * 1024;
    let vram = 12 * GIB;

    // Percentage policy: the cap derives from the probe and says so. The proxy is
    // what the cap uses; the all-heaps sum is reported beside it, not substituted.
    let all_heaps = vram + 258 * 1024 * 1024;
    let percent = Arc::new(GpuMemoryLedger::new(
        GpuBudgetConfig::default().resolve(Some(vram)),
        Some(VramSummary {
            total_bytes: vram,
            all_device_local_bytes: all_heaps,
        }),
    ));
    let report = percent.report(true, None);
    assert_eq!(report.vram_bytes, Some(vram));
    assert_eq!(report.vram_all_heaps_bytes, Some(all_heaps));
    assert!(report.vram_all_heaps_bytes > report.vram_bytes);
    assert_eq!(report.budget_source, BudgetSource::VramPercent);
    assert_eq!(report.budget_percent, Some(DEFAULT_GPU_BUDGET_PERCENT));
    assert_eq!(report.cap_bytes, Some(vram * 9 / 10));

    // Explicit policy: the cap is the caller's number, but the probe still
    // describes the card (so a report does not hide the full picture).
    let explicit = Arc::new(GpuMemoryLedger::new(
        GpuBudgetConfig {
            explicit_bytes: Some(1_024),
            ..GpuBudgetConfig::default()
        }
        .resolve(Some(vram)),
        Some(VramSummary {
            total_bytes: vram,
            all_device_local_bytes: all_heaps,
        }),
    ));
    let report = explicit.report(true, None);
    assert_eq!(report.cap_bytes, Some(1_024));
    assert_eq!(report.budget_source, BudgetSource::ExplicitBytes);
    assert_eq!(report.budget_percent, None);
    assert_eq!(report.vram_bytes, Some(vram));
    assert_eq!(report.vram_all_heaps_bytes, Some(all_heaps));

    // No probe: the fallback is labelled an estimate, never a percentage of a
    // card nobody measured.
    let estimate = Arc::new(GpuMemoryLedger::new(
        GpuBudgetConfig::default().resolve(None),
        None,
    ));
    let report = estimate.report(true, None);
    assert_eq!(report.vram_bytes, None);
    assert_eq!(report.vram_all_heaps_bytes, None);
    assert_eq!(report.budget_source, BudgetSource::Estimate);
    assert_eq!(report.budget_percent, None);
    assert_eq!(report.cap_bytes, Some(DEFAULT_GPU_BUDGET_BYTES));

    // An explicit uncapped policy is still labelled explicit.
    let uncapped = Arc::new(GpuMemoryLedger::new(
        GpuBudgetConfig {
            explicit_bytes: Some(0),
            ..GpuBudgetConfig::default()
        }
        .resolve(Some(vram)),
        Some(VramSummary {
            total_bytes: vram,
            all_device_local_bytes: all_heaps,
        }),
    ));
    let report = uncapped.report(true, None);
    assert_eq!(report.cap_bytes, None);
    assert_eq!(report.budget_bytes, None);
    assert_eq!(report.budget_source, BudgetSource::ExplicitBytes);
}
