//! Vendor-entity decoding pass state: the carrier-neutral half of the
//! **pending → records → apply** channel used by WebView-hosted plugins.
//!
//! # Why this lives in `cad-ui-core`
//!
//! The state machine has no Tauri, DOM, GPU or document authority: it is a
//! bounded token/frame validator plus the stable `vendor.*` refusal vocabulary.
//! Every carrier needs the **same** rules, so it lives here and is reused by
//! `cad-desktop-shell` (all `native-below` hosts) and by the `dom-canvas` wasm
//! session in `apps/app-web`. A per-carrier copy would drift on token, framing
//! and `applied` semantics.
//!
//! # The pass
//!
//! Tianzheng (`TCH_*`) and other vendor objects are opaque to the importer, and
//! the decoder is shipped as a WebView plugin (TypeScript or wasm). Neither a
//! native host nor a wasm session can call into the WebView plugin itself, so the
//! pass is deliberately **two-phase** and asynchronous:
//!
//! 1. a document opens and the importer copies every `TCH_*` record;
//! 2. the owner tick (native) / the open path (`dom-canvas`) stages those bytes
//!    and the next event flush emits
//!    `vendor-pending { token, generation, entities }`;
//! 3. the frontend asks `cad_vendor_records(token)` for the binary record frame,
//!    decodes it, and posts the binary result back through `cad_vendor_apply`;
//! 4. the carrier validates token/generation/limits and settles the pass; the
//!    next flush emits `vendor-settled { token, applied, diagnostics }`.
//!
//! # What a settled frame means
//!
//! ABI v4 carries real primitives. `applied` must equal the number of parsed
//! primitives, a pass with records may not settle as an empty success, and a
//! non-zero `applied` is therefore real derived geometry rather than a claim.
//! The primitives are display-only; `cad-db` stays authoritative.
//!
//! # Failure closure
//!
//! Every rejection is a stable `vendor.*` reason: unknown/stale token, already
//! settled, oversized payload, corrupt framing, unsupported ABI, or a count that
//! does not match the primitives. No partial result is accepted.

use super::CadSession;

use std::fmt;
use std::sync::Arc;

use cad_app::vendor::{encode_records_frame, VendorEntityPrimitives, VendorPrimitive, VendorText};
use cad_domain::Point3;
use serde::Serialize;

/// Version of the vendor wire frames. Bumped only for a breaking layout change.
pub use cad_app::vendor::{RECORDS_MAGIC, VENDOR_ABI_VERSION};

/// Result frame magic (`"YANR"`, little-endian).
pub const RESULT_MAGIC: u32 = 0x5941_4e52;

/// Upper bound of entity groups in one result frame.
pub const MAX_ENTITY_GROUPS: usize = 200_000;

/// Upper bound of primitives in one result frame.
pub const MAX_PRIMITIVE_COUNT: usize = 2_000_000;

/// Upper bound of points in one primitive.
pub const MAX_PRIMITIVE_POINTS: usize = 1_000_000;

/// Upper bound of UTF-8 bytes in one text primitive.
pub const MAX_TEXT_BYTES: usize = 4096;

/// Upper bound of one block-insert scale factor.
///
/// A malicious or corrupt frame must not be able to request an expansion that
/// overflows the scene float range; real symbol blocks scale by a wall width.
pub const MAX_BLOCK_SCALE: f64 = 1.0e6;

/// Upper bound of one result payload (base64-decoded bytes).
pub const MAX_RESULT_BYTES: usize = 4 * 1024 * 1024;

/// Upper bound of diagnostics carried by one result frame.
pub const MAX_DIAGNOSTICS: usize = 16;

/// Upper bound of one diagnostic string, in bytes.
pub const MAX_DIAGNOSTIC_BYTES: usize = 512;

/// Upper bound of the `applied` count a plugin may claim.
pub const MAX_APPLIED: u32 = 2_000_000;

/// The current pass descriptor, served to a chrome that connected late.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VendorStatus {
    /// The active pass token, or `None` when no pass is armed.
    pub token: Option<u64>,
    /// The pass generation (echoed by the result frame).
    pub generation: Option<u64>,
    /// Vendor records in the active frame.
    pub entities: u32,
}

/// The acknowledgement returned to the frontend after `cad_vendor_apply`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VendorAck {
    /// Whether the pass settled without a protocol rejection.
    pub accepted: bool,
    /// Primitives the host accepted into the derived-representation store.
    pub applied: u32,
    /// Stable diagnostics from the plugin or the host.
    pub diagnostics: Vec<String>,
}

