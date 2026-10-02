//! Public proxy cache decoding only; raw DWG records are not graphic_data.
//!
//! Spec v2.0 §7.2. The metafile framing is the same one acadrust 0.5.5 parses
//! (`total_size: u32`, `record_count: u32`, then per record `size: u32`,
//! `type: u32`, payload). Only opcodes we have real evidence for are decoded —
//! today that is type 21 (`FillOff`) and type 36 (Unicode text), which the
//! library itself types. Everything else is treated conservatively: an unknown
//! opcode may change state for all following records, so replay stops emitting
//! geometry and the output is marked incomplete. Nothing is guessed, and raw
//! DWG records are never mixed into `graphic_data`.
//!
//! # Fail-closed contract
//!
//! Any record we cannot decode with evidence — unknown opcode, malformed
//! payload, over-limit vertex budget, or a decoder chain deeper than
//! [`DecodeLimits::max_stack_depth`] — is reported as
//! [`Completeness::Partial`]/[`Completeness::Missing`] and the *raw payload is
//! retained* in [`ProxyOutput::unsupported`]. We never emit partially-wrong
//! geometry: a record either decodes fully under the documented layout or it is
//! reported as unsupported. See `docs/proxy-support.md` for the per-opcode
//! evidence table.

use cad_domain::*;

pub mod metafile;

/// Bounded limits applied while decoding untrusted data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeLimits {
    pub max_bytes: usize,
    pub max_records: usize,
    pub max_vertices: usize,
    pub max_stack_depth: usize,
}

impl Default for DecodeLimits {
    fn default() -> Self {
        DecodeLimits {
            max_bytes: 16 * 1024 * 1024,
            max_records: 200_000,
            max_vertices: 5_000_000,
            max_stack_depth: 64,
        }
    }
}

/// Provenance of a proxy capture, recorded before decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxySource {
    pub handle: String,
    pub class_name: String,
    pub application: Option<String>,
    pub dwg_version: String,
}

/// One raw proxy record handed to a decoder.
#[derive(Debug, Clone, Copy)]
pub struct ProxyRecord<'a> {
    pub record_type: u32,
    pub data: &'a [u8],
}

/// A record that could not be decoded with evidence, kept verbatim.
///
/// The raw payload is retained so a caller (or an evidence collector) can
/// inspect it later without re-parsing the cache; nothing here is interpreted
/// as geometry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedRecord {
    pub record_type: u32,
    /// Why the record was rejected (unknown opcode, malformed payload, limit).
    pub reason: String,
    /// The exact payload bytes after the 8-byte record header.
    pub data: Vec<u8>,
}

/// Replay state carried across records.
#[derive(Debug, Clone)]
pub struct ReplayState {
    pub transform: Transform3,
    pub rgba: [u8; 4],
    pub fill: bool,
    /// False once an unknown or malformed opcode makes later state untrusted.
    pub state_reliable: bool,
}

impl Default for ReplayState {
    fn default() -> Self {
        ReplayState {
            transform: Transform3::identity(),
            rgba: [255, 255, 255, 255],
            fill: true,
            state_reliable: true,
        }
    }
}

/// Result of replaying a proxy metafile.
#[derive(Debug, Clone, PartialEq)]
pub struct ProxyOutput {
    pub geometry: Vec<SemanticGeometry>,
    pub completeness: Completeness,
    pub diagnostics: Vec<Diagnostic>,
    /// Records rejected while replaying, with their raw payloads retained.
    ///
    /// Empty means every record present was decoded with evidence (which may
    /// still be [`Completeness::Missing`] if the cache itself was empty).
    pub unsupported: Vec<UnsupportedRecord>,
}

impl ProxyOutput {
    pub fn empty_missing(reason: impl Into<String>) -> Self {
        ProxyOutput {
            geometry: Vec::new(),
            completeness: Completeness::Missing(vec![reason.into()]),
            diagnostics: Vec::new(),
            unsupported: Vec::new(),
        }
    }
}

/// A pluggable, evidence-based opcode decoder (spec §17.1 registry item).
pub trait ProxyRecordDecoder {
    fn registration(&self) -> Registration;
    fn decode(
        &self,
        record: &ProxyRecord<'_>,
        state: &mut ReplayState,
        limits: &DecodeLimits,
    ) -> CadResult<Vec<SemanticGeometry>>;
}

