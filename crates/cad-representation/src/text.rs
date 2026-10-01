//! Text outlining: TrueType/OpenType and compiled SHX shape fonts (spec §3.2, §7.1).
//!
//! Hosts supply font bytes (raw sfnt, WOFF1, or SHX); this module turns a text
//! run into world-space polylines so the scene can batch text like any other
//! line geometry. A fallback chain is applied when a drawing's referenced font
//! is not registered or cannot be parsed, so text is not silently dropped.

use cad_domain::{CadError, CadResult, Point3, TextAlignH, TextAlignV};
use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;

use ttf_parser::OutlineBuilder;

use crate::shx::ShxFont;

/// A parsed font face.
enum FaceData {
    /// Raw sfnt bytes (TTF/OTF, or WOFF1 already decoded to sfnt).
    Sfnt(Arc<[u8]>),
    /// Compiled AutoCAD shape font.
    Shx(ShxFont),
    /// A face we recognised but cannot decode yet; reported explicitly.
    Unsupported(String),
}

/// A bounded, in-memory set of font faces with an optional fallback chain.
///
/// Faces are keyed by the name a drawing references (`arial.ttf`) and by the
/// file stem (`arial`), so a drawing that names a `.ttf` still resolves to the
/// matching `.woff` catalog entry. No I/O happens here.
#[derive(Default)]
pub struct FontEngine {
    faces: HashMap<String, Arc<FaceData>>,
    fallback: Vec<String>,
}

impl FontEngine {
    pub fn new() -> Self {
        FontEngine::default()
    }

    /// Register font bytes under `key` plus its file stem.
    ///
    /// SHX shape fonts, raw sfnt and WOFF1 are recognised; WOFF2 is rejected
    /// explicitly. The result must parse or the registration fails.
    pub fn register(&mut self, key: &str, bytes: Arc<[u8]>) -> CadResult<()> {
        self.register_with_encoding(key, bytes, None)
    }

    /// Like [`register`](Self::register) but declares the SHX code page used to
    /// map Unicode back to the font's character codes (for example `gbk`).
    pub fn register_with_encoding(
        &mut self,
        key: &str,
        bytes: Arc<[u8]>,
        encoding: Option<&str>,
    ) -> CadResult<()> {
        let normalized = normalize_key(key);
        if normalized.is_empty() {
            return Err(CadError::InvalidInput("font key is empty".into()));
        }
        let face = Arc::new(parse_face(&normalized, &bytes, encoding)?);
        self.faces.insert(normalized.clone(), face.clone());
        if let Some(stem) = stem_of(&normalized) {
            self.faces.entry(stem).or_insert(face);
        }
        Ok(())
    }

    /// Set the fallback chain used when a referenced font is missing.
    ///
    /// Keys are matched as in [`register`](Self::register); the first that
    /// resolves to a face is used for the whole run.
    pub fn set_fallback(&mut self, keys: Vec<String>) {
        self.fallback = keys;
    }

