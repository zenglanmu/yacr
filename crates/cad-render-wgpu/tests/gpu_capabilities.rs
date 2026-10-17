//! Read-only GPU capability probe: what does this machine's adapter actually
//! **support** for indirect / multi-draw?
//!
//! `Renderer::device_features()` only ever reports the **enabled** device
//! features, which is `Features::empty()` by construction (no feature is ever
//! requested when the device is created — see `crates/cad-render-wgpu/src/headless.rs`
//! and `src/native_presenter.rs`, both of which use `DeviceDescriptor::default()`).
//! That says nothing about what the adapter **supports**, which is the set the
//! next design decision depends on. This probe prints the *supported* set.
//!
//! # Read-only contract
//!
//! This test is a **capability query**, not a measurement:
//!
//! * it must **not** request any non-default device feature;
//! * it must **not** create a render pipeline;
//! * it must **not** change rendering behaviour;
//! * it must not launch the GUI or any host; it creates only `Instance`s and
//!   enumerates adapters, then requests one device with `DeviceDescriptor::default()`
//!   — exactly what the app already does today — purely to show that the
//!   *enabled* set stays empty while the *supported* set does not.
//!
//! Feature presence here is a capability fact only. It is **not** a performance
//! claim and no speedup is implied (AGENTS.md rule 6).
//!
//! # Adapter selection mirrored from the app
//!
//! Two selection rules exist in this repo, and both are reported:
//!
//! * the crate-wide headless rule (`headless::create_headless_gpu*`:
//!   `Backends::VULKAN | Backends::GL`, discrete-first inside the
//!   highest-priority backend) used by the CLI/engine paths;
//! * the desktop-host rule (`native_presenter::rank_probe_adapters`: a
//!   **Vulkan-only** instance, CPU adapters excluded, name preference
//!   `DEFAULT_PREFERRED_ADAPTER_NAMES` — currently `["P620"]` — then
//!   discrete > integrated > other), which is how the Linux host's CAD device is
//!   actually created.
//!
//! The host rule cannot be fully reproduced headlessly because it also requires
//! surface compatibility (`adapter.is_surface_supported`), which needs a real
//! surface; the surface check is therefore reported as *not evaluated* rather
//! than assumed true.
//!
//! # Running
//!
//! ```text
//! cargo test -p cad-render-wgpu --test gpu_capabilities -- --ignored --nocapture
//! ```
//!
//! Ignored by default because it is a hardware probe.

#![cfg(not(target_arch = "wasm32"))]

use cad_render_wgpu::native_presenter::DEFAULT_PREFERRED_ADAPTER_NAMES;
use cad_render_wgpu::wgpu;
use cad_render_wgpu::{headless, BackendPreference};
use std::collections::HashSet;

/// Backends the crate-wide headless path uses (`headless::backends_for(Auto)`).
fn headless_backends() -> wgpu::Backends {
    wgpu::Backends::VULKAN | wgpu::Backends::GL
}

/// Same instance construction as `headless::make_instance` /
/// `native_presenter::make_probe_instance` (no display handle, explicit backends).
fn make_instance(backends: wgpu::Backends) -> wgpu::Instance {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = backends;
    wgpu::Instance::new(descriptor)
}

fn enumerate(instance: &wgpu::Instance, backends: wgpu::Backends) -> Vec<wgpu::Adapter> {
    futures_lite::future::block_on(instance.enumerate_adapters(backends))
}

/// Stable `backend/name` key for cross-enumeration matching.
fn key(info: &wgpu::AdapterInfo) -> (String, String) {
    (info.backend.to_str().to_string(), info.name.clone())
}

/// Sort weight inside the Vulkan backend, mirroring
/// `native_presenter::device_rank` (discrete before integrated, others last).
fn device_rank(device_type: wgpu::DeviceType) -> u8 {
    match device_type {
        wgpu::DeviceType::DiscreteGpu => 0,
        wgpu::DeviceType::IntegratedGpu => 1,
        _ => 2,
    }
}

