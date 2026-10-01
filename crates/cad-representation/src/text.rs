//! TrueType/OpenType text outlining (spec v2.0 §3.2, §7.1).
//!
//! Hosts supply font bytes (raw sfnt, or WOFF1 which is decoded here); this
//! module turns a text run into world-space polylines so the scene can batch
//! text like any other line geometry. SHX shape fonts are not decoded yet and
//! are reported as unsupported rather than faked.

use cad_domain::{CadError, CadResult, Point3};
use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;

use ttf_parser::OutlineBuilder;

/// A bounded, in-memory set of font faces.
///
/// Faces are keyed by the name a drawing references (`arial.ttf`) and by the
/// file stem (`arial`), so a drawing that names a `.ttf` still resolves to the
/// matching `.woff` catalog entry. No I/O happens here.
#[derive(Default)]
pub struct FontEngine {
    faces: HashMap<String, Arc<[u8]>>,
}

impl FontEngine {
    pub fn new() -> Self {
        FontEngine::default()
    }

    /// Register font bytes under `key` plus its file stem.
    ///
    /// WOFF1 is decoded to sfnt; WOFF2 is rejected explicitly. The result must
    /// parse as a font face or the registration fails.
    pub fn register(&mut self, key: &str, bytes: Arc<[u8]>) -> CadResult<()> {
        let data = prepare_font(&bytes)?;
        ttf_parser::Face::parse(&data, 0)
            .map_err(|e| CadError::CorruptData(format!("font '{key}' cannot be parsed: {e}")))?;
        let normalized = normalize_key(key);
        if normalized.is_empty() {
            return Err(CadError::InvalidInput("font key is empty".into()));
        }
        self.faces.insert(normalized.clone(), data.clone());
        if let Some(stem) = stem_of(&normalized) {
            self.faces.entry(stem).or_insert(data);
        }
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.faces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }

    /// Whether a face for `key` (or its stem) is available.
    pub fn contains(&self, key: &str) -> bool {
        self.lookup(key).is_some()
    }

    fn lookup(&self, key: &str) -> Option<Arc<[u8]>> {
        let normalized = normalize_key(key);
        if let Some(data) = self.faces.get(&normalized) {
            return Some(data.clone());
        }
        stem_of(&normalized).and_then(|stem| self.faces.get(&stem).cloned())
    }

    /// Outline `text` into world-space polylines at `origin`.
    ///
    /// One polyline per glyph contour; multiple lines are separated by `\n`.
    /// `height` is the cap/em height in world units and `rotation` is radians
    /// about the origin.
    pub fn outline(
        &self,
        font_key: &str,
        text: &str,
        origin: Point3,
        height: f64,
        rotation: f64,
    ) -> CadResult<Vec<Vec<Point3>>> {
        let data = self.lookup(font_key).ok_or_else(|| {
            CadError::ResourceMissing(format!("font '{font_key}' is not registered"))
        })?;
        outline_with(&data, text, origin, height, rotation)
    }
}

fn normalize_key(raw: &str) -> String {
    let bare = raw.trim().trim_matches('"');
    let bare = bare.rsplit(['/', '\\']).next().unwrap_or(bare);
    bare.to_ascii_lowercase()
}

fn stem_of(name: &str) -> Option<String> {
    name.rsplit_once('.')
        .map(|(stem, _)| stem.to_string())
        .filter(|stem| !stem.is_empty())
}

/// Decode WOFF1 to sfnt; pass raw sfnt through; reject WOFF2.
fn prepare_font(bytes: &[u8]) -> CadResult<Arc<[u8]>> {
    if bytes.len() >= 4 && &bytes[0..4] == b"wOFF" {
        woff_to_sfnt(bytes).map(Arc::from)
    } else if bytes.len() >= 4 && &bytes[0..4] == b"wOF2" {
        Err(CadError::Unsupported(
            "WOFF2 fonts are not supported yet".into(),
        ))
    } else {
        Ok(Arc::from(bytes.to_vec()))
    }
}

