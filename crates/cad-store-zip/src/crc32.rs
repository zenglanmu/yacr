//! CRC-32 (IEEE 802.3), the checksum a zip entry carries.
//!
//! The manifest records each entry's zip CRC. Every encoded entry is zip `Stored`,
//! so for those the CRC covers exactly the bytes the entry stores (the zstd frame,
//! or a `raw` payload's verbatim bytes); a copied foreign entry keeps the CRC its
//! writer recorded, which for a `Deflated` entry covers the uncompressed bytes. The
//! reader cross-checks the recorded value against the zip central directory. The zip
//! writer computes its own copy internally but does not expose it, and this crate
//! deliberately has no second checksum dependency (`crc32fast` is only reached
//! through `zip`), so the polynomial is implemented here: 256-entry table, built at
//! compile time, ~1 GB/s — not the bottleneck of a save.
//!
//! Reflected polynomial `0xEDB88320`, initial value `0xFFFF_FFFF`, final NOT —
//! i.e. exactly what `zip`/`crc32fast` produce, which the round-trip test in
//! `tests/archive.rs` proves by comparing with the archive's own central
//! directory.

/// The 256-entry lookup table for the reflected polynomial.
const fn table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut index = 0usize;
    while index < 256 {
        let mut value = index as u32;
        let mut bit = 0;
        while bit < 8 {
            value = if value & 1 == 1 {
                0xEDB8_8320 ^ (value >> 1)
            } else {
                value >> 1
            };
            bit += 1;
        }
        table[index] = value;
        index += 1;
    }
    table
}

static TABLE: [u32; 256] = table();

/// CRC-32 of `bytes`.
pub(crate) fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = Crc32::new();
    crc.update(bytes);
    crc.finish()
}

/// A streaming CRC-32, for entry bodies that are measured while they are
/// written rather than collected first (ADR-0040 D15).
pub(crate) struct Crc32(u32);

impl Crc32 {
    pub(crate) fn new() -> Crc32 {
        Crc32(0xFFFF_FFFF)
    }

    pub(crate) fn update(&mut self, bytes: &[u8]) {
        let mut crc = self.0;
        for byte in bytes {
            crc = TABLE[((crc ^ u32::from(*byte)) & 0xff) as usize] ^ (crc >> 8);
        }
        self.0 = crc;
    }

    pub(crate) fn finish(self) -> u32 {
        !self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_published_vectors() {
        // CRC-32/ISO-HDLC ("123456789" -> 0xCBF43926).
        assert_eq!(crc32(b""), 0x0000_0000);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b"a"), 0xE8B7_BE43);
        assert_eq!(
            crc32(b"The quick brown fox jumps over the lazy dog"),
            0x414F_A339
        );
    }
}
