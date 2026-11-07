//! Host performance sampling for the performance/diagnostics panel.
//!
//! [`PerformanceState`] is the shell half of the `cad_status.performance` block:
//! it samples the process and the machine on a bounded cadence and stores the
//! renderer's own GPU accounting reported by the concrete host. It holds no
//! renderer type — the host maps its `GpuMemoryReport` onto
//! [`GpuMemoryStats`] — so this crate stays GPU-free (enforced by
//! `scripts/check-architecture.py`).
//!
//! # Honesty rules
//!
//! * A failed read leaves the previous value (or `None`); it never becomes `0`.
//! * A carrier without a sampler leaves every field `None` (see
//!   [`PerformanceState::refresh_system`]); the GPU half is only filled when the
//!   concrete host calls [`PerformanceState::record_gpu`].
//! * Failed reads are not logged: this runs on the owner tick and a missing
//!   `/proc` entry says nothing about the drawing.
//!
//! # Cadence
//!
//! System metrics refresh at most once per [`SAMPLE_INTERVAL`] (1 s) and GPU
//! statistics at most once per [`SAMPLE_INTERVAL`] as well (the host calls
//! [`PerformanceState::gpu_sample_due`] before reading the renderer). Both are
//! bounded so a 60 Hz tick never hits `/proc` or the driver allocator per frame.

use std::time::{Duration, Instant};

use cad_ui_core::session::CadPerformance;

/// Minimum wall time between two system (and GPU) samples.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

/// `USER_HZ`: the Linux ABI constant for `/proc` clock ticks per second.
///
/// It is documented as a compile-time constant of the kernel ABI (not the
/// CPU's `CLOCKS_PER_SEC`), and every architecture this project ships keeps the
/// `100` default. A different value would only shift the measured percentage, so
/// it is stated here instead of being guessed at runtime.
#[cfg(any(target_os = "linux", target_os = "android"))]
const USER_HZ: f64 = 100.0;

/// The renderer's own GPU-memory accounting, copied out of the renderer report.
///
/// All numbers describe **this renderer**, not the whole device, except
/// [`Self::vram_total_bytes`], which is the advertised device-local VRAM proxy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GpuMemoryStats {
    /// Bytes this renderer currently charges against the GPU device.
    pub used_bytes: u64,
    /// Peak of [`Self::used_bytes`] since the renderer started.
    pub peak_bytes: u64,
    /// Driver-reserved bytes (whole `Device`), when the backend sub-allocates.
    pub reserved_bytes: Option<u64>,
    /// Advertised device-local VRAM proxy (largest heap), when known.
    pub vram_total_bytes: Option<u64>,
}

/// The owner-tick performance sampler behind `cad_status.performance`.
///
/// Holds the last sampled values plus the instants that bound the sampling
/// cadence. Cheap to refresh: `/proc` reads are a few small files once per
/// second, and the GPU read happens in the host, only when
/// [`Self::gpu_sample_due`] is `true`.
#[derive(Debug, Default)]
pub struct PerformanceState {
    cpu_percent: Option<f64>,
    process_rss_bytes: Option<u64>,
    process_virtual_bytes: Option<u64>,
    system_total_bytes: Option<u64>,
    system_available_bytes: Option<u64>,
    gpu: Option<GpuMemoryStats>,
    /// CPU clock ticks and the instant they were read, for the next delta.
    last_cpu_sample: Option<(u64, Instant)>,
    /// When the system metrics were last refreshed (any target).
    last_system_sample: Option<Instant>,
    /// When the GPU statistics were last recorded (host-driven).
    last_gpu_sample: Option<Instant>,
}

impl PerformanceState {
    /// Refresh the process/system metrics, at most once per [`SAMPLE_INTERVAL`].
    ///
    /// On Linux and Android the values come from `/proc`:
    ///
    /// * `/proc/self/stat` → `utime + stime` clock ticks;
    /// * `/proc/self/status` → `VmRSS` / `VmSize`;
    /// * `/proc/meminfo` → `MemTotal` / `MemAvailable`.
    ///
    /// The CPU percentage is derived from the tick delta over the **real**
    /// elapsed wall time, so it is a measurement, not a tick-count guess. The
    /// first sample has no delta and therefore reports `None`.
    ///
    /// Every other target leaves all metrics `None`: this build has no OS
    /// measurement there, and guessing one is forbidden. Errors read as `None`
    /// as well (the previous value is kept untouched).
    pub fn refresh_system(&mut self) {
        let now = Instant::now();
        if self
            .last_system_sample
            .is_some_and(|last| now.duration_since(last) < SAMPLE_INTERVAL)
        {
            return;
        }
        self.last_system_sample = Some(now);
        #[cfg(any(target_os = "linux", target_os = "android"))]
        self.refresh_procfs(now);
    }