/// One settled result plus the derived primitives it carried.
///
/// `ack` is what the frontend sees; `primitives` is what the carrier hands to its
/// derived-representation store. Both come from the same parse, so a frame cannot
/// report an `applied` count the primitive list does not support.
#[derive(Debug, Clone, PartialEq)]
pub struct VendorSettlement {
    pub ack: VendorAck,
    pub primitives: Vec<VendorEntityPrimitives>,
}

/// One armed pass: the only token/generation pair the host will accept.
#[derive(Debug, Clone, PartialEq)]
struct VendorPass {
    token: u64,
    generation: u64,
    records_frame: Vec<u8>,
    entities: u32,
    announced: bool,
    /// The first valid settlement, retained so a repeated apply returns the same
    /// primitives instead of overwriting the derived snapshot with an empty one.
    settled: Option<VendorSettlement>,
}

/// The host's vendor pass state.
///
/// One pass at a time: a new document replaces the previous pass, and a stale
/// token can never settle into the new document.
#[derive(Debug, Default)]
pub struct VendorStore {
    next_token: u64,
    /// Whether this carrier registered/serves the vendor commands. A carrier that
    /// does not stays false, so no `vendor-pending` event can point at a command
    /// that is not there; every native host and the `dom-canvas` wasm session
    /// enable it.
    enabled: bool,
    active: Option<VendorPass>,
    /// A settled result the owner tick has not emitted yet.
    unreported: Option<(u64, VendorAck)>,
}

impl VendorStore {
    /// Enable the pass on a host that registered the three vendor commands.
    pub fn enable(&mut self) {
        self.enabled = true;
    }

    /// Whether the pass is enabled on this host.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Stage the records copied by the last import.
    ///
    /// Returns `(token, generation)`. A host that never enabled the pass refuses
    /// explicitly instead of building a frame nothing can fetch.
    pub fn stage(&mut self, records: Vec<cad_app::VendorRecord>) -> Result<(u64, u64), String> {
        if !self.enabled {
            return Err("vendor.disabled".to_string());
        }
        self.next_token = self.next_token.saturating_add(1);
        let token = self.next_token;
        let generation = token;
        let frame = encode_records_frame(token, generation, &records)?;
        self.active = Some(VendorPass {
            token,
            generation,
            records_frame: frame,
            entities: records.len().min(u32::MAX as usize) as u32,
            announced: false,
            settled: None,
        });
        self.unreported = None;
        Ok((token, generation))
    }

    /// Report a staging failure through the next `vendor-settled` event.
    pub fn report_failure(&mut self, reason: impl Into<String>) {
        self.unreported = Some((
            self.next_token,
            VendorAck {
                accepted: false,
                applied: 0,
                diagnostics: vec![reason.into()],
            },
        ));
    }

    /// Forget the active pass (document closed / replaced).
    pub fn clear(&mut self) {
        self.active = None;
        self.unreported = None;
    }

    /// The active `(token, generation)`, if any.
    pub fn active(&self) -> Option<(u64, u64)> {
        self.active
            .as_ref()
            .map(|pass| (pass.token, pass.generation))
    }

    /// The pass descriptor a late-connecting chrome can recover from.
    pub fn status(&self) -> VendorStatus {
        match self.active.as_ref() {
            Some(pass) => VendorStatus {
                token: Some(pass.token),
                generation: Some(pass.generation),
                entities: pass.entities,
            },
            None => VendorStatus {
                token: None,
                generation: None,
                entities: 0,
            },
        }
    }

    /// Take the pending announcement, at most once per pass.
    pub fn announce_pending(&mut self) -> Option<(u64, u64, u32)> {
        let pass = self.active.as_mut()?;
        if pass.announced {
            return None;
        }
        pass.announced = true;
        Some((pass.token, pass.generation, pass.entities))
    }

    /// Take the settled result the owner tick has not emitted yet.
    pub fn take_unreported(&mut self) -> Option<(u64, VendorAck)> {
        self.unreported.take()
    }

    /// The records frame for `token`.
    pub fn records_frame(&self, token: u64) -> Result<Vec<u8>, String> {
        let pass = self
            .active
            .as_ref()
            .ok_or_else(|| "vendor.no_pending_pass".to_string())?;
        if pass.token != token {
            return Err("vendor.stale_token".to_string());
        }
        if pass.settled.is_some() {
            return Err("vendor.already_settled".to_string());
        }
        Ok(pass.records_frame.clone())
    }

    /// Settle the pass from a base64-encoded result frame, discarding primitives.
    ///
    /// Kept for callers that only need the acknowledgement; carriers that render
    /// the derived geometry call [`Self::settle_base64_full`].
    pub fn settle_base64(
        &mut self,
        token: u64,
        generation: u64,
        payload: &str,
    ) -> Result<VendorAck, String> {
        self.settle_base64_full(token, generation, payload)
            .map(|settlement| settlement.ack)
    }