/// Reconstruct an sfnt (TTF) container from a WOFF1 file.
fn woff_to_sfnt(data: &[u8]) -> CadResult<Vec<u8>> {
    if data.len() < 44 {
        return Err(CadError::CorruptData("WOFF file is truncated".into()));
    }
    let u32be = |o: usize| u32::from_be_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
    let u16be = |o: usize| u16::from_be_bytes([data[o], data[o + 1]]);

    let flavor = u32be(4);
    let num_tables = u16be(12) as usize;
    let dir_start = 44;
    if num_tables == 0 || dir_start + num_tables * 20 > data.len() {
        return Err(CadError::CorruptData(
            "WOFF table directory is invalid".into(),
        ));
    }

    struct Entry {
        tag: [u8; 4],
        offset: usize,
        comp: usize,
        orig: usize,
        checksum: u32,
    }
    let mut entries = Vec::with_capacity(num_tables);
    for index in 0..num_tables {
        let o = dir_start + index * 20;
        entries.push(Entry {
            tag: [data[o], data[o + 1], data[o + 2], data[o + 3]],
            offset: u32be(o + 4) as usize,
            comp: u32be(o + 8) as usize,
            orig: u32be(o + 12) as usize,
            checksum: u32be(o + 16),
        });
    }
    // sfnt requires the directory sorted by tag.
    entries.sort_by_key(|entry| entry.tag);

    let mut max_power = 0usize;
    while (1usize << (max_power + 1)) <= num_tables {
        max_power += 1;
    }
    let search_range = (1usize << max_power) * 16;
    let entry_selector = max_power;
    let range_shift = num_tables * 16 - search_range;

    let mut header = Vec::with_capacity(12);
    header.extend_from_slice(&flavor.to_be_bytes());
    header.extend_from_slice(&(num_tables as u16).to_be_bytes());
    header.extend_from_slice(&(search_range as u16).to_be_bytes());
    header.extend_from_slice(&(entry_selector as u16).to_be_bytes());
    header.extend_from_slice(&(range_shift as u16).to_be_bytes());

    let mut records: Vec<([u8; 4], u32, u32, u32)> = Vec::with_capacity(num_tables);
    let mut body: Vec<u8> = Vec::new();
    let mut next_offset = 12 + num_tables * 16;
    for entry in &entries {
        if entry.offset + entry.comp > data.len() {
            return Err(CadError::CorruptData("WOFF table extends past EOF".into()));
        }
        let raw = &data[entry.offset..entry.offset + entry.comp];
        let table: Vec<u8> = if entry.comp == entry.orig {
            raw.to_vec()
        } else {
            let mut decoder = flate2::read::ZlibDecoder::new(raw);
            let mut decoded = Vec::with_capacity(entry.orig);
            decoder
                .read_to_end(&mut decoded)
                .map_err(|e| CadError::CorruptData(format!("WOFF table decompress failed: {e}")))?;
            if decoded.len() != entry.orig {
                return Err(CadError::CorruptData(
                    "WOFF table decompressed to the wrong size".into(),
                ));
            }
            decoded
        };
        while next_offset % 4 != 0 {
            next_offset += 1;
            body.push(0);
        }
        records.push((
            entry.tag,
            entry.checksum,
            next_offset as u32,
            entry.orig as u32,
        ));
        body.extend_from_slice(&table);
        next_offset += table.len();
    }

    let mut out = header;
    for (tag, checksum, offset, length) in records {
        out.extend_from_slice(&tag);
        out.extend_from_slice(&checksum.to_be_bytes());
        out.extend_from_slice(&offset.to_be_bytes());
        out.extend_from_slice(&length.to_be_bytes());
    }
    out.extend_from_slice(&body);
    Ok(out)
}

/// Flatten a glyph outline into polylines with a fixed subdivision per curve.
struct OutlineToPolylines {
    scale: f64,
    pen: [f64; 2],
    origin: Point3,
    cos: f64,
    sin: f64,
    polys: Vec<Vec<Point3>>,
    current: Vec<Point3>,
    last: [f64; 2],
}