    /// Read the Linux `/proc` metrics and update the stored values.
    ///
    /// Kept as a separate method so the non-Linux build has no `/proc` code at
    /// all. Each read is independent: a missing/unreadable file leaves that
    /// metric at its previous value instead of clearing it to a fake zero.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn refresh_procfs(&mut self, now: Instant) {
        if let Some(ticks) = read_proc_stat_ticks() {
            self.cpu_percent = match self.last_cpu_sample {
                // `now` is the same instant the caller read, so the elapsed
                // window is the real one between the two samples.
                Some((previous, sampled_at)) => {
                    cpu_percent_from_delta(ticks, previous, now.duration_since(sampled_at))
                }
                // First sample: there is no delta yet, so there is no honest
                // percentage to report.
                None => None,
            };
            self.last_cpu_sample = Some((ticks, now));
        }
        if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
            if let Some(bytes) = parse_status_kb(&status, "VmRSS") {
                self.process_rss_bytes = Some(bytes);
            }
            if let Some(bytes) = parse_status_kb(&status, "VmSize") {
                self.process_virtual_bytes = Some(bytes);
            }
        }
        if let Ok(meminfo) = std::fs::read_to_string("/proc/meminfo") {
            if let Some(bytes) = parse_meminfo_kb(&meminfo, "MemTotal") {
                self.system_total_bytes = Some(bytes);
            }
            if let Some(bytes) = parse_meminfo_kb(&meminfo, "MemAvailable") {
                self.system_available_bytes = Some(bytes);
            }
        }
    }

    /// Whether the host should read the renderer's GPU report now.
    ///
    /// `true` on the first call and at most once per [`SAMPLE_INTERVAL`]
    /// afterwards. The host reads the report **only** when this is `true`, so a
    /// per-frame tick never queries the driver allocator.
    pub fn gpu_sample_due(&self) -> bool {
        self.last_gpu_sample
            .is_none_or(|last| last.elapsed() >= SAMPLE_INTERVAL)
    }

    /// Store the GPU report the host read this tick (or its explicit absence)
    /// and start the next sampling interval.
    ///
    /// `None` means the host has no attached renderer: the GPU fields stay
    /// `None` rather than keeping a stale number from a device that is gone.
    pub fn record_gpu(&mut self, stats: Option<GpuMemoryStats>) {
        self.gpu = stats;
        self.last_gpu_sample = Some(Instant::now());
    }

    /// Project the stored metrics onto the frozen wire block.
    ///
    /// Nothing is filled in here: every absent value stays `None`, which
    /// serializes as `null`.
    pub fn snapshot(&self) -> CadPerformance {
        CadPerformance {
            cpu_percent: self.cpu_percent,
            process_rss_bytes: self.process_rss_bytes,
            process_virtual_bytes: self.process_virtual_bytes,
            system_total_bytes: self.system_total_bytes,
            system_available_bytes: self.system_available_bytes,
            gpu_memory_used_bytes: self.gpu.map(|gpu| gpu.used_bytes),
            gpu_memory_peak_bytes: self.gpu.map(|gpu| gpu.peak_bytes),
            gpu_memory_reserved_bytes: self.gpu.and_then(|gpu| gpu.reserved_bytes),
            vram_total_bytes: self.gpu.and_then(|gpu| gpu.vram_total_bytes),
        }
    }
}

/// Read `utime + stime` from `/proc/self/stat`.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn read_proc_stat_ticks() -> Option<u64> {
    std::fs::read_to_string("/proc/self/stat")
        .ok()
        .and_then(|stat| parse_proc_stat_ticks(&stat))
}