/// The opcodes acadrust 0.5.5 itself types, and therefore evidence-backed.
#[derive(Debug, Default, Clone, Copy)]
pub struct KnownOpcodeDecoder;

impl KnownOpcodeDecoder {
    pub const FILL_OFF: u32 = 21;
    pub const UNICODE_TEXT: u32 = 36;
    /// Fixed prefix size of a type-36 record: 3 vectors + 3 f64.
    const UNICODE_TEXT_FIXED: usize = 96;
}

impl ProxyRecordDecoder for KnownOpcodeDecoder {
    fn registration(&self) -> Registration {
        Registration {
            type_key: "yacr.proxy.known_opcodes".into(),
            version: 1,
            priority: 100,
            entity_types: vec!["ACAD_PROXY_ENTITY".into()],
            capabilities: vec!["FillOff".into(), "UnicodeText".into()],
        }
    }

    fn decode(
        &self,
        record: &ProxyRecord<'_>,
        state: &mut ReplayState,
        limits: &DecodeLimits,
    ) -> CadResult<Vec<SemanticGeometry>> {
        if record.data.len() > limits.max_bytes {
            return Err(CadError::CorruptData(
                "proxy record exceeds byte limit".into(),
            ));
        }
        match record.record_type {
            Self::FILL_OFF => {
                // acadrust 0.5.5 only types type 21 as FillOff when the payload
                // is empty; a non-empty payload is `Unknown` there, so we must
                // not silently drop the bytes or treat it as a decoded fill-off.
                if !record.data.is_empty() {
                    return Err(CadError::Unsupported(
                        "type 21 is only FillOff when its payload is empty".into(),
                    ));
                }
                state.fill = false;
                Ok(Vec::new())
            }
            Self::UNICODE_TEXT => {
                let t = decode_unicode_text(record.data)?;
                Ok(vec![SemanticGeometry::Text {
                    text: t.0,
                    position: t.1,
                    // StyleId(0) is the default text style resolved by the
                    // representation layer; proxy text carries no style handle.
                    style: StyleId(0),
                    height: t.2,
                    rotation: t.3,
                    font: None,
                    h_align: TextAlignH::Left,
                    v_align: TextAlignV::Baseline,
                }])
            }
            other => Err(CadError::Unsupported(format!(
                "proxy opcode {other} has no evidence-backed decoder"
            ))),
        }
    }
}

/// Decode `(text, position, height, rotation)` from a type-36 record.
///
/// The layout is exactly acadrust 0.5.5's: `position`/`normal`/`direction`
/// vectors at 0/24/48, then `height`/`width_factor`/`oblique_angle` at
/// 72/80/88, then little-endian UTF-16 units terminated by `0x0000` (and, in
/// acadrust's encoder, 4-byte alignment padding). This decoder is deliberately
/// stricter than acadrust's `Option` fallback:
///
/// * a missing/absent terminator or an odd trailing byte is rejected instead of
///   being silently truncated;
/// * invalid UTF-16 (unpaired surrogates) is rejected instead of being lossily
///   replaced;
/// * non-finite vectors/height are rejected.
///
/// Each rejection becomes `CorruptData`, which `replay` records as an
/// unsupported record with the raw bytes retained.
fn decode_unicode_text(data: &[u8]) -> CadResult<(String, Point3, f64, f64)> {
    const FIXED: usize = KnownOpcodeDecoder::UNICODE_TEXT_FIXED;
    if data.len() < FIXED + 2 {
        return Err(CadError::CorruptData(
            "Unicode text record is truncated".into(),
        ));
    }
    let f = |off: usize| -> f64 {
        let mut b = [0u8; 8];
        b.copy_from_slice(&data[off..off + 8]);
        f64::from_le_bytes(b)
    };
    let position = Point3 {
        x: f(0),
        y: f(8),
        z: f(16),
    };
    // bytes 24..48 normal (retained by acadrust; unused for 2D placement),
    // 48..72 direction.
    let direction = Point3 {
        x: f(48),
        y: f(56),
        z: f(64),
    };
    let height = f(72);
    // width_factor at 80, oblique at 88 (reserved for later style support).

    let tail = &data[FIXED..];
    if tail.len() % 2 != 0 {
        return Err(CadError::CorruptData(
            "Unicode text has an odd number of tail bytes".into(),
        ));
    }
    let units: Vec<u16> = tail
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let terminator = units.iter().position(|u| *u == 0).ok_or_else(|| {
        CadError::CorruptData("Unicode text is missing its 0x0000 terminator".into())
    })?;
    let text = String::from_utf16(&units[..terminator])
        .map_err(|_| CadError::CorruptData("Unicode text has invalid UTF-16".into()))?;

    let rotation = direction.y.atan2(direction.x);
    if !position.x.is_finite()
        || !position.y.is_finite()
        || !position.z.is_finite()
        || !height.is_finite()
    {
        return Err(CadError::CorruptData(
            "Unicode text has non-finite fields".into(),
        ));
    }
    Ok((text, position, height, rotation))
}