impl OutlineToPolylines {
    fn new(scale: f64, pen: [f64; 2], origin: Point3, rotation: f64) -> Self {
        OutlineToPolylines {
            scale,
            pen,
            origin,
            cos: rotation.cos(),
            sin: rotation.sin(),
            polys: Vec::new(),
            current: Vec::new(),
            last: [0.0, 0.0],
        }
    }

    fn emit(&mut self, x: f64, y: f64) {
        let lx = x * self.scale + self.pen[0];
        let ly = y * self.scale + self.pen[1];
        self.current.push(Point3 {
            x: self.origin.x + self.cos * lx - self.sin * ly,
            y: self.origin.y + self.sin * lx + self.cos * ly,
            z: self.origin.z,
        });
        self.last = [x, y];
    }

    fn flush(&mut self) {
        if self.current.len() >= 2 {
            self.polys.push(std::mem::take(&mut self.current));
        } else {
            self.current.clear();
        }
    }

    fn quad(&mut self, c: (f64, f64), end: (f64, f64)) {
        let start = self.last;
        const STEPS: usize = 6;
        for step in 1..=STEPS {
            let t = step as f64 / STEPS as f64;
            let mt = 1.0 - t;
            let x = mt * mt * start[0] + 2.0 * mt * t * c.0 + t * t * end.0;
            let y = mt * mt * start[1] + 2.0 * mt * t * c.1 + t * t * end.1;
            self.emit(x, y);
        }
    }

    fn cubic(&mut self, c1: (f64, f64), c2: (f64, f64), end: (f64, f64)) {
        let start = self.last;
        const STEPS: usize = 10;
        for step in 1..=STEPS {
            let t = step as f64 / STEPS as f64;
            let mt = 1.0 - t;
            let a = mt * mt * mt;
            let b = 3.0 * mt * mt * t;
            let c = 3.0 * mt * t * t;
            let d = t * t * t;
            let x = a * start[0] + b * c1.0 + c * c2.0 + d * end.0;
            let y = a * start[1] + b * c1.1 + c * c2.1 + d * end.1;
            self.emit(x, y);
        }
    }
}

impl OutlineBuilder for OutlineToPolylines {
    fn move_to(&mut self, x: f32, y: f32) {
        self.flush();
        self.emit(x as f64, y as f64);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.emit(x as f64, y as f64);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.quad((x1 as f64, y1 as f64), (x as f64, y as f64));
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.cubic(
            (x1 as f64, y1 as f64),
            (x2 as f64, y2 as f64),
            (x as f64, y as f64),
        );
    }
    fn close(&mut self) {
        if let Some(first) = self.current.first().copied() {
            if self.current.last().copied() != Some(first) {
                self.current.push(first);
            }
        }
        self.flush();
    }
}

/// Outline `text` using already-decoded font bytes.
fn outline_with(
    data: &[u8],
    text: &str,
    origin: Point3,
    height: f64,
    rotation: f64,
) -> CadResult<Vec<Vec<Point3>>> {
    let face = ttf_parser::Face::parse(data, 0)
        .map_err(|e| CadError::CorruptData(format!("font cannot be parsed: {e}")))?;
    let upem = face.units_per_em().max(1) as f64;
    let scale = height.abs() / upem;
    if !scale.is_finite() || scale <= 0.0 {
        return Err(CadError::InvalidInput(
            "text height must be a positive finite value".into(),
        ));
    }
    let line_step = -1.2 * height.abs();

    let mut polys = Vec::new();
    let mut pen = [0.0f64, 0.0];
    for ch in text.chars() {
        if ch == '\n' {
            pen = [0.0, pen[1] + line_step];
            continue;
        }
        let Some(glyph) = face.glyph_index(ch) else {
            continue;
        };
        let advance = face.glyph_hor_advance(glyph).unwrap_or(0) as f64 * scale;
        let mut builder = OutlineToPolylines::new(scale, pen, origin, rotation);
        if face.outline_glyph(glyph, &mut builder).is_some() {
            builder.flush();
            polys.extend(builder.polys);
        }
        pen[0] += advance;
    }
    Ok(polys)
}

