//! Proxy metafile framing.
//!
//! Layout (matching acadrust 0.6.3's parser):
//!
//! ```text
//! u32 total_size
//! u32 record_count
//! repeat record_count:
//!   u32 record_size      (includes the 8-byte record header)
//!   u32 record_type
//!   [u8; record_size - 8] payload
//! ```
//!
//! All reads are bounds-checked and integer overflow is avoided with checked
//! arithmetic, so malformed input yields an error instead of panicking.

use cad_domain::{CadError, CadResult};

use crate::DecodeLimits;

const HEADER_SIZE: usize = 8;
const RECORD_HEADER_SIZE: usize = 8;

/// A record as borrowed from the input buffer.
#[derive(Debug, Clone, Copy)]
pub struct RawRecord<'a> {
    pub record_type: u32,
    pub data: &'a [u8],
}

fn read_u32(data: &[u8], offset: usize) -> CadResult<u32> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| CadError::CorruptData("offset overflow".into()))?;
    let bytes = data
        .get(offset..end)
        .ok_or_else(|| CadError::CorruptData("truncated proxy header".into()))?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// Decode the record list, enforcing `limits`.
pub fn decode(data: &[u8], limits: DecodeLimits) -> CadResult<Vec<RawRecord<'_>>> {
    if data.len() < HEADER_SIZE {
        return Err(CadError::CorruptData(
            "proxy metafile is smaller than its header".into(),
        ));
    }
    let total_size = read_u32(data, 0)? as usize;
    let record_count = read_u32(data, 4)? as usize;
    if total_size < HEADER_SIZE || total_size > data.len() {
        return Err(CadError::CorruptData(format!(
            "proxy metafile declares {total_size} bytes but only {} are present",
            data.len()
        )));
    }
    if record_count > limits.max_records {
        return Err(CadError::Unsupported(format!(
            "proxy metafile declares {record_count} records, over the {} limit",
            limits.max_records
        )));
    }
    let mut records = Vec::with_capacity(record_count.min(4096));
    let mut offset = HEADER_SIZE;
    for _ in 0..record_count {
        let record_size = read_u32(data, offset)? as usize;
        let record_type = read_u32(data, offset + 4)?;
        if record_size < RECORD_HEADER_SIZE {
            return Err(CadError::CorruptData(
                "proxy record is smaller than its header".into(),
            ));
        }
        let payload_size = record_size - RECORD_HEADER_SIZE;
        if payload_size > limits.max_bytes {
            return Err(CadError::Unsupported(format!(
                "proxy record payload of {payload_size} bytes exceeds the {} byte limit",
                limits.max_bytes
            )));
        }
        let record_end = offset
            .checked_add(record_size)
            .ok_or_else(|| CadError::CorruptData("proxy record size overflow".into()))?;
        if record_end > total_size {
            return Err(CadError::CorruptData(
                "proxy record overruns the metafile".into(),
            ));
        }
        let payload = &data[offset + RECORD_HEADER_SIZE..record_end];
        records.push(RawRecord {
            record_type,
            data: payload,
        });
        offset = record_end;
    }
    if offset != total_size {
        return Err(CadError::CorruptData(
            "proxy record list does not fill the metafile".into(),
        ));
    }
    Ok(records)
}

/// Encode a single record for tests (and for callers building synthetic data).
pub fn encode_for_test(record_type: u32, payload: &[u8]) -> Vec<u8> {
    let size = (RECORD_HEADER_SIZE + payload.len()) as u32;
    let mut out = Vec::with_capacity(size as usize);
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&record_type.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

/// Concatenate records into a complete metafile.
pub fn concat(records: &[Vec<u8>]) -> Vec<u8> {
    let body_len: usize = records.iter().map(|r| r.len()).sum();
    let total = HEADER_SIZE + body_len;
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(records.len() as u32).to_le_bytes());
    for r in records {
        out.extend_from_slice(r);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_metafile() {
        let data = concat(&[encode_for_test(21, &[]), encode_for_test(36, &[1u8, 2, 3])]);
        let recs = decode(&data, DecodeLimits::default()).unwrap();
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].record_type, 21);
        assert_eq!(recs[1].data, &[1, 2, 3]);
    }

    #[test]
    fn rejects_lying_total_size() {
        let mut data = concat(&[encode_for_test(21, &[])]);
        data[0..4].copy_from_slice(&1000u32.to_le_bytes());
        assert!(decode(&data, DecodeLimits::default()).is_err());
    }

    #[test]
    fn rejects_truncated_input() {
        let data = concat(&[encode_for_test(36, &[0u8; 8])]);
        assert!(decode(&data[..data.len() - 4], DecodeLimits::default()).is_err());
    }

    #[test]
    fn rejects_record_count_over_limit() {
        let data = concat(&[encode_for_test(21, &[])]);
        let limits = DecodeLimits {
            max_records: 0,
            ..Default::default()
        };
        assert!(decode(&data, limits).is_err());
    }
}