/// Parse the 14th/15th fields (`utime`, `stime`) of `/proc/self/stat`.
///
/// The second field is the executable `comm` in parentheses and **may contain
/// spaces and `)`**, so the fields must be split after the **last** `)`; a
/// simple `split_whitespace` would mis-index such a process name. Fields 3.. are
/// then tokenised from there, making `utime`/`stime` token 11/12.
///
/// Kept compiled on every target so the synthetic-string tests run everywhere;
/// it is unreachable (hence allowed to be dead) on a target without `/proc`.
#[cfg_attr(not(any(target_os = "linux", target_os = "android")), allow(dead_code))]
fn parse_proc_stat_ticks(stat: &str) -> Option<u64> {
    let close = stat.rfind(')')?;
    let mut fields = stat[close + 1..].split_whitespace();
    // `fields.next()` is field 3 (`state`); utime is field 14 and stime field 15,
    // so they are tokens 11 and 12 of this tail.
    let utime: u64 = fields.nth(11)?.parse().ok()?;
    let stime: u64 = fields.next()?.parse().ok()?;
    utime.checked_add(stime)
}

/// Parse a `VmRSS:`/`VmSize:` style `kB` line from `/proc/self/status`.
///
/// Returns bytes (`value × 1024`); a non-kB unit or a missing key is `None`.
/// Kept compiled on every target for the synthetic-string tests; unreachable on
/// a target without `/proc`.
#[cfg_attr(not(any(target_os = "linux", target_os = "android")), allow(dead_code))]
fn parse_status_kb(status: &str, key: &str) -> Option<u64> {
    parse_kb_line(status, key)
}

/// Parse a `MemTotal:`/`MemAvailable:` style `kB` line from `/proc/meminfo`.
///
/// The format is the same as `/proc/self/status`; the separate helper documents
/// that the two files are read independently. Unreachable on a target without
/// `/proc`, like [`parse_status_kb`].
#[cfg_attr(not(any(target_os = "linux", target_os = "android")), allow(dead_code))]
fn parse_meminfo_kb(meminfo: &str, key: &str) -> Option<u64> {
    parse_kb_line(meminfo, key)
}

/// Shared `Key: <value> kB` parser: returns bytes.
///
/// Only the `kB` unit is accepted (the only unit either file uses for these
/// keys); a different unit would make a silent conversion wrong, so it is
/// `None` instead.
#[cfg_attr(not(any(target_os = "linux", target_os = "android")), allow(dead_code))]
fn parse_kb_line(text: &str, key: &str) -> Option<u64> {
    for line in text.lines() {
        let Some((name, rest)) = line.split_once(':') else {
            continue;
        };
        if name != key {
            continue;
        }
        let mut parts = rest.split_whitespace();
        let value: u64 = parts.next()?.parse().ok()?;
        if parts.next()? != "kB" {
            return None;
        }
        return value.checked_mul(1024);
    }
    None
}