/// Strip MTEXT/TEXT formatting so the raw glyph text can be shaped.
///
/// Handles `\P` line breaks, `%%d`/`%%p`/`%%c` symbols, brace grouping and
/// simple `\X...;` codes. Unknown escapes are dropped; this is an
/// approximation, not a full MTEXT layout engine.
pub fn sanitize_text(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' {
            if i + 1 >= chars.len() {
                i += 1;
                continue;
            }
            let n = chars[i + 1];
            match n {
                'P' | 'p' => {
                    out.push('\n');
                    i += 2;
                }
                '~' => {
                    out.push(' ');
                    i += 2;
                }
                '\\' => {
                    out.push('\\');
                    i += 2;
                }
                _ if n.is_ascii_alphabetic() => {
                    let mut j = i + 2;
                    while j < chars.len() && chars[j] != ';' {
                        j += 1;
                    }
                    i = if j < chars.len() { j + 1 } else { chars.len() };
                }
                _ => i += 2,
            }
            continue;
        }
        if c == '{' || c == '}' {
            i += 1;
            continue;
        }
        if c == '%' && i + 2 < chars.len() && chars[i + 1] == '%' {
            match chars[i + 2].to_ascii_lowercase() {
                'd' => {
                    out.push('°');
                    i += 3;
                    continue;
                }
                'p' => {
                    out.push('±');
                    i += 3;
                    continue;
                }
                'c' => {
                    out.push('⌀');
                    i += 3;
                    continue;
                }
                _ => {}
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_handles_mtext_and_text_codes() {
        assert_eq!(sanitize_text("a\\Pb"), "a\nb");
        assert_eq!(sanitize_text("{\\fArial|b0;Hi}"), "Hi");
        assert_eq!(sanitize_text("45%%d"), "45°");
        assert_eq!(sanitize_text("plain"), "plain");
        assert_eq!(sanitize_text("a\\~b"), "a b");
    }

    #[test]
    fn unregistered_font_is_reported_not_faked() {
        let engine = FontEngine::new();
        let err = engine
            .outline("arial.ttf", "hi", Point3::default(), 2.0, 0.0)
            .unwrap_err();
        assert!(matches!(err, CadError::ResourceMissing(_)));
        assert!(engine.is_empty());
    }

    #[test]
    fn garbage_font_bytes_are_rejected() {
        let mut engine = FontEngine::new();
        assert!(engine
            .register("bad.ttf", Arc::from(vec![0u8; 128]))
            .is_err());
        assert!(engine.is_empty());
    }

    #[test]
    fn woff2_is_explicitly_unsupported() {
        let mut engine = FontEngine::new();
        let err = engine
            .register("x.woff2", Arc::from(b"wOF2....".to_vec()))
            .unwrap_err();
        assert!(matches!(err, CadError::Unsupported(_)));
    }

    /// Opt-in glyph test: run with `YACR_TEST_FONT=/path/to/font.woff`.
    ///
    /// Proves WOFF decoding, key stemming and outline generation against a real
    /// face without committing font binaries (see `docs/fonts.md`).
    #[test]
    fn outlines_a_real_font_when_provided() {
        let Ok(path) = std::env::var("YACR_TEST_FONT") else {
            return;
        };
        let bytes = std::fs::read(path).unwrap();
        let mut engine = FontEngine::new();
        // Registered as `arial.woff`; the run asks for `arial.ttf`, which must
        // resolve through the shared file stem.
        engine
            .register("arial.woff", Arc::from(bytes.into_boxed_slice()))
            .unwrap();
        let polys = engine
            .outline("arial.ttf", "AB", Point3::default(), 10.0, 0.0)
            .unwrap();
        assert!(!polys.is_empty(), "no glyph outlines produced");
        let mut min_x = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        for poly in &polys {
            assert!(poly.len() >= 2);
            for p in poly {
                assert!(p.x.is_finite() && p.y.is_finite());
                min_x = min_x.min(p.x);
                max_x = max_x.max(p.x);
            }
        }
        // Two glyphs at height 10 stay within a modest horizontal span.
        assert!(min_x >= -1.0 && max_x <= 30.0, "span {min_x}..{max_x}");
    }
}