    /// Settle the pass from an ABI v4 result frame, returning its primitives.
    pub fn settle_base64_full(
        &mut self,
        token: u64,
        generation: u64,
        payload: &str,
    ) -> Result<VendorSettlement, String> {
        if payload.len() > MAX_RESULT_BYTES.saturating_mul(4).saturating_div(3) + 8 {
            return Err("vendor.payload_too_large".to_string());
        }
        let bytes = decode_base64(payload).map_err(|error| format!("vendor.base64: {error}"))?;
        self.settle_bytes_full(token, generation, &bytes)
    }

    /// Settle the pass from a result frame, discarding primitives.
    pub fn settle_bytes(
        &mut self,
        token: u64,
        generation: u64,
        bytes: &[u8],
    ) -> Result<VendorAck, String> {
        self.settle_bytes_full(token, generation, bytes)
            .map(|settlement| settlement.ack)
    }

    /// Settle the pass from an ABI v4 result frame.
    ///
    /// The frame is validated against the active token/generation and the frozen
    /// ABI. A protocol rejection does **not** settle the pass, so the frontend can
    /// retry with a corrected frame; a valid frame settles it exactly once.
    ///
    /// `applied` must equal the number of parsed primitives, and a document whose
    /// pass carried records may not settle as an empty success. A non-zero
    /// `applied` is therefore real derived geometry, not an unsupported claim.
    pub fn settle_bytes_full(
        &mut self,
        token: u64,
        generation: u64,
        bytes: &[u8],
    ) -> Result<VendorSettlement, String> {
        let pass = self
            .active
            .as_mut()
            .ok_or_else(|| "vendor.no_pending_pass".to_string())?;
        if pass.token != token {
            return Err("vendor.stale_token".to_string());
        }
        if pass.generation != generation {
            return Err("vendor.stale_generation".to_string());
        }
        if let Some(settlement) = &pass.settled {
            return Ok(settlement.clone());
        }
        let parsed = parse_result_frame(bytes)?;
        if parsed.token != token {
            return Err("vendor.frame_token_mismatch".to_string());
        }
        if parsed.generation != generation {
            return Err("vendor.frame_generation_mismatch".to_string());
        }
        let total = parsed
            .primitives
            .iter()
            .map(|entity| entity.primitives.len())
            .sum::<usize>();
        if parsed.applied as usize != total {
            return Err("vendor.applied_mismatch".to_string());
        }
        if parsed.applied > MAX_APPLIED {
            return Err("vendor.applied_out_of_range".to_string());
        }
        let ack = match parsed.status {
            0 => {
                if parsed.applied == 0 && pass.entities > 0 {
                    // The plugin was given records yet claims a clean empty
                    // success. That is a fake result, not "nothing to draw".
                    return Err("vendor.empty_success".to_string());
                }
                VendorAck {
                    accepted: true,
                    applied: parsed.applied,
                    diagnostics: parsed.diagnostics,
                }
            }
            1 => {
                if parsed.applied != 0 || !parsed.primitives.is_empty() {
                    return Err("vendor.unsupported_with_primitives".to_string());
                }
                VendorAck {
                    accepted: false,
                    applied: 0,
                    diagnostics: if parsed.diagnostics.is_empty() {
                        vec!["vendor.unsupported".to_string()]
                    } else {
                        parsed.diagnostics
                    },
                }
            }
            _ => return Err("vendor.invalid_status".to_string()),
        };
        let settlement = VendorSettlement {
            ack: ack.clone(),
            primitives: parsed.primitives,
        };
        pass.settled = Some(settlement.clone());
        self.unreported = Some((token, ack));
        Ok(settlement)
    }
}

/// One parsed result frame.
#[derive(Debug, Clone, PartialEq)]
struct ParsedResult {
    token: u64,
    generation: u64,
    status: u32,
    applied: u32,
    diagnostics: Vec<String>,
    primitives: Vec<VendorEntityPrimitives>,
}

fn finite(value: f64) -> bool {
    value.is_finite() && value.abs() <= 1e12
}

fn point(x: f64, y: f64, z: f64) -> Result<Point3, String> {
    if !finite(x) || !finite(y) || !finite(z) {
        return Err("vendor.nonfinite_primitive".to_string());
    }
    Ok(Point3 { x, y, z })
}