/// Replays a proxy metafile through a chain of decoders.
pub struct ProxyPlayer {
    pub limits: DecodeLimits,
    decoders: Vec<Box<dyn ProxyRecordDecoder>>,
}

impl Default for ProxyPlayer {
    fn default() -> Self {
        Self::new(DecodeLimits::default())
    }
}

impl ProxyPlayer {
    pub fn new(limits: DecodeLimits) -> Self {
        ProxyPlayer {
            limits,
            decoders: vec![Box::new(KnownOpcodeDecoder)],
        }
    }

    /// Register an additional decoder; higher priority wins ties.
    pub fn with_decoder(mut self, decoder: Box<dyn ProxyRecordDecoder>) -> Self {
        self.decoders.push(decoder);
        self
    }

    fn decode_record(
        &self,
        record: &ProxyRecord<'_>,
        state: &mut ReplayState,
    ) -> CadResult<Vec<SemanticGeometry>> {
        // The decoder chain is caller-extensible (`with_decoder`), so it is the
        // one genuinely unbounded dispatch path in this crate. Bound it by
        // `max_stack_depth` and fail closed rather than consulting an
        // arbitrarily deep chain.
        if self.decoders.len() > self.limits.max_stack_depth {
            return Err(CadError::Unsupported(format!(
                "decoder chain of {} exceeds the stack-depth limit of {}",
                self.decoders.len(),
                self.limits.max_stack_depth
            )));
        }
        let mut ordered: Vec<&Box<dyn ProxyRecordDecoder>> = self.decoders.iter().collect();
        ordered.sort_by_key(|d| std::cmp::Reverse(d.registration().priority));
        let mut last: Option<CadError> = None;
        for decoder in ordered {
            match decoder.decode(record, state, &self.limits) {
                Ok(g) => return Ok(g),
                Err(e @ CadError::Unsupported(_)) => last = Some(e),
                Err(e) => return Err(e),
            }
        }
        Err(last.unwrap_or(CadError::Unsupported(format!(
            "no decoder handles record type {}",
            record.record_type
        ))))
    }