/// Position of the first preferred name contained in `name`, or `usize::MAX`;
/// mirrors `native_presenter::preferred_rank`.
fn preferred_rank(name: &str, preferred_names: &[&str]) -> usize {
    let lowered = name.to_ascii_lowercase();
    preferred_names
        .iter()
        .position(|candidate| {
            let needle = candidate.to_ascii_lowercase();
            !needle.is_empty() && lowered.contains(&needle)
        })
        .unwrap_or(usize::MAX)
}

/// The adapter the desktop host's presenter would pick: Vulkan-only, CPU
/// adapters excluded, surface compatibility **not evaluated** here (needs a real
/// surface), then `(preferred name, device rank, enumeration index)`.
///
/// Mirrors `native_presenter::rank_probe_adapters` minus the `surface_compatible`
/// filter; the caller prints that caveat.
fn app_presenter_choice(infos: &[wgpu::AdapterInfo]) -> Option<usize> {
    let mut eligible: Vec<usize> = infos
        .iter()
        .enumerate()
        .filter(|(_, info)| {
            info.backend == wgpu::Backend::Vulkan && info.device_type != wgpu::DeviceType::Cpu
        })
        .map(|(index, _)| index)
        .collect();
    eligible.sort_by_key(|&index| {
        let info = &infos[index];
        (
            preferred_rank(&info.name, DEFAULT_PREFERRED_ADAPTER_NAMES),
            device_rank(info.device_type),
            index,
        )
    });
    eligible.first().copied()
}

fn print_adapter_block(tag: &str, adapter: &wgpu::Adapter, vulkan_only_visible: bool) {
    let info = adapter.get_info();
    let downlevel = adapter.get_downlevel_capabilities();
    let features = adapter.features();
    let limits = adapter.limits();

    println!("==== {tag} ====");
    println!("  info.backend     = {}", info.backend.to_str());
    println!("  info.name        = {}", info.name);
    println!("  info.device_type = {:?}", info.device_type);
    println!("  info.driver      = {}", info.driver);
    println!("  info.driver_info = {}", info.driver_info);
    println!("  info.debug       = {info:?}");
    println!("  vulkan_only_instance_visible = {vulkan_only_visible}");

    println!("  -- downlevel --");
    println!("  downlevel.flags = {:?}", downlevel.flags);
    println!(
        "  downlevel.flags.INDIRECT_EXECUTION = {}",
        downlevel
            .flags
            .contains(wgpu::DownlevelFlags::INDIRECT_EXECUTION)
    );
    println!("  downlevel.limits = {:?}", downlevel.limits);
    println!("  downlevel.shader_model = {:?}", downlevel.shader_model);
    println!(
        "  downlevel.is_webgpu_compliant = {}",
        downlevel.is_webgpu_compliant()
    );
    println!("  downlevel.is_webgl = NOT AVAILABLE (wgpu-types 30.0.1 has no such query)");
    println!(
        "  backend_is_gl = {} (a native OpenGL/GLES backend through EGL, never WebGL)",
        info.backend == wgpu::Backend::Gl
    );
    println!(
        "  backend_is_webgl_class = false (native instance via \
         InstanceDescriptor::new_without_display_handle; WebGL/WebGL2 requires the browser canvas \
         path, and wgpu-types 30.0.1 exposes no is_webgl() query to ask otherwise)"
    );

    println!("  -- adapter-supported features (KEY LINE) --");
    println!("  adapter.features.bits = {:?}", features.bits());
    println!("  adapter.features.debug = {features:?}");
    let named: Vec<String> = features.iter().map(|flag| format!("{flag:?}")).collect();
    if named.is_empty() {
        println!("  adapter.features.named = [] (empty supported set)");
    } else {
        println!("  adapter.features.named = [{}]", named.join(", "));
    }

    println!("  -- named feature booleans --");
    // `MULTI_DRAW_INDIRECT` does not exist as a `Features` flag in wgpu 30.0.1
    // (checked against the pinned wgpu-types 30.0.1 source): `multi_draw_indirect`
    // is core API and is emulated with per-draw calls unless
    // MULTI_DRAW_INDIRECT_COUNT is present. Reported instead of guessed at.
    println!(
        "  feature.MULTI_DRAW_INDIRECT = NAME DOES NOT EXIST in wgpu 30.0.1 \
         (multi_draw_indirect is core; emulated unless MULTI_DRAW_INDIRECT_COUNT)"
    );
    println!(
        "  feature.MULTI_DRAW_INDIRECT_COUNT = {}",
        features.contains(wgpu::Features::MULTI_DRAW_INDIRECT_COUNT)
    );
    println!(
        "  feature.INDIRECT_FIRST_INSTANCE = {}",
        features.contains(wgpu::Features::INDIRECT_FIRST_INSTANCE)
    );
    println!(
        "  feature.TIMESTAMP_QUERY = {}",
        features.contains(wgpu::Features::TIMESTAMP_QUERY)
    );
    println!(
        "  feature.TIMESTAMP_QUERY_INSIDE_ENCODERS = {}",
        features.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)
    );
    println!(
        "  feature.TIMESTAMP_QUERY_INSIDE_PASSES = {}",
        features.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES)
    );

    println!("  -- limits (context; wgpu-types 30.0.1 has no indirect-specific limit) --");
    println!("  limits.max_buffer_size = {}", limits.max_buffer_size);
    println!(
        "  limits.max_storage_buffer_binding_size = {}",
        limits.max_storage_buffer_binding_size
    );
    println!(
        "  limits.max_storage_buffers_per_shader_stage = {}",
        limits.max_storage_buffers_per_shader_stage
    );
    println!("  limits.max_bind_groups = {}", limits.max_bind_groups);
    println!(
        "  limits.max_compute_workgroup_storage_size = {}",
        limits.max_compute_workgroup_storage_size
    );
}

