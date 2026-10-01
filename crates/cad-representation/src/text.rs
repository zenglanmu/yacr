//! Text outlining: TrueType/OpenType and compiled SHX shape fonts (spec §3.2, §7.1).
//!
//! Hosts supply font bytes (raw sfnt, WOFF1, or SHX); this module turns a text
//! run into world-space polylines so the scene can batch text like any other
//! line geometry. A fallback chain is applied when a drawing's referenced font
//! is not registered or cannot be parsed, so text is not silently dropped.
//!
//! ## Known limitations (explicit, not silently approximated)
//!
//! This is a glyph-per-glyph outline renderer, not a text shaping engine. The
//! following remain unsupported:
//!
//! - **Bidirectional text and complex-script reordering** are not performed; the
//!   logical order is rendered as-is.
//! - **Contextual shaping** (Arabic joining, Indic reordering, ligature
//!   substitution) is not applied; the OpenType `kern` table is the only layout
//!   feature consulted.
//! - **MTEXT columns** are not laid out; column/justification codes are consumed
//!   by [`sanitize_text`] but do not affect the result.
//! - **Exact line spacing / vertical metrics** are not read from the font; line
//!   spacing is the fixed `1.2 * height` used by [`finalize_lines`].
//! - **MTEXT stacked fractions** (`\S`) are rendered as two stacked lines with
//!   that normal line spacing and no fraction bar or vertical scaling.
//! - **Tabs** are expanded to a fixed run of spaces (see `TAB_STEP`); true
//!   tab-stop alignment against proportional advances is not implemented.
//! - **Per-glyph fallback only**: a missing glyph is substituted from the next
//!   face in the chain. There is no per-style (bold/italic) or per-script fallback.
//!
//! See `docs/fonts.md` §未完成 for the tracked gaps.

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