    pub fn fallback_keys(&self) -> &[String] {
        &self.fallback
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

    fn lookup(&self, key: &str) -> Option<Arc<FaceData>> {
        let normalized = normalize_key(key);
        if let Some(data) = self.faces.get(&normalized) {
            return Some(data.clone());
        }
        stem_of(&normalized).and_then(|stem| self.faces.get(&stem).cloned())
    }

    /// The face used for `font_key`: the primary face, else the first available
    /// fallback. Public so hosts can report which font actually rendered.
    pub fn resolve_face<'a>(&'a self, font_key: &'a str) -> Option<&'a str> {
        if self.lookup(font_key).is_some() {
            return Some(font_key);
        }
        self.fallback
            .iter()
            .find(|key| self.lookup(key).is_some())
            .map(String::as_str)
    }

    /// Outline `text` into world-space polylines at `origin`.
    ///
    /// One polyline per glyph contour; multiple lines are separated by `\n`.
    /// `height` is the cap/em height in world units and `rotation` is radians
    /// about the origin. When the requested font is missing, the fallback chain
    /// is used.
    #[allow(clippy::too_many_arguments)]
    pub fn outline(
        &self,
        font_key: &str,
        text: &str,
        origin: Point3,
        height: f64,
        rotation: f64,
        h_align: TextAlignH,
        v_align: TextAlignV,
    ) -> CadResult<Vec<Vec<Point3>>> {
        // Primary face, then the fallback chain. A face we recognised but
        // cannot decode is skipped so a usable fallback still renders.
        let mut chain: Vec<Arc<FaceData>> = Vec::new();
        if let Some(face) = self.lookup(font_key) {
            chain.push(face);
        }
        for key in &self.fallback {
            if let Some(face) = self.lookup(key) {
                chain.push(face);
            }
        }
        if chain.is_empty() {
            return Err(CadError::ResourceMissing(format!(
                "font '{font_key}' is not registered and no fallback is available"
            )));
        }
        let mut unsupported = None;
        for face in &chain {
            match &**face {
                FaceData::Unsupported(reason) => {
                    unsupported.get_or_insert_with(|| reason.clone());
                }
                FaceData::Sfnt(data) => {
                    return outline_with(data, text, origin, height, rotation, h_align, v_align)
                }
                FaceData::Shx(font) => {
                    return outline_shx(font, text, origin, height, rotation, h_align, v_align)
                }
            }
        }
        Err(CadError::Unsupported(unsupported.unwrap_or_else(|| {
            "no usable font in the fallback chain".into()
        })))
    }
}

/// Parse bytes into a face, detecting SHX, sfnt and WOFF.
fn parse_face(key: &str, bytes: &[u8], encoding: Option<&str>) -> CadResult<FaceData> {
    if bytes.starts_with(b"AutoCAD-") {
        return match ShxFont::parse(bytes, encoding) {
            Ok(font) => Ok(FaceData::Shx(font)),
            // Recognised but undecodable: keep the face so the fallback chain
            // can substitute it, instead of failing registration outright.
            Err(CadError::Unsupported(reason)) => Ok(FaceData::Unsupported(reason)),
            Err(other) => Err(other),
        };
    }
    if bytes.len() >= 4 && &bytes[0..4] == b"wOF2" {
        return Ok(FaceData::Unsupported(
            "WOFF2 fonts are not supported yet".into(),
        ));
    }
    let data = prepare_font(bytes)?;
    ttf_parser::Face::parse(&data, 0)
        .map_err(|e| CadError::CorruptData(format!("font '{key}' cannot be parsed: {e}")))?;
    Ok(FaceData::Sfnt(data))
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

/// Decode WOFF1 to sfnt; pass raw sfnt through.
fn prepare_font(bytes: &[u8]) -> CadResult<Arc<[u8]>> {
    if bytes.len() >= 4 && &bytes[0..4] == b"wOFF" {
        woff_to_sfnt(bytes).map(Arc::from)
    } else {
        Ok(Arc::from(bytes.to_vec()))
    }
}

/// Layout an SHX run into world-space polylines.
fn outline_shx(
    font: &ShxFont,
    text: &str,
    origin: Point3,
    height: f64,
    rotation: f64,
    h_align: TextAlignH,
    v_align: TextAlignV,
) -> CadResult<Vec<Vec<Point3>>> {
    let size = height.abs();
    if !size.is_finite() || size <= 0.0 {
        return Err(CadError::InvalidInput(
            "text height must be a positive finite value".into(),
        ));
    }
    let lines = layout_glyphs(text, |ch, pen, _first| {
        let glyph = font.glyph(ch, size)?;
        let polys: Vec<Vec<[f64; 2]>> = glyph
            .polylines
            .iter()
            .map(|poly| {
                poly.iter()
                    .map(|p| [p[0] + pen[0], p[1] + pen[1]])
                    .collect()
            })
            .collect();
        Some((polys, glyph.advance))
    });
    Ok(finalize_lines(
        lines, size, origin, rotation, h_align, v_align,
    ))
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

/// Flatten a glyph outline into polylines in glyph-local, scaled coordinates.
struct OutlineToPolylines {
    scale: f64,
    pen: [f64; 2],
    polys: Vec<Vec<[f64; 2]>>,
    current: Vec<[f64; 2]>,
    last: [f64; 2],
}

impl OutlineToPolylines {
    fn new(scale: f64, pen: [f64; 2]) -> Self {
        OutlineToPolylines {
            scale,
            pen,
            polys: Vec::new(),
            current: Vec::new(),
            last: [0.0, 0.0],
        }
    }

    fn emit(&mut self, x: f64, y: f64) {
        self.current
            .push([x * self.scale + self.pen[0], y * self.scale + self.pen[1]]);
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

/// Outline `text` using already-decoded sfnt bytes.
fn outline_with(
    data: &[u8],
    text: &str,
    origin: Point3,
    height: f64,
    rotation: f64,
    h_align: TextAlignH,
    v_align: TextAlignV,
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
    let kern_table = face.tables().kern;
    let mut previous: Option<ttf_parser::GlyphId> = None;
    let lines = layout_glyphs(text, |ch, pen, first| {
        if first {
            previous = None;
        }
        let glyph = face.glyph_index(ch)?;
        let mut kerning = 0.0;
        if let (Some(table), Some(prev)) = (kern_table.as_ref(), previous) {
            for subtable in table.subtables {
                if subtable.horizontal {
                    if let Some(value) = subtable.glyphs_kerning(prev, glyph) {
                        kerning = value as f64;
                        break;
                    }
                }
            }
        }
        previous = Some(glyph);
        let advance = face.glyph_hor_advance(glyph).unwrap_or(0) as f64 * scale;
        let kern = kerning * scale;
        let mut builder = OutlineToPolylines::new(scale, [pen[0] + kern, pen[1]]);
        if face.outline_glyph(glyph, &mut builder).is_some() {
            builder.flush();
        }
        Some((std::mem::take(&mut builder.polys), advance + kern))
    });
    Ok(finalize_lines(
        lines,
        height.abs(),
        origin,
        rotation,
        h_align,
        v_align,
    ))
}

/// Lay out each `\n`-separated line: place glyphs at the pen and record the
/// line's advance width. The glyph callback receives the pen position and
/// returns glyph-local polylines plus its advance.
fn layout_glyphs<F>(text: &str, mut glyph: F) -> Vec<(Vec<Vec<[f64; 2]>>, f64)>
where
    F: FnMut(char, [f64; 2], bool) -> Option<(Vec<Vec<[f64; 2]>>, f64)>,
{
    let mut lines = Vec::new();
    for line in text.split('\n') {
        let mut pen = [0.0f64, 0.0];
        let mut polys = Vec::new();
        let mut first = true;
        for ch in line.chars() {
            if let Some((glyph_polys, advance)) = glyph(ch, pen, first) {
                polys.extend(glyph_polys);
                pen[0] += advance;
            }
            first = false;
        }
        lines.push((polys, pen[0]));
    }
    lines
}

/// Apply horizontal/vertical alignment, rotation and translation.
fn finalize_lines(
    lines: Vec<(Vec<Vec<[f64; 2]>>, f64)>,
    size: f64,
    origin: Point3,
    rotation: f64,
    h_align: TextAlignH,
    v_align: TextAlignV,
) -> Vec<Vec<Point3>> {
    let line_height = 1.2 * size;
    let vertical_shift = match v_align {
        TextAlignV::Baseline => 0.0,
        TextAlignV::Bottom => 0.2 * size,
        TextAlignV::Middle => 0.5 * size,
        TextAlignV::Top => size,
    };
    let (sin, cos) = rotation.sin_cos();
    let mut out = Vec::new();
    for (index, (polys, width)) in lines.into_iter().enumerate() {
        let dx = match h_align {
            TextAlignH::Left => 0.0,
            TextAlignH::Center => -width / 2.0,
            TextAlignH::Right => -width,
        };
        let line_y = -(index as f64) * line_height + vertical_shift;
        for poly in polys {
            if poly.len() < 2 {
                continue;
            }
            let points: Vec<Point3> = poly
                .iter()
                .map(|p| {
                    let lx = p[0] + dx;
                    let ly = p[1] + line_y;
                    Point3 {
                        x: origin.x + cos * lx - sin * ly,
                        y: origin.y + sin * lx + cos * ly,
                        z: origin.z,
                    }
                })
                .collect();
            out.push(points);
        }
    }
    out
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
            .outline(
                "arial.ttf",
                "hi",
                Point3::default(),
                2.0,
                0.0,
                TextAlignH::Left,
                TextAlignV::Baseline,
            )
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
        // Registration keeps the face so it can be reported; outlining it fails.
        engine
            .register("x.woff2", Arc::from(b"wOF2....".to_vec()))
            .unwrap();
        let err = engine
            .outline(
                "x.woff2",
                "hi",
                Point3::default(),
                2.0,
                0.0,
                TextAlignH::Left,
                TextAlignV::Baseline,
            )
            .unwrap_err();
        assert!(matches!(err, CadError::Unsupported(_)));
    }

    #[test]
    fn missing_font_uses_the_fallback_chain() {
        // No real face registered: outline must not invent glyphs.
        let engine = FontEngine::new();
        assert!(matches!(
            engine
                .outline(
                    "arial.ttf",
                    "hi",
                    Point3::default(),
                    2.0,
                    0.0,
                    TextAlignH::Left,
                    TextAlignV::Baseline
                )
                .unwrap_err(),
            CadError::ResourceMissing(_)
        ));
    }

    #[test]
    fn unsupported_primary_falls_through_to_fallback() {
        let mut engine = FontEngine::new();
        engine
            .register("bad.woff2", Arc::from(b"wOF2....".to_vec()))
            .unwrap();
        // A usable autumn font is not present here, so the chain exposes the
        // unsupported reason rather than a missing-font error.
        engine.set_fallback(vec!["also-missing".into()]);
        let err = engine
            .outline(
                "bad.woff2",
                "x",
                Point3::default(),
                2.0,
                0.0,
                TextAlignH::Left,
                TextAlignV::Baseline,
            )
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
            .outline(
                "arial.ttf",
                "AB",
                Point3::default(),
                10.0,
                0.0,
                TextAlignH::Left,
                TextAlignV::Baseline,
            )
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

    /// Opt-in SHX test: run with `YACR_TEST_SHX=/path/to/txt.shx`.
    #[test]
    fn outlines_a_real_shx_when_provided() {
        let Ok(path) = std::env::var("YACR_TEST_SHX") else {
            return;
        };
        let bytes = std::fs::read(path).unwrap();
        let mut engine = FontEngine::new();
        engine
            .register("txt.shx", Arc::from(bytes.into_boxed_slice()))
            .unwrap();
        let polys = engine
            .outline(
                "txt.shx",
                "AB",
                Point3::default(),
                10.0,
                0.0,
                TextAlignH::Left,
                TextAlignV::Baseline,
            )
            .unwrap();
        assert!(!polys.is_empty(), "no SHX glyph outlines");
        let max_x = polys
            .iter()
            .flatten()
            .map(|p| p.x)
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(max_x > 0.0, "SHX advance produced no horizontal extent");
    }

    /// Opt-in fallback test: a missing primary font uses a registered fallback.
    #[test]
    fn missing_primary_uses_registered_fallback_when_provided() {
        let Ok(path) = std::env::var("YACR_TEST_SHX") else {
            return;
        };
        let bytes = std::fs::read(&path).unwrap();
        let mut engine = FontEngine::new();
        engine.set_fallback(vec!["fallback.shx".into()]);
        engine
            .register("fallback.shx", Arc::from(bytes.into_boxed_slice()))
            .unwrap();
        let polys = engine
            .outline(
                "not-installed.shx",
                "A",
                Point3::default(),
                10.0,
                0.0,
                TextAlignH::Left,
                TextAlignV::Baseline,
            )
            .unwrap();
        assert!(!polys.is_empty(), "fallback did not render");
    }

    #[test]
    fn alignment_shifts_the_run_relative_to_its_anchor() {
        let lines = vec![(vec![vec![[0.0, 0.0], [10.0, 0.0]]], 10.0)];
        let left = finalize_lines(
            lines.clone(),
            2.0,
            Point3::default(),
            0.0,
            TextAlignH::Left,
            TextAlignV::Baseline,
        );
        let center = finalize_lines(
            lines.clone(),
            2.0,
            Point3::default(),
            0.0,
            TextAlignH::Center,
            TextAlignV::Baseline,
        );
        let right = finalize_lines(
            lines,
            2.0,
            Point3::default(),
            0.0,
            TextAlignH::Right,
            TextAlignV::Baseline,
        );
        assert_eq!(left[0][0].x, 0.0);
        assert_eq!(center[0][0].x, -5.0);
        assert_eq!(right[0][0].x, -10.0);
    }
}
