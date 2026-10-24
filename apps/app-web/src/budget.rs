//! Named memory budgets and the bounded upload-chunk planner.
//!
//! This module is **target-independent on purpose**: it is plain arithmetic over
//! `usize`/`f64`, with no `web-sys`, no `wgpu` and no CAD type. That is what lets
//! the refusal arithmetic and the chunk boundaries be unit-tested on the host
//! (`cargo test -p app-web`) instead of only inside a browser, where a failed
//! budget would be observed as a dead module rather than a test failure.
//!
//! # Why budgets at all
//!
//! `wasm32-unknown-unknown` has a **hard** 4 GiB linear-memory ceiling. Crossing
//! it is not an `Err`: the allocator aborts and the module (and with it the whole
//! canvas session, its renderer and its GPU surface) is gone. A named refusal is
//! therefore strictly better than "try and see": the previous document keeps its
//! pixels and stays navigable, and the host reports a stable
//! `step: "budget"` instead of a blank canvas.
//!
//! Two checks, at the only two points where the answer is knowable:
//!
//! 1. [`check_input_bytes`] — **before** parsing. Cheap, and it refuses an
//!    absurd input before the importer copies and parses it.
//! 2. [`check_heap_after_build`] — **after** the scene build and **before** any
//!    GPU upload. The imported `Database` and the display representation are both
//!    resident at that instant, so this is the peak the upload would grow from.
//!
//! Neither check can bound the footprint *a priori*: measured on this machine
//! (Chromium 141, release wasm, 2026-10-24) a 4,228,788-byte DWG
//! (`fixtures/complex-test.dwg`) builds 145,947 batches and grows the wasm heap to
//! 1,909,850,112 B — about 450× the input. Input size is a useful cheap gate; the
//! heap measurement is the one that actually protects the session.

/// Largest input file this host will parse: **64 MiB**.
///
/// Rationale, not a round number picked for looks:
///
/// * the largest file measured to open successfully on this machine is
///   4,228,788 B, so 64 MiB is ~15× that — far above any file the pipeline is
///   known to handle, so it does not refuse plausible inputs today;
/// * the importer clones the bytes once (`Arc<[u8]>`), i.e. this cap bounds a
///   *cheap* allocation at 1.5 % of the wasm address space;
/// * it refuses obviously-absurd input (a 1 GiB DWG) before the importer spends
///   minutes parsing it only to abort at the 4 GiB ceiling.
///
/// It is deliberately **not** a promise that a file just under it fits: the
/// footprint is ~450× the input and depends on entity/representation shape, so
/// the post-build heap budget is the guard that must hold.
pub const MAX_INPUT_BYTES: usize = 64 * 1024 * 1024;

/// The hard linear-memory ceiling of a 32-bit wasm module: 65,536 pages × 64 KiB.
///
/// Named so the refusal message can say what fraction of it was used.
pub const WASM_HEAP_LIMIT_BYTES: f64 = 4.0 * 1024.0 * 1024.0 * 1024.0;

/// Largest wasm heap allowed **after the scene build**: **3 GiB**, i.e. 75 % of
/// [`WASM_HEAP_LIMIT_BYTES`].
///
/// The remaining 1 GiB is the headroom the upload needs: chunk staging bytes,
/// the renderer's `GpuBatch` metadata for every batch, and the frame texture
/// plumbing. Refusing at the build (before any GPU object exists) means the
/// refusal costs no partial scene and no second presenter; refusing later would
/// mean refusing after the upload had already moved the peak most of the way to
/// the abort.
///
/// This is a **cliff**, not a target: the session aborts at 4 GiB, so a build
/// that lands above 3 GiB has no honest path to a complete first frame.
pub const MAX_WASM_HEAP_BYTES: f64 = 3.0 * 1024.0 * 1024.0 * 1024.0;

/// Whether the session keeps the imported `Database` after a successful open.
///
/// **`false`: the database is released.** The canvas contract in this crate is
/// presentation + camera navigation; the database is used exactly twice
/// (`build_scene` and the one-off camera fit from its bounds) and then dropped
/// before any GPU object exists. There is no save, measure, query or command
/// surface here that could need it later.
///
/// It must become `true` — and the status then reports `db_retained: true` — as
/// soon as a capability that needs the imported database (save, measure, query,
/// entity selection by id) is added to this host. Keeping it "just in case" is a
/// silent, unexplained multi-hundred-megabyte shadow copy of the document, which
/// is exactly what this constant exists to make visible.
pub const DB_RETAINED: bool = false;