/// A face from the resolved fallback chain that can actually supply glyphs.
///
/// `Sfnt` faces are parsed once per [`FontEngine::outline`] call and borrow the
/// caller's kept-alive `Arc<[u8]>`; `Shx` faces are borrowed from the chain.
enum ResolvedFace<'a> {
    Sfnt {
        // Boxed: `ttf_parser::Face` is several KiB and would dominate this enum.
        face: Box<ttf_parser::Face<'a>>,
        kern: Option<ttf_parser::kern::Table<'a>>,
        /// World units per font unit: `height / units_per_em`.
        scale: f64,
    },
    Shx(&'a ShxFont),
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
    ///
    /// Fallback is resolved **per glyph**: each character is drawn with the
    /// first face in the chain (primary, then fallbacks in order) that actually
    /// contains it, and the pen advances by that face's advance. When the
    /// primary face has every glyph, the result is byte-for-byte the same as
    /// single-face rendering.
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
        let size = height.abs();
        if !size.is_finite() || size <= 0.0 {
            return Err(CadError::InvalidInput(
                "text height must be a positive finite value".into(),
            ));
        }

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

        // Keep every sfnt `Arc<[u8]>` alive for the whole call so the parsed
        // faces (which borrow the bytes) stay valid. Each Sfnt candidate is
        // parsed exactly once, regardless of how many glyphs come from it.
        let keepalive: Vec<Arc<[u8]>> = chain
            .iter()
            .filter_map(|face| match &**face {
                FaceData::Sfnt(data) => Some(data.clone()),
                _ => None,
            })
            .collect();

        let mut unsupported = None;
        let mut resolved: Vec<ResolvedFace<'_>> = Vec::new();
        let mut sfnt_index = 0usize;
        for face in &chain {
            match &**face {
                FaceData::Unsupported(reason) => {
                    unsupported.get_or_insert_with(|| reason.clone());
                }
                FaceData::Sfnt(_) => {
                    let data: &[u8] = &keepalive[sfnt_index];
                    sfnt_index += 1;
                    // Registration already proved parseability; skip defensively
                    // so a bad fallback does not abort a usable chain.
                    if let Ok(parsed) = ttf_parser::Face::parse(data, 0) {
                        let upem = parsed.units_per_em().max(1) as f64;
                        let scale = size / upem;
                        if scale.is_finite() && scale > 0.0 {
                            let kern = parsed.tables().kern;
                            resolved.push(ResolvedFace::Sfnt {
                                face: Box::new(parsed),
                                kern,
                                scale,
                            });
                        }
                    }
                }
                FaceData::Shx(font) => resolved.push(ResolvedFace::Shx(font)),
            }
        }
        if resolved.is_empty() {
            return Err(CadError::Unsupported(
                unsupported.unwrap_or_else(|| "no usable font in the fallback chain".into()),
            ));
        }

        // `previous` remembers which face supplied the preceding glyph so that
        // kerning is only applied within a single face (cross-face pairs are
        // meaningless).
        let mut previous: Option<(usize, ttf_parser::GlyphId)> = None;
        let lines = layout_glyphs(text, |ch, pen, first| {
            if first {
                previous = None;
            }
            for (index, candidate) in resolved.iter().enumerate() {
                match candidate {
                    ResolvedFace::Sfnt { face, kern, scale } => {
                        let Some(glyph) = face.glyph_index(ch) else {
                            continue;
                        };
                        let mut kerning = 0.0;
                        if let (Some(table), Some((prev_index, prev))) = (kern.as_ref(), previous) {
                            if prev_index == index {
                                for subtable in table.subtables {
                                    if subtable.horizontal {
                                        if let Some(value) = subtable.glyphs_kerning(prev, glyph) {
                                            kerning = value as f64;
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        previous = Some((index, glyph));
                        let advance = face.glyph_hor_advance(glyph).unwrap_or(0) as f64 * scale;
                        let shift = kerning * scale;
                        let mut builder = OutlineToPolylines::new(*scale, [pen[0] + shift, pen[1]]);
                        if face.outline_glyph(glyph, &mut builder).is_some() {
                            builder.flush();
                        }
                        return Some((std::mem::take(&mut builder.polys), advance + shift));
                    }
                    ResolvedFace::Shx(font) => {
                        if let Some(glyph) = font.glyph(ch, size) {
                            previous = None;
                            let polys: Vec<Vec<[f64; 2]>> = glyph
                                .polylines
                                .iter()
                                .map(|poly| {
                                    poly.iter()
                                        .map(|p| [p[0] + pen[0], p[1] + pen[1]])
                                        .collect()
                                })
                                .collect();
                            return Some((polys, glyph.advance));
                        }
                    }
                }
            }
            None
        });
        Ok(finalize_lines(
            lines, size, origin, rotation, h_align, v_align,
        ))
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

/// Spaces a tab expands to.
///
/// Tabs are rendered as a fixed run of spaces rather than snapping the pen to
/// proportional tab stops, so `\t`/`^I` spacing is an explicit approximation
/// (see the module-level limitations).
const TAB_STEP: usize = 4;

/// Consume a `\X...;` parameter code whose value begins at `start` (the index
/// just after the code letter), returning the index just past the terminating
/// `;` — or the end of input when no `;` is present.
fn skip_param_code(chars: &[char], start: usize) -> usize {
    let mut j = start;
    while j < chars.len() && chars[j] != ';' {
        j += 1;
    }
    if j < chars.len() {
        j + 1
    } else {
        j
    }
}

/// Strip MTEXT/TEXT formatting so the raw glyph text can be shaped.
///
/// Handles:
///
/// - `\P`/`\p` line breaks, `\~` non-breaking space and `\\` escapes;
/// - `%%d`/`%%p`/`%%c` symbols and brace grouping;
/// - `\S<num><sep><den>;` stacked fractions, rendered as two lines (see below);
/// - `\t` and `^I` tabs, expanded to `TAB_STEP` spaces;
/// - no-argument toggles `\L`/`\l` (underline), `\O`/`\o` (overline) and
///   `\K`/`\k` (strikethrough), which affect styling we do not render;
/// - parameter codes `\H`, `\W`, `\A`, `\Q`, `\C`, `\f`, `\F`, … terminated by
///   `;`, consumed without emitting text.
///
/// Unknown escapes are dropped. This is an approximation, not a full MTEXT
/// layout engine: stacked fractions use the normal line spacing with no
/// fraction bar or vertical scaling, tabs do not align to true tab stops, and
/// MTEXT columns are ignored.
pub fn sanitize_text(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // DXF caret notation: `^I` is a TAB control character.
        if c == '^' && i + 1 < chars.len() && matches!(chars[i + 1], 'I' | 'i') {
            for _ in 0..TAB_STEP {
                out.push(' ');
            }
            i += 2;
            continue;
        }
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
                't' => {
                    // `\t` is a tab; expand to a fixed run of spaces.
                    for _ in 0..TAB_STEP {
                        out.push(' ');
                    }
                    i += 2;
                }
                'S' => {
                    i = push_stacked_fraction(&chars, i, &mut out);
                }
                // No-argument style toggles. These must not scan for `;`, or
                // they would swallow the rest of the run.
                'L' | 'l' | 'O' | 'o' | 'K' | 'k' => {
                    i += 2;
                }
                // Parameter codes: `\H<val>;`, `\W<val>;`, `\A<val>;`,
                // `\Q<val>;`, `\C<val>;`, `\f<name|…>;`, etc. Consume the value
                // and its terminating `;` without emitting text.
                'H' | 'W' | 'A' | 'Q' | 'C' | 'f' | 'F' => {
                    i = skip_param_code(&chars, i + 2);
                }
                _ if n.is_ascii_alphabetic() => {
                    i = skip_param_code(&chars, i + 2);
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

/// Render an MTEXT stacked fraction starting at the `\` of `\S`.
///
/// The separator is the first of `^` (centre), `/` (horizontal bar) or `#`
/// (diagonal). All three are rendered the same way here: numerator and
/// denominator on consecutive lines, in the order written, using the normal
/// line spacing. There is no fraction bar, no vertical scaling and no reduced
/// font size — a deliberate, documented approximation. Returns the index just
/// past the consumed `\S…;` code.
fn push_stacked_fraction(chars: &[char], backslash: usize, out: &mut String) -> usize {
    let end = skip_param_code(chars, backslash + 2);
    let content_end = if end > backslash + 2 && chars.get(end - 1) == Some(&';') {
        end - 1
    } else {
        end
    };
    let content: String = chars[backslash + 2..content_end].iter().collect();
    match content
        .char_indices()
        .find(|(_, ch)| matches!(ch, '^' | '/' | '#'))
    {
        Some((idx, sep)) => {
            out.push_str(&sanitize_text(&content[..idx]));
            out.push('\n');
            out.push_str(&sanitize_text(&content[idx + sep.len_utf8()..]));
        }
        None => out.push_str(&sanitize_text(&content)),
    }
    end
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
    fn sanitize_stacks_fractions_on_two_lines() {
        // `^`, `/` and `#` all render as numerator over denominator.
        assert_eq!(sanitize_text("\\S1/2;"), "1\n2");
        assert_eq!(sanitize_text("x\\Sa^b;y"), "xa\nby");
        assert_eq!(sanitize_text("\\S10#3;"), "10\n3");
        // No separator: the payload is emitted verbatim (sans the code).
        assert_eq!(sanitize_text("\\Sabc;"), "abc");
        // Missing terminator is tolerated.
        assert_eq!(sanitize_text("\\S1/2"), "1\n2");
    }

    #[test]
    fn sanitize_expands_tabs() {
        let four = " ".repeat(TAB_STEP);
        assert_eq!(sanitize_text("a\\tb"), format!("a{four}b"));
        assert_eq!(sanitize_text("a^Ib"), format!("a{four}b"));
        // A caret that is not `^I` is left alone.
        assert_eq!(sanitize_text("a^b"), "a^b");
    }

    #[test]
    fn sanitize_consumes_style_toggles_without_swallowing_text() {
        // No-argument toggles must consume exactly two characters.
        assert_eq!(sanitize_text("\\Lunder\\l"), "under");
        assert_eq!(sanitize_text("\\Oover\\o"), "over");
        assert_eq!(sanitize_text("\\Kstrike\\k"), "strike");
        // Regression: `\L` must not scan the rest of the run for a `;`.
        assert_eq!(sanitize_text("a\\Lb"), "ab");
    }

    #[test]
    fn sanitize_consumes_parameter_codes() {
        assert_eq!(sanitize_text("\\H2.5;Hi"), "Hi");
        assert_eq!(sanitize_text("\\W0.8;Hi"), "Hi");
        assert_eq!(sanitize_text("\\A1;Hi"), "Hi");
        assert_eq!(sanitize_text("\\Q45;Hi"), "Hi");
        assert_eq!(sanitize_text("\\C1;Hi"), "Hi");
        assert_eq!(sanitize_text("{\\fArial|b0;Hi}"), "Hi");
        // Codes may follow one another without corrupting the text.
        assert_eq!(sanitize_text("\\H1;\\W2;\\C3;ok"), "ok");
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

    /// Outline a run with the standard test parameters.
    fn outline_run(engine: &FontEngine, key: &str, text: &str) -> Vec<Vec<Point3>> {
        engine
            .outline(
                key,
                text,
                Point3::default(),
                10.0,
                0.0,
                TextAlignH::Left,
                TextAlignV::Baseline,
            )
            .unwrap()
    }

    fn max_x(polys: &[Vec<Point3>]) -> f64 {
        polys
            .iter()
            .flatten()
            .map(|p| p.x)
            .fold(f64::NEG_INFINITY, f64::max)
    }

    /// Opt-in per-glyph fallback test (needs both `YACR_TEST_FONT` and
    /// `YACR_TEST_SHX`).
    ///
    /// The SHX face is the primary and the sfnt face the fallback, so this also
    /// exercises the branch where the sfnt face is only consulted for glyphs the
    /// primary lacks.
    #[test]
    fn per_glyph_fallback_uses_first_face_with_the_glyph() {
        let (Ok(sfnt_path), Ok(shx_path)) = (
            std::env::var("YACR_TEST_FONT"),
            std::env::var("YACR_TEST_SHX"),
        ) else {
            return;
        };
        let sfnt_bytes = std::fs::read(sfnt_path).unwrap();
        let shx_bytes = std::fs::read(shx_path).unwrap();

        let mut sfnt_only = FontEngine::new();
        sfnt_only
            .register(
                "arial.woff",
                Arc::from(sfnt_bytes.clone().into_boxed_slice()),
            )
            .unwrap();
        let mut shx_only = FontEngine::new();
        shx_only
            .register(
                "simplex.shx",
                Arc::from(shx_bytes.clone().into_boxed_slice()),
            )
            .unwrap();

        // Find a probe glyph the SHX primary lacks but the sfnt fallback has.
        let probe = ['Ω', 'Ж', 'é', '→', '∑']
            .into_iter()
            .find(|p| {
                let text = p.to_string();
                !outline_run(&sfnt_only, "arial.woff", &text).is_empty()
                    && outline_run(&shx_only, "simplex.shx", &text).is_empty()
            })
            .expect("no probe glyph shared by the supplied test fonts");

        let mut mixed = FontEngine::new();
        mixed
            .register("simplex.shx", Arc::from(shx_bytes.into_boxed_slice()))
            .unwrap();
        mixed
            .register("arial.woff", Arc::from(sfnt_bytes.into_boxed_slice()))
            .unwrap();
        mixed.set_fallback(vec!["arial.woff".into()]);

        // The glyph missing from the primary is supplied by the fallback.
        let probe_text = probe.to_string();
        let fallback_polys = outline_run(&mixed, "simplex.shx", &probe_text);
        assert!(
            !fallback_polys.is_empty(),
            "fallback did not supply {probe:?}"
        );

        // A glyph the primary has still renders exactly as the primary alone.
        let primary_only = outline_run(&shx_only, "simplex.shx", "A");
        let primary_mixed = outline_run(&mixed, "simplex.shx", "A");
        assert_eq!(primary_only, primary_mixed);

        // A run mixing both sources advances past either source alone.
        let mixed_run = outline_run(&mixed, "simplex.shx", &format!("A{probe}"));
        assert!(max_x(&mixed_run) > max_x(&primary_mixed));
        assert!(max_x(&mixed_run) > max_x(&fallback_polys));
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