    /// Replay a `graphic_data` metafile.
    pub fn replay(&self, source: &ProxySource, graphic_data: &[u8]) -> CadResult<ProxyOutput> {
        if graphic_data.is_empty() {
            return Ok(ProxyOutput::empty_missing(
                "proxy entity has no cached graphic data",
            ));
        }
        if graphic_data.len() > self.limits.max_bytes {
            return Err(CadError::CorruptData(format!(
                "proxy cache is {} bytes, over the {} byte limit",
                graphic_data.len(),
                self.limits.max_bytes
            )));
        }
        let records = metafile::decode(graphic_data, self.limits)?;
        let mut state = ReplayState::default();
        let mut geometry = Vec::new();
        let mut diagnostics = vec![Diagnostic {
            object: None,
            code: "proxy.capture".into(),
            message: format!(
                "{} {} cached {} bytes from {}",
                source.class_name,
                source.handle,
                graphic_data.len(),
                source.dwg_version
            ),
        }];
        let mut unsupported: Vec<UnsupportedRecord> = Vec::new();
        let mut stop_reason: Option<String> = None;
        let mut vertices = 0usize;
        let mut decoded_records = 0usize;

        for (index, raw) in records.iter().enumerate() {
            if index >= self.limits.max_records {
                if stop_reason.is_none() {
                    stop_reason = Some(format!(
                        "record limit of {} reached",
                        self.limits.max_records
                    ));
                    diagnostics.push(Diagnostic {
                        object: None,
                        code: "proxy.limit".into(),
                        message: format!(
                            "stopped after {} records (limit)",
                            self.limits.max_records
                        ),
                    });
                }
                break;
            }
            if !state.state_reliable {
                // Replay already stopped on an undecodable record; later
                // records are untrusted, so we neither decode nor report them
                // again. The remaining framing is still bounded by max_records.
                continue;
            }
            let record = ProxyRecord {
                record_type: raw.record_type,
                data: raw.data,
            };
            match self.decode_record(&record, &mut state) {
                Ok(g) => {
                    let record_vertices = g.iter().map(vertex_count).sum::<usize>();
                    // Enforce the vertex budget *before* emitting: appending the
                    // over-budget primitive would hand callers geometry the
                    // limit was meant to reject (audit B/F11).
                    if vertices + record_vertices > self.limits.max_vertices {
                        stop_reason = Some("vertex limit reached".into());
                        state.state_reliable = false;
                        diagnostics.push(Diagnostic {
                            object: None,
                            code: "proxy.limit".into(),
                            message: format!(
                                "record {} would exceed the vertex limit of {}",
                                raw.record_type, self.limits.max_vertices
                            ),
                        });
                        continue;
                    }
                    vertices += record_vertices;
                    geometry.extend(g);
                    decoded_records += 1;
                }
                Err(CadError::Unsupported(msg)) => {
                    // Unknown opcode: an unrecognised opcode may be a state
                    // instruction affecting everything after it, so replay stops
                    // and the raw payload is retained for evidence.
                    unsupported.push(UnsupportedRecord {
                        record_type: raw.record_type,
                        reason: msg.clone(),
                        data: raw.data.to_vec(),
                    });
                    state.state_reliable = false;
                    stop_reason = Some(format!("unrecognised opcode(s): [{}]", raw.record_type));
                    diagnostics.push(Diagnostic {
                        object: None,
                        code: "proxy.unknown_opcode".into(),
                        message: format!("record type {}: {msg}", raw.record_type),
                    });
                }
                Err(CadError::CorruptData(msg)) => {
                    // A malformed payload inside a framed record: retain the
                    // exact bytes, emit nothing for this record, and stop.
                    unsupported.push(UnsupportedRecord {
                        record_type: raw.record_type,
                        reason: msg.clone(),
                        data: raw.data.to_vec(),
                    });
                    state.state_reliable = false;
                    stop_reason = Some(format!("malformed record type {}", raw.record_type));
                    diagnostics.push(Diagnostic {
                        object: None,
                        code: "proxy.corrupt".into(),
                        message: format!("record type {}: {msg}", raw.record_type),
                    });
                }
                Err(other) => return Err(other),
            }
        }

        let completeness = match &stop_reason {
            None => Completeness::Complete,
            Some(reason) if geometry.is_empty() => Completeness::Missing(vec![reason.clone()]),
            Some(reason) => Completeness::Partial(vec![format!(
                "{decoded_records} of {} records decoded before {reason}",
                records.len()
            )]),
        };
        Ok(ProxyOutput {
            geometry,
            completeness,
            diagnostics,
            unsupported,
        })
    }

    /// Raw DWG object bytes are *not* a playable metafile (spec §7.2 item 4).
    ///
    /// This entry point exists to make that explicit: it reports the data as
    /// unsupported rather than attempting to misinterpret it as `graphic_data`.
    pub fn inspect_raw_dwg(&self, raw: &[u8]) -> CadResult<ProxyOutput> {
        Ok(ProxyOutput {
            geometry: Vec::new(),
            completeness: Completeness::Unverified,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "proxy.raw_dwg".into(),
                message: format!(
                    "{} raw DWG bytes require a separate, sample-verified record parser; not decoded as graphic_data",
                    raw.len()
                ),
            }],
            unsupported: Vec::new(),
        })
    }
}

fn vertex_count(g: &SemanticGeometry) -> usize {
    match g {
        SemanticGeometry::Polyline { points, .. } => points.len(),
        SemanticGeometry::Mesh(m) => m.vertices.len(),
        SemanticGeometry::Line { .. }
        | SemanticGeometry::Point(_)
        | SemanticGeometry::Text { .. } => 2,
        _ => 4,
    }
}

#[cfg(test)]
mod tests;