/// Batches staged by one `prepare_upload_batches` call, at most.
///
/// 2048 is the batching bound only; the vertex bound below is what usually ends
/// a chunk first. The measured baseline (145,947 batches) becomes 72 chunks, so
/// the per-chunk bookkeeping stays negligible while the per-chunk staging
/// transient (packed instance/quad bytes) never approaches a whole-scene copy.
pub const UPLOAD_CHUNK_MAX_BATCHES: usize = 2048;

/// Vertices staged by one `prepare_upload_batches` call, at most.
///
/// The same 262,144-vertex piece cap `cad-engine` uses when it pages uploads, so
/// the two hosts bound their staging by the same rule.
pub const UPLOAD_CHUNK_MAX_VERTICES: usize = 262_144;

/// One refused budget: which named budget tripped, and the two numbers behind it.
///
/// Carried into the status JSON (`budget: {name, limit_bytes, measured_bytes}`) so
/// evidence can read the numbers without parsing the message, while
/// [`BudgetRefusal::message`] still names all three in prose (`AGENTS.md` rule 2:
/// an unsupported/over-budget case is modelled, never a silent success).
#[derive(Debug, Clone, PartialEq)]
pub struct BudgetRefusal {
    /// The constant that tripped: `MAX_INPUT_BYTES` or `MAX_WASM_HEAP_BYTES`.
    pub name: &'static str,
    /// That constant's value, in bytes.
    pub limit_bytes: f64,
    /// The measured value that tripped it, in bytes.
    pub measured_bytes: f64,
    /// The stable, human-readable refusal naming limit, measurement and budget.
    pub message: String,
}

impl BudgetRefusal {
    /// Build a refusal whose message names the budget, its limit and the measured
    /// value, plus what the refusal did (and did not) change.
    fn new(
        name: &'static str,
        limit_bytes: f64,
        measured_bytes: f64,
        label: &str,
        what: &str,
        effect: &str,
    ) -> Self {
        let message = format!(
            "{label}: {what} is {measured_bytes} bytes, over {name} = {limit_bytes} bytes; \
             {effect}"
        );
        Self {
            name,
            limit_bytes,
            measured_bytes,
            message,
        }
    }
}

/// Refuse an input file larger than [`MAX_INPUT_BYTES`], before it is parsed.
///
/// `label` names the caller's file in the message. The limit is inclusive: a file
/// of exactly [`MAX_INPUT_BYTES`] is admitted.
pub fn check_input_bytes(label: &str, input_bytes: usize) -> Result<(), BudgetRefusal> {
    let measured = input_bytes as f64;
    if measured > MAX_INPUT_BYTES as f64 {
        return Err(BudgetRefusal::new(
            "MAX_INPUT_BYTES",
            MAX_INPUT_BYTES as f64,
            measured,
            label,
            "the input file",
            "the file was not parsed and the open document is unchanged",
        ));
    }
    Ok(())
}

/// Refuse a scene whose wasm heap already exceeds [`MAX_WASM_HEAP_BYTES`] after
/// the build, before any GPU object is created.
///
/// `heap_bytes` is the module's own linear-memory length. `None` (a host that does
/// not expose `WebAssembly.Memory`) passes: an unmeasurable heap cannot be gated,
/// and the status then reports `heap_after_open: null` rather than a made-up
/// number — the honest limit of this check, stated instead of hidden.
///
/// The limit is inclusive: exactly [`MAX_WASM_HEAP_BYTES`] is admitted.
///
/// A refusal here does not *free* the refused footprint: wasm linear memory is a
/// high-water mark and never shrinks, so a build that peaked above the budget
/// leaves the module there. What it buys is that the peak stops **at the build**
/// instead of continuing into the upload toward the 4 GiB abort, and that the
/// still-open document keeps its renderer, its pixels and its navigable camera.
pub fn check_heap_after_build(label: &str, heap_bytes: Option<f64>) -> Result<(), BudgetRefusal> {
    let Some(measured) = heap_bytes else {
        return Ok(());
    };
    if !measured.is_finite() {
        return Ok(());
    }
    if measured > MAX_WASM_HEAP_BYTES {
        return Err(BudgetRefusal::new(
            "MAX_WASM_HEAP_BYTES",
            MAX_WASM_HEAP_BYTES,
            measured,
            label,
            "the wasm heap after the scene build",
            "the new scene was dropped before any GPU upload and the open document is unchanged",
        ));
    }
    Ok(())
}