#[test]
#[ignore = "hardware capability probe; read-only, run with --ignored --nocapture"]
fn gpu_capabilities_indirect_multi_draw() {
    println!("== wgpu capability probe (crate cad-render-wgpu) ==");
    println!("wgpu version pinned by the workspace: wgpu = \"=30.0.1\" (wgpu-types 30.0.1)");
    println!("feature names below were checked against that pinned version's source");

    // View 1: the crate-wide headless enumeration (Vulkan | GL), matching
    // `headless::enumerate_adapters(BackendPreference::Auto)`.
    let headless_instance = make_instance(headless_backends());
    let all = enumerate(&headless_instance, headless_backends());

    // View 2: the desktop host presenter enumeration (Vulkan only).
    let vulkan_instance = make_instance(wgpu::Backends::VULKAN);
    let vulkan_only = enumerate(&vulkan_instance, wgpu::Backends::VULKAN);
    let vulkan_keys: HashSet<(String, String)> = vulkan_only
        .iter()
        .map(|adapter| key(&adapter.get_info()))
        .collect();

    // Cross-check that the public headless helper sees the same adapters as the
    // private-exact enumeration done here (they must agree for this probe to be
    // representative of `headless::create_headless_gpu*`).
    let public_headless: Vec<(String, String)> =
        headless::enumerate_adapters(BackendPreference::Auto)
            .iter()
            .map(|info| (info.backend.clone(), info.name.clone()))
            .collect();
    let local_list: Vec<(String, String)> = all.iter().map(|a| key(&a.get_info())).collect();
    println!();
    println!("enumeration cross-check: headless::enumerate_adapters(Auto) = {public_headless:?}");
    println!("enumeration cross-check: local VULKAN|GL enumeration  = {local_list:?}");
    println!(
        "enumeration cross-check matches = {}",
        public_headless.len() == local_list.len()
            && public_headless
                .iter()
                .all(|entry| local_list.contains(entry))
    );

    if all.is_empty() {
        println!();
        println!(
            "PROBE RESULT: NO_ADAPTER_AVAILABLE — no wgpu adapter reachable from this process"
        );
        println!("(no GPU/display here; the probe is NOT RUN, not a passing capability check)");
        return;
    }

    let infos: Vec<wgpu::AdapterInfo> = all.iter().map(|adapter| adapter.get_info()).collect();
    let app_index = app_presenter_choice(&infos);

    println!();
    match app_index {
        Some(index) => println!(
            "APP CHOICE (desktop host presenter rule, Vulkan-only + non-CPU + name preference \
             {:?} + discrete-first): backend={} name={} device_type={:?} \
             [surface compatibility NOT evaluated headlessly]",
            DEFAULT_PREFERRED_ADAPTER_NAMES,
            infos[index].backend.to_str(),
            infos[index].name,
            infos[index].device_type,
        ),
        None => println!(
            "APP CHOICE (desktop host presenter rule): NONE — no Vulkan non-CPU adapter \
             enumerated (the host requires Vulkan)"
        ),
    }

    // Print the app-choice adapter first because its finding is the decision input.
    let mut order: Vec<usize> = (0..all.len()).collect();
    if let Some(index) = app_index {
        order.retain(|&i| i != index);
        order.insert(0, index);
    }

    for index in order {
        let adapter = &all[index];
        let info = &infos[index];
        let tag = if Some(index) == app_index {
            "adapter (APP CHOICE)".to_string()
        } else {
            format!("adapter [{index}]")
        };
        println!();
        print_adapter_block(&tag, adapter, vulkan_keys.contains(&key(info)));
    }

    // Show that the *default* device (what the app requests) enables no feature:
    // supported != enabled. Requested only for the app-choice adapter.
    let enabled_probe = app_index.or(Some(0));
    if let Some(index) = enabled_probe {
        println!();
        println!(
            "---- default-feature device request on APP CHOICE adapter ({} / {}) ----",
            infos[index].backend.to_str(),
            infos[index].name
        );
        match futures_lite::future::block_on(all[index].request_device(&wgpu::DeviceDescriptor {
            label: Some("cad-gpu-capabilities-probe"),
            ..Default::default()
        })) {
            Ok((device, _queue)) => {
                let enabled = device.features();
                println!("  device.enabled_features.bits = {:?}", enabled.bits());
                println!("  device.enabled_features.debug = {enabled:?}");
                println!(
                    "  device.enabled_features.is_empty = {} (matches Renderer::device_features() \
                     being empty by construction)",
                    enabled.is_empty()
                );
                println!(
                    "  contrast: adapter.adjusted_supported set is printed above; nothing here \
                     requests MULTI_DRAW_INDIRECT_COUNT"
                );
            }
            Err(error) => println!("  device request failed: {error}"),
        }
    }

    println!();
    println!("== SUMMARY (machine-specific, capability only; not a performance claim) ==");
    match app_index {
        Some(index) => {
            let adapter = &all[index];
            let features = adapter.features();
            let downlevel = adapter.get_downlevel_capabilities();
            println!(
                "summary.app_choice = {}/{} ({:?})",
                infos[index].backend.to_str(),
                infos[index].name,
                infos[index].device_type
            );
            println!(
                "summary.app_choice.MULTI_DRAW_INDIRECT_COUNT = {}",
                features.contains(wgpu::Features::MULTI_DRAW_INDIRECT_COUNT)
            );
            println!(
                "summary.app_choice.INDIRECT_EXECUTION = {}",
                downlevel
                    .flags
                    .contains(wgpu::DownlevelFlags::INDIRECT_EXECUTION)
            );
            println!(
                "summary.app_choice.INDIRECT_FIRST_INSTANCE = {}",
                features.contains(wgpu::Features::INDIRECT_FIRST_INSTANCE)
            );
            println!(
                "summary.app_choice.TIMESTAMP_QUERY = {}",
                features.contains(wgpu::Features::TIMESTAMP_QUERY)
            );
        }
        None => println!("summary.app_choice = NONE (no Vulkan non-CPU adapter enumerated)"),
    }
    println!(
        "summary.note = unemulated multi-draw is gated by MULTI_DRAW_INDIRECT_COUNT on the app-choice \
         adapter; INDIRECT_EXECUTION is the downlevel gate for any indirect call"
    );
}