fn parse_result_frame(bytes: &[u8]) -> Result<ParsedResult, String> {
    if bytes.len() > MAX_RESULT_BYTES {
        return Err("vendor.payload_too_large".to_string());
    }
    let mut cursor = Cursor::new(bytes);
    let magic = cursor.u32()?;
    if magic != RESULT_MAGIC {
        return Err("vendor.bad_magic".to_string());
    }
    let abi = cursor.u32()?;
    if abi != VENDOR_ABI_VERSION {
        return Err(format!("vendor.unsupported_abi: {abi}"));
    }
    let token = cursor.u64()?;
    let generation = cursor.u64()?;
    let status = cursor.u32()?;
    let applied = cursor.u32()?;
    let count = cursor.u32()?;
    if count as usize > MAX_DIAGNOSTICS {
        return Err("vendor.too_many_diagnostics".to_string());
    }
    let mut diagnostics = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let length = cursor.u16()? as usize;
        if length > MAX_DIAGNOSTIC_BYTES {
            return Err("vendor.diagnostic_too_long".to_string());
        }
        let text = std::str::from_utf8(cursor.bytes(length)?)
            .map_err(|_| "vendor.diagnostic_not_utf8".to_string())?;
        diagnostics.push(text.to_string());
    }
    let entity_count = cursor.u32()? as usize;
    if entity_count > MAX_ENTITY_GROUPS {
        return Err("vendor.too_many_entities".to_string());
    }
    let mut primitives = Vec::with_capacity(entity_count);
    let mut total = 0usize;
    for _ in 0..entity_count {
        let handle = cursor.u64()?;
        let primitive_count = cursor.u32()? as usize;
        if primitive_count > MAX_PRIMITIVE_COUNT {
            return Err("vendor.too_many_primitives".to_string());
        }
        total = total.saturating_add(primitive_count);
        if total > MAX_PRIMITIVE_COUNT {
            return Err("vendor.too_many_primitives".to_string());
        }
        let mut entity = Vec::with_capacity(primitive_count);
        for _ in 0..primitive_count {
            let kind = cursor.u8()?;
            entity.push(parse_primitive(&mut cursor, kind)?);
        }
        primitives.push(VendorEntityPrimitives {
            handle,
            primitives: entity,
        });
    }
    if cursor.remaining() != 0 {
        return Err("vendor.trailing_bytes".to_string());
    }
    Ok(ParsedResult {
        token,
        generation,
        status,
        applied,
        diagnostics,
        primitives,
    })
}

fn parse_primitive(cursor: &mut Cursor<'_>, kind: u8) -> Result<VendorPrimitive, String> {
    match kind {
        0 | 1 => {
            let point_count = cursor.u32()? as usize;
            if point_count > MAX_PRIMITIVE_POINTS {
                return Err("vendor.too_many_points".to_string());
            }
            if point_count < 2 || (kind == 1 && point_count % 2 != 0) {
                return Err("vendor.invalid_point_count".to_string());
            }
            let mut points = Vec::with_capacity(point_count);
            for _ in 0..point_count {
                let x = cursor.f64()?;
                let y = cursor.f64()?;
                let z = cursor.f64()?;
                points.push(point(x, y, z)?);
            }
            if kind == 0 {
                Ok(VendorPrimitive::Lines(points))
            } else {
                Ok(VendorPrimitive::Segments(points))
            }
        }
        2 => {
            let center = point(cursor.f64()?, cursor.f64()?, cursor.f64()?)?;
            let radius = cursor.f64()?;
            let start = cursor.f64()?;
            let sweep = cursor.f64()?;
            if !finite(radius) || radius <= 0.0 || !finite(start) || !finite(sweep) || sweep == 0.0
            {
                return Err("vendor.invalid_arc".to_string());
            }
            Ok(VendorPrimitive::Arc {
                center,
                radius,
                start,
                sweep,
            })
        }
        3 => {
            let center = point(cursor.f64()?, cursor.f64()?, cursor.f64()?)?;
            let radius = cursor.f64()?;
            if !finite(radius) || radius <= 0.0 {
                return Err("vendor.invalid_circle".to_string());
            }
            Ok(VendorPrimitive::Circle { center, radius })
        }
        4 => {
            let origin = point(cursor.f64()?, cursor.f64()?, cursor.f64()?)?;
            let height = cursor.f64()?;
            let rotation = cursor.f64()?;
            let wrap_width = cursor.f64()?;
            let attachment = cursor.u8()?;
            let length = cursor.u16()? as usize;
            if length == 0 || length > MAX_TEXT_BYTES {
                return Err("vendor.invalid_text_length".to_string());
            }
            let text = std::str::from_utf8(cursor.bytes(length)?)
                .map_err(|_| "vendor.text_not_utf8".to_string())?
                .to_string();
            if text.is_empty()
                || !finite(height)
                || height <= 0.0
                || !finite(rotation)
                || !finite(wrap_width)
                || wrap_width < 0.0
                || !(1..=9).contains(&attachment)
            {
                return Err("vendor.invalid_text".to_string());
            }
            Ok(VendorPrimitive::Text(VendorText {
                origin,
                height,
                rotation,
                wrap_width: (wrap_width > 0.0).then_some(wrap_width),
                attachment,
                text,
            }))
        }
        5 => {
            let block = cursor.u64()?;
            let origin = point(cursor.f64()?, cursor.f64()?, cursor.f64()?)?;
            let scale = point(cursor.f64()?, cursor.f64()?, cursor.f64()?)?;
            let rotation = cursor.f64()?;
            if block == 0
                || !finite(rotation)
                || [scale.x, scale.y, scale.z]
                    .iter()
                    .any(|value| !finite(*value) || *value == 0.0 || value.abs() > MAX_BLOCK_SCALE)
            {
                return Err("vendor.invalid_block".to_string());
            }
            Ok(VendorPrimitive::Block {
                block,
                origin,
                scale,
                rotation,
            })
        }
        other => Err(format!("vendor.unsupported_primitive: {other}")),
    }
}