/// The exclusive end index of the upload chunk starting at `start`.
///
/// `vertices_at(index)` is that batch's vertex count (zero for conic-only
/// batches). A chunk ends at the first of:
///
/// * [`UPLOAD_CHUNK_MAX_BATCHES`] batches, or
/// * the batch that would take the chunk past [`UPLOAD_CHUNK_MAX_VERTICES`].
///
/// A batch is never split, so a single batch larger than the vertex bound is a
/// chunk of its own — this always returns `> start` while `start < total_batches`,
/// which is what makes the caller's loop terminate.
pub fn next_chunk_end(
    total_batches: usize,
    start: usize,
    vertices_at: impl Fn(usize) -> usize,
) -> usize {
    if start >= total_batches {
        return total_batches;
    }
    let mut end = start + 1;
    let mut vertices = vertices_at(start);
    while end < total_batches && end - start < UPLOAD_CHUNK_MAX_BATCHES {
        let next = vertices.saturating_add(vertices_at(end));
        if next > UPLOAD_CHUNK_MAX_VERTICES {
            break;
        }
        vertices = next;
        end += 1;
    }
    end
}

/// How many chunks [`next_chunk_end`] will produce for this batch list.
///
/// The open path uses it once, up front, to report *real* chunk progress
/// (`chunk 7/72`) instead of a guessed percentage.
pub fn chunk_count(total_batches: usize, vertices_at: impl Fn(usize) -> usize) -> usize {
    let mut chunks = 0;
    let mut start = 0;
    while start < total_batches {
        start = next_chunk_end(total_batches, start, &vertices_at);
        chunks += 1;
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_budget_admits_exactly_the_limit() {
        assert_eq!(check_input_bytes("f.dwg", MAX_INPUT_BYTES), Ok(()));
        assert_eq!(check_input_bytes("f.dwg", 0), Ok(()));
    }

    #[test]
    fn input_budget_refuses_one_byte_over_and_names_all_three_facts() {
        let refusal = check_input_bytes("big.dwg", MAX_INPUT_BYTES + 1)
            .expect_err("one byte over the input cap must be refused");
        assert_eq!(refusal.name, "MAX_INPUT_BYTES");
        assert_eq!(refusal.limit_bytes, MAX_INPUT_BYTES as f64);
        assert_eq!(refusal.measured_bytes, (MAX_INPUT_BYTES + 1) as f64);
        // The message names the limit, the measurement and the budget.
        assert!(
            refusal.message.contains("MAX_INPUT_BYTES"),
            "{}",
            refusal.message
        );
        assert!(
            refusal.message.contains(&(MAX_INPUT_BYTES + 1).to_string()),
            "{}",
            refusal.message
        );
        assert!(refusal.message.contains("big.dwg"), "{}", refusal.message);
        // ... and says nothing was parsed and the open document is unchanged.
        assert!(
            refusal.message.contains("not parsed"),
            "{}",
            refusal.message
        );
        assert!(refusal.message.contains("unchanged"), "{}", refusal.message);
    }

    #[test]
    fn heap_budget_admits_the_limit_and_unmeasurable_heaps() {
        assert_eq!(
            check_heap_after_build("f.dwg", Some(MAX_WASM_HEAP_BYTES)),
            Ok(())
        );
        // No measurement (a host without WebAssembly.Memory): not gated, reported
        // as null instead of pretended.
        assert_eq!(check_heap_after_build("f.dwg", None), Ok(()));
        assert_eq!(check_heap_after_build("f.dwg", Some(f64::NAN)), Ok(()));
    }

    #[test]
    fn heap_budget_refuses_over_the_limit_at_three_gib() {
        assert_eq!(MAX_WASM_HEAP_BYTES, 0.75 * WASM_HEAP_LIMIT_BYTES);
        let measured = MAX_WASM_HEAP_BYTES + 4096.0;
        let refusal = check_heap_after_build("huge.dwg", Some(measured)).expect_err("over budget");
        assert_eq!(refusal.name, "MAX_WASM_HEAP_BYTES");
        assert_eq!(refusal.limit_bytes, MAX_WASM_HEAP_BYTES);
        assert_eq!(refusal.measured_bytes, measured);
        assert!(
            refusal.message.contains("MAX_WASM_HEAP_BYTES"),
            "{}",
            refusal.message
        );
        assert!(refusal.message.contains("huge.dwg"), "{}", refusal.message);
        assert!(
            refusal.message.contains("before any GPU upload"),
            "{}",
            refusal.message
        );
    }

    #[test]
    fn chunk_end_on_empty_and_exhausted_lists_makes_no_progress() {
        assert_eq!(next_chunk_end(0, 0, |_| 0), 0);
        assert_eq!(next_chunk_end(3, 3, |_| 10), 3);
        assert_eq!(chunk_count(0, |_| 0), 0);
    }

    #[test]
    fn chunk_end_respects_the_batch_cap() {
        let total = UPLOAD_CHUNK_MAX_BATCHES * 2 + 5;
        assert_eq!(next_chunk_end(total, 0, |_| 0), UPLOAD_CHUNK_MAX_BATCHES);
        // 2053 batches remain, so the second chunk stops at the batch cap too and
        // only the five leftover batches form the third chunk.
        assert_eq!(
            next_chunk_end(total, UPLOAD_CHUNK_MAX_BATCHES, |_| 0),
            UPLOAD_CHUNK_MAX_BATCHES * 2
        );
        assert_eq!(chunk_count(total, |_| 0), 3);
    }

    #[test]
    fn chunk_end_respects_the_vertex_cap_before_the_batch_cap() {
        // Four 100k-vertex batches: only two fit under 262,144 vertices.
        let per_batch = 100_000;
        let total = 4;
        let end = next_chunk_end(total, 0, |_| per_batch);
        assert_eq!(end, 2, "200,000 fits, 300,000 does not");
    }

    #[test]
    fn one_oversized_batch_is_its_own_chunk() {
        let vertices = [UPLOAD_CHUNK_MAX_VERTICES * 3, 4, 4];
        // The oversized first batch gets a chunk to itself; the two small batches
        // then share the next one.
        assert_eq!(next_chunk_end(vertices.len(), 0, |i| vertices[i]), 1);
        assert_eq!(next_chunk_end(vertices.len(), 1, |i| vertices[i]), 3);
        assert_eq!(chunk_count(vertices.len(), |i| vertices[i]), 2);
    }

    #[test]
    fn chunk_plan_covers_every_batch_exactly_once_and_stays_in_bounds() {
        // Deterministic pseudo-random vertex counts (no rand dependency).
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        let counts: Vec<usize> = (0..5000)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state % 5000) as usize
            })
            .collect();

        let mut start = 0;
        let mut seen = 0;
        let mut chunks = 0;
        while start < counts.len() {
            let end = next_chunk_end(counts.len(), start, |i| counts[i]);
            assert!(end > start, "planner must make progress at {start}");
            let batches = end - start;
            assert!(
                batches <= UPLOAD_CHUNK_MAX_BATCHES,
                "chunk of {batches} batches over the cap"
            );
            if batches > 1 {
                let vertices: usize = counts[start..end].iter().sum();
                assert!(
                    vertices <= UPLOAD_CHUNK_MAX_VERTICES,
                    "chunk of {vertices} vertices over the cap"
                );
            }
            seen += batches;
            chunks += 1;
            start = end;
        }
        assert_eq!(seen, counts.len(), "every batch is uploaded exactly once");
        assert_eq!(chunks, chunk_count(counts.len(), |i| counts[i]));
    }

    #[test]
    fn the_complex_fixture_baseline_plans_a_bounded_number_of_chunks() {
        // The measured baseline: 145,947 batches, ~14 vertices each on average.
        let total = 145_947;
        let chunks = chunk_count(total, |_| 14);
        assert!(
            chunks > 1,
            "the baseline must be chunked, not staged at once"
        );
        assert!(
            chunks <= total / UPLOAD_CHUNK_MAX_BATCHES + 2,
            "chunk count {chunks} should follow the batch cap"
        );
        // Peak is the first chunk, and it is the batch cap, not the whole scene.
        assert_eq!(next_chunk_end(total, 0, |_| 14), UPLOAD_CHUNK_MAX_BATCHES);
    }
}