/// Percentage of one core from a clock-tick delta and the real elapsed window.
///
/// `delta_ticks / USER_HZ` is CPU seconds; dividing by the elapsed seconds gives
/// the average number of cores used, which is the percentage of one core when
/// multiplied by 100. A zero window, a non-positive tick delta (a counter reset)
/// or a non-finite result is `None`, never a guessed value.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn cpu_percent_from_delta(ticks: u64, previous: u64, elapsed: Duration) -> Option<f64> {
    let delta = ticks.checked_sub(previous)?;
    let seconds = elapsed.as_secs_f64();
    if seconds <= 0.0 {
        return None;
    }
    let percent = delta as f64 / USER_HZ / seconds * 100.0;
    percent.is_finite().then_some(percent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_state_reports_no_metrics() {
        let state = PerformanceState::default();
        assert_eq!(state.snapshot(), CadPerformance::default());
        assert_eq!(state.snapshot().cpu_percent, None);
        assert_eq!(state.snapshot().gpu_memory_used_bytes, None);
    }

    #[test]
    fn recording_gpu_stats_projects_exactly_those_values() {
        let mut state = PerformanceState::default();
        state.record_gpu(Some(GpuMemoryStats {
            used_bytes: 1_048_576,
            peak_bytes: 2_097_152,
            reserved_bytes: Some(4_194_304),
            vram_total_bytes: Some(8_589_934_592),
        }));
        let snapshot = state.snapshot();
        assert_eq!(snapshot.gpu_memory_used_bytes, Some(1_048_576));
        assert_eq!(snapshot.gpu_memory_peak_bytes, Some(2_097_152));
        assert_eq!(snapshot.gpu_memory_reserved_bytes, Some(4_194_304));
        assert_eq!(snapshot.vram_total_bytes, Some(8_589_934_592));
        // The system metrics are untouched: an absent sampler value stays `None`.
        assert_eq!(snapshot.cpu_percent, None);
        assert_eq!(snapshot.process_rss_bytes, None);
    }

    #[test]
    fn a_missing_gpu_report_clears_the_gpu_fields_instead_of_keeping_stale_ones() {
        let mut state = PerformanceState::default();
        state.record_gpu(Some(GpuMemoryStats {
            used_bytes: 10,
            ..GpuMemoryStats::default()
        }));
        state.record_gpu(None);
        let snapshot = state.snapshot();
        assert_eq!(snapshot.gpu_memory_used_bytes, None);
        assert_eq!(snapshot.gpu_memory_peak_bytes, None);
    }

    #[test]
    fn a_status_line_with_a_spaced_and_parenthesised_comm_still_parses() {
        // `comm` contains both a space and nested parentheses: only a split after
        // the **last** `)` indexes the fields correctly.
        let stat = "1234 (my (weird) prog) S 1 1234 1234 0 -1 4194560 123 0 0 0 \
                    100 50 0 0 20 0 1 0 12345 123456 789";
        assert_eq!(parse_proc_stat_ticks(stat), Some(150));
    }

    #[test]
    fn a_malformed_status_line_parses_to_none() {
        assert_eq!(parse_proc_stat_ticks("no parentheses here"), None);
        assert_eq!(parse_proc_stat_ticks("1 (short) S 1"), None);
        assert_eq!(parse_proc_stat_ticks("1 (x) S 1 2 3"), None);
    }

    #[test]
    fn status_and_meminfo_kb_keys_convert_to_bytes() {
        let status = "Name:\tyacr\nVmSize:\t   40960 kB\nVmRSS:\t    2048 kB\n";
        assert_eq!(parse_status_kb(status, "VmRSS"), Some(2_048 * 1024));
        assert_eq!(parse_status_kb(status, "VmSize"), Some(40_960 * 1024));
        assert_eq!(parse_status_kb(status, "VmPeak"), None);

        let meminfo =
            "MemTotal:       16384000 kB\nMemFree: 1000 kB\nMemAvailable:    8192000 kB\n";
        assert_eq!(
            parse_meminfo_kb(meminfo, "MemTotal"),
            Some(16_384_000 * 1024)
        );
        assert_eq!(
            parse_meminfo_kb(meminfo, "MemAvailable"),
            Some(8_192_000 * 1024)
        );
    }

    #[test]
    fn a_non_kb_unit_or_non_numeric_value_is_none() {
        assert_eq!(parse_status_kb("VmRSS:\t 2048 MB\n", "VmRSS"), None);
        assert_eq!(parse_status_kb("VmRSS:\t huge kB\n", "VmRSS"), None);
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn the_first_cpu_sample_has_no_delta() {
        let mut state = PerformanceState::default();
        state.refresh_system();
        // Regardless of the machine, the first sample has no previous tick
        // count, so no percentage can be reported.
        assert_eq!(state.snapshot().cpu_percent, None);
        assert!(state.last_cpu_sample.is_some());
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn cpu_percent_is_the_tick_delta_over_the_real_window() {
        // 20 ticks over one real second at USER_HZ=100 → 0.2 CPU-seconds → 20 %.
        let percent = cpu_percent_from_delta(120, 100, Duration::from_secs(1)).expect("delta");
        assert!((percent - 20.0).abs() < 1e-9, "{percent}");
        // Half a second: the same delta is twice the rate.
        let percent = cpu_percent_from_delta(120, 100, Duration::from_millis(500)).expect("delta");
        assert!((percent - 40.0).abs() < 1e-9, "{percent}");
        // Idle: no tick delta → 0 % is a real measurement, not a fabricated one.
        assert_eq!(
            cpu_percent_from_delta(100, 100, Duration::from_secs(1)),
            Some(0.0)
        );
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn an_impossible_delta_or_window_is_none() {
        assert_eq!(
            cpu_percent_from_delta(90, 100, Duration::from_secs(1)),
            None
        );
        assert_eq!(cpu_percent_from_delta(120, 100, Duration::ZERO), None);
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    #[test]
    fn a_target_without_a_sampler_reports_no_system_metrics() {
        let mut state = PerformanceState::default();
        state.refresh_system();
        let snapshot = state.snapshot();
        assert_eq!(snapshot.cpu_percent, None);
        assert_eq!(snapshot.process_rss_bytes, None);
        assert_eq!(snapshot.process_virtual_bytes, None);
        assert_eq!(snapshot.system_total_bytes, None);
        assert_eq!(snapshot.system_available_bytes, None);
    }
}