/// A tiny little-endian reader with exact bounds.
struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn bytes(&mut self, length: usize) -> Result<&'a [u8], String> {
        let end = self
            .position
            .checked_add(length)
            .ok_or_else(|| "vendor.truncated".to_string())?;
        if end > self.bytes.len() {
            return Err("vendor.truncated".to_string());
        }
        let slice = &self.bytes[self.position..end];
        self.position = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.bytes(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, String> {
        let bytes = self.bytes(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn f64(&mut self) -> Result<f64, String> {
        let bytes = self.bytes(8)?;
        let mut value = [0u8; 8];
        value.copy_from_slice(bytes);
        Ok(f64::from_le_bytes(value))
    }

    fn u32(&mut self) -> Result<u32, String> {
        let bytes = self.bytes(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn u64(&mut self) -> Result<u64, String> {
        let bytes = self.bytes(8)?;
        let mut value = [0u8; 8];
        value.copy_from_slice(bytes);
        Ok(u64::from_le_bytes(value))
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.position)
    }
}

fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    const TABLE: [i8; 128] = {
        let mut table = [-1i8; 128];
        let mut index = 0usize;
        while index < 26 {
            table[b'A' as usize + index] = index as i8;
            table[b'a' as usize + index] = (26 + index) as i8;
            index += 1;
        }
        let mut digit = 0usize;
        while digit < 10 {
            table[b'0' as usize + digit] = (52 + digit) as i8;
            digit += 1;
        }
        table[b'+' as usize] = 62;
        table[b'/' as usize] = 63;
        table
    };
    let mut output = Vec::with_capacity(input.len().saturating_mul(3) / 4);
    let mut buffer = 0u32;
    let mut bits = 0u32;
    let mut padding = 0usize;
    for byte in input.bytes() {
        if byte == b'=' {
            padding += 1;
            continue;
        }
        if padding > 0 {
            return Err("padding before end".to_string());
        }
        let value = *TABLE
            .get(byte as usize)
            .ok_or_else(|| "invalid character".to_string())?;
        if value < 0 {
            return Err("invalid character".to_string());
        }
        buffer = (buffer << 6) | value as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    if padding > 2 {
        return Err("too much padding".to_string());
    }
    Ok(output)
}

impl fmt::Display for VendorAck {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "vendor-ack accepted={} applied={} diagnostics={}",
            self.accepted,
            self.applied,
            self.diagnostics.len()
        )
    }
}

impl CadSession {
    /// Install the vendor derived-representation snapshot for the next scene
    /// preparation. Display-only: `cad-db` stays authoritative.
    pub fn set_vendor_representations(
        &self,
        store: Option<Arc<cad_app::vendor::VendorRepresentationStore>>,
    ) {
        self.core.set_vendor_representations(store);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_app::{VendorRecord, VendorSource};

    fn record(handle: u64) -> VendorRecord {
        VendorRecord {
            class_name: "TCH_WALL".to_string(),
            handle,
            dwg_version: 1027,
            source: VendorSource::UnknownEntity,
            main: vec![0x06, 0b1010_0000],
            main_bits: 10,
            text: vec![0b1100_0000],
            text_bits: 2,
            handle_stream: vec![0b0100_0000],
            handle_bits: 2,
            main_skip_bits: 3,
            text_skip_bits: 0,
            handle_skip_bits: 0,
            references: vec![0x2c],
        }
    }

    fn result_frame(
        token: u64,
        generation: u64,
        status: u32,
        applied: u32,
        diagnostics: &[&str],
    ) -> Vec<u8> {
        result_frame_with(token, generation, status, applied, diagnostics, &[])
    }

    fn result_frame_with(
        token: u64,
        generation: u64,
        status: u32,
        applied: u32,
        diagnostics: &[&str],
        entities: &[(u64, Vec<VendorPrimitive>)],
    ) -> Vec<u8> {
        let mut frame = Vec::new();
        frame.extend_from_slice(&RESULT_MAGIC.to_le_bytes());
        frame.extend_from_slice(&VENDOR_ABI_VERSION.to_le_bytes());
        frame.extend_from_slice(&token.to_le_bytes());
        frame.extend_from_slice(&generation.to_le_bytes());
        frame.extend_from_slice(&status.to_le_bytes());
        frame.extend_from_slice(&applied.to_le_bytes());
        frame.extend_from_slice(&(diagnostics.len() as u32).to_le_bytes());
        for diagnostic in diagnostics {
            frame.extend_from_slice(&(diagnostic.len() as u16).to_le_bytes());
            frame.extend_from_slice(diagnostic.as_bytes());
        }
        frame.extend_from_slice(&(entities.len() as u32).to_le_bytes());
        for (handle, primitives) in entities {
            frame.extend_from_slice(&handle.to_le_bytes());
            frame.extend_from_slice(&(primitives.len() as u32).to_le_bytes());
            for primitive in primitives {
                match primitive {
                    VendorPrimitive::Lines(points) => {
                        frame.push(0);
                        frame.extend_from_slice(&(points.len() as u32).to_le_bytes());
                        for point in points {
                            for value in [point.x, point.y, point.z] {
                                frame.extend_from_slice(&value.to_le_bytes());
                            }
                        }
                    }
                    VendorPrimitive::Segments(points) => {
                        frame.push(1);
                        frame.extend_from_slice(&(points.len() as u32).to_le_bytes());
                        for point in points {
                            for value in [point.x, point.y, point.z] {
                                frame.extend_from_slice(&value.to_le_bytes());
                            }
                        }
                    }
                    VendorPrimitive::Arc {
                        center,
                        radius,
                        start,
                        sweep,
                    } => {
                        frame.push(2);
                        for value in [center.x, center.y, center.z, *radius, *start, *sweep] {
                            frame.extend_from_slice(&value.to_le_bytes());
                        }
                    }
                    VendorPrimitive::Circle { center, radius } => {
                        frame.push(3);
                        for value in [center.x, center.y, center.z, *radius] {
                            frame.extend_from_slice(&value.to_le_bytes());
                        }
                    }
                    VendorPrimitive::Text(text) => {
                        frame.push(4);
                        for value in [
                            text.origin.x,
                            text.origin.y,
                            text.origin.z,
                            text.height,
                            text.rotation,
                            text.wrap_width.unwrap_or(0.0),
                        ] {
                            frame.extend_from_slice(&value.to_le_bytes());
                        }
                        frame.push(text.attachment);
                        let bytes = text.text.as_bytes();
                        frame.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
                        frame.extend_from_slice(bytes);
                    }
                    VendorPrimitive::Block {
                        block,
                        origin,
                        scale,
                        rotation,
                    } => {
                        frame.push(5);
                        frame.extend_from_slice(&block.to_le_bytes());
                        for value in [
                            origin.x, origin.y, origin.z, scale.x, scale.y, scale.z, *rotation,
                        ] {
                            frame.extend_from_slice(&value.to_le_bytes());
                        }
                    }
                }
            }
        }
        frame
    }

    fn line(handle: u64) -> (u64, Vec<VendorPrimitive>) {
        (
            handle,
            vec![VendorPrimitive::Lines(vec![
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: 10.0,
                    y: 0.0,
                    z: 0.0,
                },
            ])],
        )
    }

    #[test]
    fn records_frame_is_the_frozen_v4_layout() {
        let mut store = VendorStore::default();
        store.enable();
        let (token, generation) = store.stage(vec![record(0x2c)]).expect("stage");
        let frame = store.records_frame(token).expect("active token");
        assert_eq!(&frame[0..4], &RECORDS_MAGIC.to_le_bytes());
        assert_eq!(&frame[4..8], &VENDOR_ABI_VERSION.to_le_bytes());
        assert_eq!(&frame[8..16], &token.to_le_bytes());
        assert_eq!(&frame[16..24], &generation.to_le_bytes());
        assert_eq!(&frame[24..28], &1u32.to_le_bytes());
        let class_len = u16::from_le_bytes([frame[28], frame[29]]) as usize;
        assert_eq!(&frame[30..30 + class_len], b"TCH_WALL");
        assert_eq!(
            store.records_frame(token + 1),
            Err("vendor.stale_token".into())
        );
    }

    #[test]
    fn staging_without_enable_is_refused() {
        let mut store = VendorStore::default();
        assert_eq!(
            store.stage(vec![record(1)]),
            Err("vendor.disabled".to_string())
        );
    }

    #[test]
    fn a_primitive_result_settles_once_and_reports() {
        let mut store = VendorStore::default();
        store.enable();
        let (token, generation) = store.stage(vec![record(1)]).expect("stage");
        assert_eq!(
            store.announce_pending().map(|(t, _, n)| (t, n)),
            Some((token, 1))
        );
        assert_eq!(store.announce_pending(), None, "announced once");
        let entity = line(1);
        let frame = result_frame_with(token, generation, 0, 1, &[], &[entity]);
        let settlement = store
            .settle_bytes_full(token, generation, &frame)
            .expect("settles");
        assert!(settlement.ack.accepted);
        assert_eq!(settlement.ack.applied, 1);
        assert_eq!(settlement.primitives.len(), 1);
        assert_eq!(settlement.primitives[0].primitives.len(), 1);
        assert_eq!(store.take_unreported().map(|(t, _)| t), Some(token));
        assert_eq!(store.take_unreported(), None, "reported only once");
        let again = store
            .settle_bytes(token, generation, &frame)
            .expect("same ack");
        assert_eq!(again, settlement.ack);
    }

    #[test]
    fn a_block_insert_primitive_keeps_its_handle_scale_and_rotation() {
        let mut store = VendorStore::default();
        store.enable();
        let (token, generation) = store.stage(vec![record(1)]).expect("stage");
        let frame = result_frame_with(
            token,
            generation,
            0,
            1,
            &[],
            &[(
                1,
                vec![VendorPrimitive::Block {
                    block: 0x3dd82,
                    origin: Point3 {
                        x: 1.0,
                        y: 2.0,
                        z: 0.0,
                    },
                    scale: Point3 {
                        x: 2.0,
                        y: -0.5,
                        z: 2.0,
                    },
                    rotation: 0.25,
                }],
            )],
        );
        let settlement = store
            .settle_bytes_full(token, generation, &frame)
            .expect("settles");
        assert!(settlement.ack.accepted);
        match &settlement.primitives[0].primitives[0] {
            VendorPrimitive::Block {
                block,
                scale,
                rotation,
                ..
            } => {
                assert_eq!(*block, 0x3dd82);
                assert_eq!(scale.y, -0.5, "the mirror scale is preserved");
                assert_eq!(*rotation, 0.25);
            }
            other => panic!("expected a block primitive, got {other:?}"),
        }
    }

    #[test]
    fn a_block_insert_with_a_zero_scale_is_rejected() {
        let mut store = VendorStore::default();
        store.enable();
        let (token, generation) = store.stage(vec![record(1)]).expect("stage");
        let frame = result_frame_with(
            token,
            generation,
            0,
            1,
            &[],
            &[(
                1,
                vec![VendorPrimitive::Block {
                    block: 0x3dd82,
                    origin: Point3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    scale: Point3 {
                        x: 0.0,
                        y: 1.0,
                        z: 1.0,
                    },
                    rotation: 0.0,
                }],
            )],
        );
        assert_eq!(
            store.settle_bytes_full(token, generation, &frame),
            Err("vendor.invalid_block".to_string())
        );
    }

    #[test]
    fn stale_tokens_and_generations_are_rejected() {
        let mut store = VendorStore::default();
        store.enable();
        let (token, generation) = store.stage(vec![record(1)]).expect("stage");
        let frame = result_frame_with(token, generation, 0, 1, &[], &[line(1)]);
        assert_eq!(
            store.settle_bytes(token + 1, generation, &frame),
            Err("vendor.stale_token".into())
        );
        assert_eq!(
            store.settle_bytes(token, generation + 1, &frame),
            Err("vendor.stale_generation".into())
        );
        store.clear();
        assert_eq!(
            store.settle_bytes(token, generation, &frame),
            Err("vendor.no_pending_pass".into())
        );
    }

    #[test]
    fn corrupt_frames_are_rejected_in_order() {
        let mut store = VendorStore::default();
        store.enable();
        let (token, generation) = store.stage(vec![record(1)]).expect("stage");
        let mut bad_magic = result_frame_with(token, generation, 0, 1, &[], &[line(1)]);
        bad_magic[0] ^= 0xff;
        assert_eq!(
            store.settle_bytes(token, generation, &bad_magic),
            Err("vendor.bad_magic".into())
        );
        let mut bad_abi = result_frame_with(token, generation, 0, 1, &[], &[line(1)]);
        bad_abi[4..8].copy_from_slice(&(VENDOR_ABI_VERSION + 1).to_le_bytes());
        assert!(store
            .settle_bytes(token, generation, &bad_abi)
            .unwrap_err()
            .starts_with("vendor.unsupported_abi"));
        let mut wrong_token = result_frame_with(token + 1, generation, 0, 1, &[], &[line(1)]);
        wrong_token[8..16].copy_from_slice(&(token + 1).to_le_bytes());
        assert_eq!(
            store.settle_bytes(token, generation, &wrong_token),
            Err("vendor.frame_token_mismatch".into())
        );
        assert_eq!(
            store.settle_bytes(
                token,
                generation,
                &result_frame(token, generation, 7, 0, &[])
            ),
            Err("vendor.invalid_status".into())
        );
        assert_eq!(
            store.settle_bytes(
                token,
                generation,
                &result_frame(token, generation, 0, 1, &[])
            ),
            Err("vendor.applied_mismatch".into())
        );
    }

    #[test]
    #[ignore]
    fn parses_a_real_plugin_result() {
        let Ok(path) = std::env::var("YACR_VENDOR_RESULT") else {
            return;
        };
        let bytes = std::fs::read(path).expect("read result frame");
        let mut store = VendorStore::default();
        store.enable();
        let (token, generation) = store.stage(vec![record(1)]).expect("stage");
        let settlement = store
            .settle_bytes_full(token, generation, &bytes)
            .expect("real plugin frame settles");
        let total = settlement
            .primitives
            .iter()
            .map(|entity| entity.primitives.len())
            .sum::<usize>();
        eprintln!(
            "real plugin result: applied={} entities={} primitives={} diagnostics={}",
            settlement.ack.applied,
            settlement.primitives.len(),
            total,
            settlement.ack.diagnostics.len()
        );
        assert_eq!(settlement.ack.applied as usize, total);
        assert!(total > 0);
    }

    #[test]
    fn a_record_pass_may_not_settle_as_an_empty_success() {
        let mut store = VendorStore::default();
        store.enable();
        let (token, generation) = store.stage(vec![record(1)]).expect("stage");
        assert_eq!(
            store.settle_bytes(
                token,
                generation,
                &result_frame(token, generation, 0, 0, &[])
            ),
            Err("vendor.empty_success".into())
        );
    }

    #[test]
    fn diagnostics_are_bounded_and_eof_is_exact() {
        let mut store = VendorStore::default();
        store.enable();
        let (token, generation) = store.stage(vec![record(1)]).expect("stage");
        let many: Vec<String> = (0..=MAX_DIAGNOSTICS).map(|i| i.to_string()).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        assert_eq!(
            store.settle_bytes(
                token,
                generation,
                &result_frame(token, generation, 0, 0, &refs)
            ),
            Err("vendor.too_many_diagnostics".into())
        );

        let mut trailing = result_frame(token, generation, 0, 0, &[]);
        trailing.push(0);
        assert_eq!(
            store.settle_bytes(token, generation, &trailing),
            Err("vendor.trailing_bytes".into())
        );

        let unsupported = result_frame(token, generation, 1, 0, &["vendor.unsupported_layout"]);
        let ack = store
            .settle_bytes(token, generation, &unsupported)
            .expect("settles as unsupported");
        assert!(!ack.accepted);
        assert_eq!(ack.diagnostics, ["vendor.unsupported_layout"]);
    }

    #[test]
    fn base64_round_trips_and_rejects_garbage() {
        let bytes = result_frame(1, 2, 0, 0, &["ok"]);
        let encoded = encode_base64(&bytes);
        assert_eq!(decode_base64(&encoded).expect("decodes"), bytes);
        assert!(decode_base64("!!!!").is_err());
        assert!(decode_base64("QQ===").is_err());
    }

    fn encode_base64(bytes: &[u8]) -> String {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut output = String::new();
        for chunk in bytes.chunks(3) {
            let mut buffer = 0u32;
            for (index, byte) in chunk.iter().enumerate() {
                buffer |= (*byte as u32) << (16 - index * 8);
            }
            let chars = match chunk.len() {
                1 => 2,
                2 => 3,
                _ => 4,
            };
            for index in 0..4 {
                if index < chars {
                    let shift = 18 - index * 6;
                    output.push(TABLE[((buffer >> shift) & 0x3f) as usize] as char);
                } else {
                    output.push('=');
                }
            }
        }
        output
    }
}
