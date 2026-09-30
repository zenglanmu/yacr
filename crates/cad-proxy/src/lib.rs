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
}

impl ProxyOutput {
    pub fn empty_missing(reason: impl Into<String>) -> Self {
        ProxyOutput {
            geometry: Vec::new(),
            completeness: Completeness::Missing(vec![reason.into()]),
            diagnostics: Vec::new(),
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
                if !record.data.is_empty() {
                    return Err(CadError::CorruptData("FillOff record has a payload".into()));
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
                }])
            }
            other => Err(CadError::Unsupported(format!(
                "proxy opcode {other} has no evidence-backed decoder"
            ))),
        }
    }
}

/// Decode `(text, position, height, rotation)` from a type-36 record.
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
    // bytes 24..48 normal, 48..72 direction
    let direction = Point3 {
        x: f(48),
        y: f(56),
        z: f(64),
    };
    let height = f(72);
    // width_factor at 80, oblique at 88 (reserved for later style support)
    let mut units: Vec<u16> = Vec::new();
    let mut i = FIXED;
    while i + 1 < data.len() {
        let u = u16::from_le_bytes([data[i], data[i + 1]]);
        if u == 0 {
            break;
        }
        units.push(u);
        i += 2;
    }
    let rotation = direction.y.atan2(direction.x);
    let text = String::from_utf16_lossy(&units);
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
        let mut unknowns: Vec<u32> = Vec::new();
        let mut vertices = 0usize;
        let mut stopped = false;

        for record in &records {
            if geometry.len() + unknowns.len() >= self.limits.max_records {
                diagnostics.push(Diagnostic {
                    object: None,
                    code: "proxy.limit".into(),
                    message: format!("stopped after {} records (limit)", self.limits.max_records),
                });
                return Ok(ProxyOutput {
                    geometry,
                    completeness: Completeness::Partial(vec!["record limit reached".into()]),
                    diagnostics,
                });
            }
            if !state.state_reliable {
                // Replay stopped; keep counting would need the framing loop, so
                // we stop here and report.
                break;
            }
            let record = ProxyRecord {
                record_type: record.record_type,
                data: record.data,
            };
            match self.decode_record(&record, &mut state) {
                Ok(mut g) => {
                    vertices += g.iter().map(vertex_count).sum::<usize>();
                    if vertices > self.limits.max_vertices {
                        diagnostics.push(Diagnostic {
                            object: None,
                            code: "proxy.limit".into(),
                            message: "vertex limit reached".into(),
                        });
                        geometry.append(&mut g);
                        return Ok(ProxyOutput {
                            geometry,
                            completeness: Completeness::Partial(
                                vec!["vertex limit reached".into()],
                            ),
                            diagnostics,
                        });
                    }
                    geometry.append(&mut g);
                }
                Err(CadError::Unsupported(msg)) => {
                    // Unknown opcode: an unrecognised opcode may be a state
                    // instruction affecting everything after it.
                    unknowns.push(record.record_type);
                    state.state_reliable = false;
                    stopped = true;
                    diagnostics.push(Diagnostic {
                        object: None,
                        code: "proxy.unknown_opcode".into(),
                        message: format!("record type {}: {msg}", record.record_type),
                    });
                }
                Err(CadError::CorruptData(msg)) => {
                    diagnostics.push(Diagnostic {
                        object: None,
                        code: "proxy.corrupt".into(),
                        message: msg,
                    });
                    return Ok(ProxyOutput {
                        geometry,
                        completeness: Completeness::Partial(vec!["malformed proxy record".into()]),
                        diagnostics,
                    });
                }
                Err(other) => return Err(other),
            }
        }

        let completeness = if unknowns.is_empty() && !stopped {
            Completeness::Complete
        } else if geometry.is_empty() {
            Completeness::Missing(vec![format!("unrecognised proxy opcodes: {unknowns:?}")])
        } else {
            Completeness::Partial(vec![format!(
                "{} records decoded before unrecognised opcode(s) {unknowns:?}",
                records.len()
            )])
        };
        Ok(ProxyOutput {
            geometry,
            completeness,
            diagnostics,
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
mod tests {
    use super::*;
    use metafile::encode_for_test;

    fn source() -> ProxySource {
        ProxySource {
            handle: "1A".into(),
            class_name: "TCH_WALL".into(),
            application: Some("Tianzheng".into()),
            dwg_version: "AC1027".into(),
        }
    }

    fn text_record(text: &str) -> Vec<u8> {
        let mut payload = vec![0u8; KnownOpcodeDecoder::UNICODE_TEXT_FIXED];
        // position (0,0,0), direction (1,0,0), height 2.5
        payload[72..80].copy_from_slice(&2.5f64.to_le_bytes());
        payload[48..56].copy_from_slice(&1.0f64.to_le_bytes());
        for u in text.encode_utf16() {
            payload.extend_from_slice(&u.to_le_bytes());
        }
        payload.extend_from_slice(&0u16.to_le_bytes());
        encode_for_test(36, &payload)
    }

    #[test]
    fn known_records_replay_to_geometry() {
        let data = metafile::concat(&[encode_for_test(21, &[]), text_record("room")]);
        let out = ProxyPlayer::default().replay(&source(), &data).unwrap();
        assert_eq!(out.geometry.len(), 1);
        assert_eq!(out.completeness, Completeness::Complete);
        match &out.geometry[0] {
            SemanticGeometry::Text { text, .. } => assert_eq!(text, "room"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn unknown_opcode_degrades_and_flags_incomplete() {
        let data = metafile::concat(&[encode_for_test(999, &[1u8, 2, 3, 4]), text_record("after")]);
        let out = ProxyPlayer::default().replay(&source(), &data).unwrap();
        assert!(out.geometry.is_empty());
        assert!(matches!(out.completeness, Completeness::Missing(_)));
        assert!(out
            .diagnostics
            .iter()
            .any(|d| d.code == "proxy.unknown_opcode"));
    }

    #[test]
    fn empty_cache_is_missing_not_faked() {
        let out = ProxyPlayer::default().replay(&source(), &[]).unwrap();
        assert!(out.geometry.is_empty());
        assert!(matches!(out.completeness, Completeness::Missing(_)));
    }

    #[test]
    fn oversize_input_is_rejected() {
        let limits = DecodeLimits {
            max_bytes: 4,
            ..Default::default()
        };
        let player = ProxyPlayer::new(limits);
        assert!(player.replay(&source(), &[0u8; 32]).is_err());
    }

    #[test]
    fn raw_dwg_bytes_are_not_treated_as_graphic_data() {
        let out = ProxyPlayer::default().inspect_raw_dwg(&[0xAB; 64]).unwrap();
        assert!(out.geometry.is_empty());
        assert_eq!(out.completeness, Completeness::Unverified);
    }
}
