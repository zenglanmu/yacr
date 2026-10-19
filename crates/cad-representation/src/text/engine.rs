use std::collections::HashMap;
use std::sync::Arc;

use cad_domain::{CadError, CadResult, Point3, TextAlignH, TextAlignV};
use ttf_parser::OutlineBuilder;

use crate::shx::ShxFont;

use super::faces::{normalize_key, parse_face, stem_of, FaceData};
use super::finalize::{dedupe_issues, finalize_text, FinalizedText};
use super::layout::{layout_parsed, MeasuredRun};
use super::model::{ShapedText, TextFormatIssue, TextRun};
use super::mtext::parse_mtext;
use super::text_issue;

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

    /// Outline one SHX shape glyph (by shape code) into world-space polylines.
    ///
    /// Returns `Unsupported` when `font_key` does not resolve to an SHX shape
    /// font and `InvalidInput` when the code is absent, so a SHAPE entity is
    /// never silently dropped or guessed.
    pub fn shape_glyph(
        &self,
        font_key: &str,
        code: u32,
        origin: Point3,
        size: f64,
        rotation: f64,
    ) -> CadResult<Vec<Vec<Point3>>> {
        if !size.is_finite() || size <= 0.0 {
            return Err(CadError::InvalidInput(
                "shape size must be positive and finite".into(),
            ));
        }
        let Some(face) = self.lookup(font_key) else {
            return Err(self.chain_error(Some(font_key)));
        };
        let FaceData::Shx(font) = &*face else {
            return Err(CadError::Unsupported(format!(
                "font '{font_key}' is not an SHX shape font"
            )));
        };
        let glyph = font.glyph_by_code(code, size).ok_or_else(|| {
            CadError::InvalidInput(format!("shape code {code} is not in font '{font_key}'"))
        })?;
        let (sin, cos) = rotation.sin_cos();
        Ok(glyph
            .polylines
            .iter()
            .filter(|polyline| polyline.len() >= 2)
            .map(|polyline| {
                polyline
                    .iter()
                    .map(|p| Point3 {
                        x: origin.x + p[0] * cos - p[1] * sin,
                        y: origin.y + p[0] * sin + p[1] * cos,
                        z: origin.z,
                    })
                    .collect()
            })
            .collect())
    }

    /// Parse MTEXT/TEXT formatting in `raw` and shape it into world polylines.
    ///
    /// This is the rich counterpart to [`outline`](Self::outline): it honours
    /// the per-run font/height/width/oblique overrides carried by the MTEXT
    /// control content (see [`parse_mtext`]) and returns the flattened styled
    /// runs plus an honest list of [`TextFormatIssue`]s for formatting that
    /// could only be approximated (inline color, decorations, stacked
    /// fractions, ...).
    ///
    /// `origin` is the alignment/anchor point; `height` is the base em height in
    /// world units; `rotation` is radians about `origin`. When the requested
    /// font is missing the fallback chain is used, resolved per glyph.
    #[allow(clippy::too_many_arguments)]
    pub fn shape(
        &self,
        font_key: &str,
        raw: &str,
        origin: Point3,
        height: f64,
        rotation: f64,
        h_align: TextAlignH,
        v_align: TextAlignV,
    ) -> CadResult<ShapedText> {
        self.shape_with(
            Some(font_key),
            raw,
            origin,
            height,
            rotation,
            h_align,
            v_align,
        )
    }

    /// Shape text whose style names no font.
    ///
    /// A missing font name is not an error: the glyphs resolve through the
    /// registered fallback chain, exactly like a requested-but-unavailable font
    /// (the sanctioned fallback mechanism). When the chain holds no usable face
    /// the caller gets `ResourceMissing` and keeps the unshaped placeholder
    /// instead of showing fabricated glyphs.
    #[allow(clippy::too_many_arguments)]
    pub fn shape_unstyled(
        &self,
        raw: &str,
        origin: Point3,
        height: f64,
        rotation: f64,
        h_align: TextAlignH,
        v_align: TextAlignV,
    ) -> CadResult<ShapedText> {
        self.shape_with(None, raw, origin, height, rotation, h_align, v_align)
    }

    #[allow(clippy::too_many_arguments)]
    fn shape_with(
        &self,
        font_key: Option<&str>,
        raw: &str,
        origin: Point3,
        height: f64,
        rotation: f64,
        h_align: TextAlignH,
        v_align: TextAlignV,
    ) -> CadResult<ShapedText> {
        let size = height.abs();
        if !size.is_finite() || size <= 0.0 {
            return Err(CadError::InvalidInput(
                "text height must be a positive finite value".into(),
            ));
        }
        if !self.has_usable_face(font_key) {
            return Err(self.chain_error(font_key));
        }

        let parsed = parse_mtext(raw, size);
        let mut issues = parsed.issues.clone();
        match font_key {
            Some(key) if !self.contains(key) => issues.push(TextFormatIssue::new(
                "text.font_fallback",
                format!("font '{key}' unavailable; rendering with the registered fallback chain"),
            )),
            None => issues.push(TextFormatIssue::new(
                "text.font_fallback",
                "the text style names no font; rendering with the registered fallback chain",
            )),
            Some(_) => {}
        }

        // Shape each paragraph. A run that names an unregistered font is
        // reported, not silently dropped; the rest of the text still renders.
        let mut missing_font: Option<String> = None;
        let mut missing_glyphs = Vec::new();
        let lines = layout_parsed(&parsed, size, |run, text| {
            let key = run.font.as_deref().or(font_key);
            match self.measure_run(key, text, run.effective_height(size)) {
                Some(shaped) => {
                    missing_glyphs.extend(shaped.missing_glyphs);
                    Some(shaped.geometry)
                }
                None => {
                    missing_font.get_or_insert_with(|| {
                        key.map(str::to_string)
                            .unwrap_or_else(|| "the text style's font".into())
                    });
                    None
                }
            }
        });

        if let Some(key) = missing_font {
            issues.push(TextFormatIssue::new(
                text_issue::FONT_UNAVAILABLE,
                format!("run font '{key}' is not registered and no fallback is available"),
            ));
        }
        missing_glyphs.sort_unstable();
        missing_glyphs.dedup();
        if !missing_glyphs.is_empty() {
            issues.push(TextFormatIssue::new(
                "text.glyph_missing",
                format!(
                    "no registered font contains glyphs: {}",
                    missing_glyphs.iter().collect::<String>()
                ),
            ));
        }

        let runs: Vec<TextRun> = parsed
            .lines
            .iter()
            .flat_map(|line| line.runs.iter().cloned())
            .collect();
        if runs.iter().any(|run| run.color.is_some()) {
            issues.push(TextFormatIssue::new(
                text_issue::COLOR_NOT_APPLIED,
                "inline MTEXT color is parsed but line geometry carries no color",
            ));
        }
        if runs
            .iter()
            .any(|run| run.underline || run.overline || run.strike)
        {
            issues.push(TextFormatIssue::new(
                text_issue::DECORATION_NOT_APPLIED,
                "MTEXT underline/overline/strikethrough are parsed but not drawn",
            ));
        }

        let has_renderable_content = runs.iter().any(|run| {
            run.rendered_text()
                .chars()
                .any(|character| !character.is_whitespace())
        });
        let FinalizedText {
            polylines,
            outlines,
            meshes,
            fill_failures,
        } = finalize_text(lines, size, origin, rotation, h_align, v_align);
        for failure in fill_failures {
            issues.push(TextFormatIssue::new(
                "text.sfnt_fill_partial",
                format!("SFNT glyph face was not filled: {failure}"),
            ));
        }
        Ok(ShapedText {
            polylines,
            outlines,
            meshes,
            has_renderable_content,
            runs,
            issues: dedupe_issues(issues),
        })
    }

    /// Outline `text` into world-space polylines at `origin`.
    ///
    /// A thin wrapper over [`shape`](Self::shape) that drops fills, runs and
    /// diagnostics. One polyline per glyph contour; paragraphs in `text` (via
    /// `\P`, a literal `\n` or `^J`) become consecutive lines. `height` is the
    /// base em height in world units and `rotation` is radians about `origin`.
    /// When the requested font is missing, the fallback chain is used, resolved
    /// **per glyph**: each character is drawn with the first face in the chain
    /// (primary, then fallbacks in order) that actually contains it, and the pen
    /// advances by that face's advance. When the primary face has every glyph,
    /// the result is byte-for-byte the same as single-face rendering.
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
        Ok(self
            .shape(font_key, text, origin, height, rotation, h_align, v_align)?
            .outlines)
    }

    /// Whether `font_key` or the fallback chain holds any decodable face.
    fn has_usable_face(&self, font_key: Option<&str>) -> bool {
        self.chain(font_key)
            .iter()
            .any(|face| matches!(&**face, FaceData::Sfnt(_) | FaceData::Shx(_)))
    }

    fn chain(&self, font_key: Option<&str>) -> Vec<Arc<FaceData>> {
        let mut chain: Vec<Arc<FaceData>> = Vec::new();
        if let Some(key) = font_key {
            if let Some(face) = self.lookup(key) {
                chain.push(face);
            }
        }
        for key in &self.fallback {
            if let Some(face) = self.lookup(key) {
                chain.push(face);
            }
        }
        chain
    }

    /// The error to report when [`has_usable_face`](Self::has_usable_face) is
    /// `false`: an explicit `Unsupported` reason if one was recognised, else
    /// `ResourceMissing`. Preserves the pre-MTEXT behaviour.
    fn chain_error(&self, font_key: Option<&str>) -> CadError {
        for face in self.chain(font_key) {
            if let FaceData::Unsupported(reason) = &*face {
                return CadError::Unsupported(reason.clone());
            }
        }
        let label = match font_key {
            Some(key) => format!("font '{key}'"),
            None => "the text style's font".into(),
        };
        CadError::ResourceMissing(format!(
            "{label} is not registered and no fallback is available"
        ))
    }

    /// Shape one run's `text` at `size`, returning run-local glyph polylines
    /// (baseline at y=0, pen starting at x=0) and the unscaled advance.
    ///
    /// Width factor and oblique are applied by [`layout_parsed`], not here, so
    /// the per-glyph fallback/kerning logic stays independent of the MTEXT
    /// style overrides. `None` means no usable face was available.
    fn measure_run(&self, font_key: Option<&str>, text: &str, size: f64) -> Option<MeasuredRun> {
        let chain = self.chain(font_key);
        if chain.is_empty() {
            return None;
        }

        // Keep every sfnt `Arc<[u8]>` alive for the whole call so the parsed
        // faces (which borrow the bytes) stay valid.
        let keepalive: Vec<Arc<[u8]>> = chain
            .iter()
            .filter_map(|face| match &**face {
                FaceData::Sfnt(data) => Some(data.clone()),
                _ => None,
            })
            .collect();

        let mut resolved: Vec<ResolvedFace<'_>> = Vec::new();
        let mut sfnt_index = 0usize;
        for face in &chain {
            match &**face {
                FaceData::Unsupported(_) => {}
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
            return None;
        }

        // `previous` remembers which face supplied the preceding glyph so that
        // kerning is only applied within a single face (cross-face pairs are
        // meaningless).
        let mut previous: Option<(usize, ttf_parser::GlyphId)> = None;
        let mut missing_glyphs = Vec::new();
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
                        let polys = std::mem::take(&mut builder.polys);
                        let mut marked = Vec::with_capacity(polys.len() + 2);
                        if !polys.is_empty() {
                            marked.push(Vec::new());
                            marked.extend(polys);
                            marked.push(Vec::new());
                        }
                        return Some((marked, advance + shift));
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
            missing_glyphs.push(ch);
            None
        });

        let mut polys = Vec::new();
        let mut advance = 0.0;
        for (line_polys, width) in lines {
            polys.extend(line_polys);
            advance += width;
        }
        Some(MeasuredRun {
            geometry: (polys, advance),
            missing_glyphs,
        })
    }
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
