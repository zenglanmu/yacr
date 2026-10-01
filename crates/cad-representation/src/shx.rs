//! Compiled AutoCAD shape fonts (`SHX`).
//!
//! Ported from the MIT-licensed `@mlightcad/shx-parser` reference
//! (<https://github.com/mlightcad/cad-viewer>, package `@mlightcad/shx-parser`),
//! restricted to the formats we can verify. The three content layouts are
//! `shapes`, `unifont` and `bigfont`; the bytecode interpreter is shared.
//!
//! The parser is pure: it turns bytes into polylines in font units. World-space
//! placement and the text pen are handled by [`crate::FontEngine`].

use cad_domain::{CadError, CadResult};

/// A parsed SHX font face.
pub struct ShxFont {
    font_type: ShxType,
    /// Character/shape code -> bytecode.
    data: std::collections::HashMap<u32, Vec<u8>>,
    height: f64,
    /// Declared code page for Unicode -> code mapping.
    encoding: Option<&'static encoding_rs::Encoding>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShxType {
    Shapes,
    Unifont,
    Bigfont,
}

/// One glyph: polylines in font units plus horizontal advance.
pub struct ShxGlyph {
    pub polylines: Vec<Vec<[f64; 2]>>,
    pub advance: f64,
}

impl ShxFont {
    /// Parse SHX bytes. `encoding` names the source code page (for example
    /// `gbk` or `shift-jis`) used to map Unicode back to the font's codes.
    pub fn parse(bytes: &[u8], encoding: Option<&str>) -> CadResult<ShxFont> {
        let (header, content_start) = parse_header(bytes)?;
        let parts: Vec<&str> = header.split(' ').collect();
        if parts.len() < 2 {
            return Err(CadError::CorruptData("SHX header is malformed".into()));
        }
        let font_type = match parts[1].to_ascii_lowercase().as_str() {
            "shapes" => ShxType::Shapes,
            "unifont" => ShxType::Unifont,
            "bigfont" => ShxType::Bigfont,
            other => {
                return Err(CadError::Unsupported(format!(
                    "unsupported SHX font type '{other}'"
                )))
            }
        };
        let mut font = ShxFont {
            font_type,
            data: std::collections::HashMap::new(),
            height: 10.0,
            encoding: encoding.and_then(|e| encoding_rs::Encoding::for_label(e.as_bytes())),
        };
        match font_type {
            ShxType::Shapes => font.parse_shapes(bytes, content_start)?,
            ShxType::Unifont => font.parse_unifont(bytes, content_start)?,
            ShxType::Bigfont => font.parse_bigfont(bytes, content_start)?,
        }
        Ok(font)
    }

    /// Glyph for `ch`, scaled so the font cell is `size` world units tall.
    pub fn glyph(&self, ch: char, size: f64) -> Option<ShxGlyph> {
        let code = self.char_code(ch)?;
        self.glyph_by_code(code, size)
    }

    fn glyph_by_code(&self, code: u32, size: f64) -> Option<ShxGlyph> {
        if code == 0 {
            return None;
        }
        let bytecode = self.data.get(&code)?;
        let shape = self.parse_shape(bytecode, true, true);
        // SHAPES without a shape #0 uses `size` directly; the other formats are
        // normalised by the declared cell height.
        let factor = if self.font_type == ShxType::Shapes && !self.data.contains_key(&0) {
            size
        } else if self.height > 0.0 {
            size / self.height
        } else {
            size
        };
        let scaled: Vec<Vec<[f64; 2]>> = shape
            .polylines
            .iter()
            .map(|poly| {
                poly.iter()
                    .map(|p| [p[0] * factor, p[1] * factor])
                    .collect()
            })
            .collect();
        let cell_width = factor * self.cell_width();
        let advance = resolve_advance(&scaled, shape.last[0] * factor, shape.explicit, cell_width);
        Some(ShxGlyph {
            polylines: scaled,
            advance,
        })
    }

    fn cell_width(&self) -> f64 {
        // `width` defaults to `height`; if absent, use a square cell.
        if self.height > 0.0 {
            self.height
        } else {
            10.0
        }
    }

    fn char_code(&self, ch: char) -> Option<u32> {
        if let Some(encoding) = self.encoding {
            let text = ch.to_string();
            let (bytes, _, _) = encoding.encode(&text);
            let b = bytes.as_ref();
            return match b.len() {
                0 => None,
                1 => Some(b[0] as u32),
                _ => Some(((b[0] as u32) << 8) | b[1] as u32),
            };
        }
        let code = ch as u32;
        (code <= 0xFFFF).then_some(code)
    }

    // ---- content parsers ----

    fn parse_shapes(&mut self, bytes: &[u8], mut pos: usize) -> CadResult<()> {
        pos += 4;
        let count = read_i16(bytes, &mut pos)?;
        if count <= 0 {
            return Err(CadError::CorruptData("SHX shape count is invalid".into()));
        }
        let mut entries = Vec::new();
        for _ in 0..count {
            let code = read_u16(bytes, &mut pos)? as u32;
            let len = read_u16(bytes, &mut pos)? as usize;
            if len > 0 {
                entries.push((code, len));
            }
        }
        for (code, len) in entries {
            let raw = take(bytes, &mut pos, len)?;
            self.data.insert(code, strip_name(raw));
        }
        self.read_shapes_metrics();
        Ok(())
    }

    fn parse_unifont(&mut self, bytes: &[u8], mut pos: usize) -> CadResult<()> {
        let count = read_i32(bytes, &mut pos)?;
        let info_len = read_u16(bytes, &mut pos)? as usize;
        let info = take(bytes, &mut pos, info_len)?;
        self.read_info_metrics(info);
        if count <= 0 {
            return Err(CadError::CorruptData("SHX unifont count is invalid".into()));
        }
        for _ in 0..count.saturating_sub(1) {
            let code = read_u16(bytes, &mut pos)? as u32;
            let len = read_u16(bytes, &mut pos)? as usize;
            if len > 0 {
                let raw = take(bytes, &mut pos, len)?;
                self.data.insert(code, strip_name(raw));
            }
        }
        Ok(())
    }

    fn parse_bigfont(&mut self, bytes: &[u8], mut pos: usize) -> CadResult<()> {
        let _skip = read_i16(bytes, &mut pos)?;
        let count = read_i16(bytes, &mut pos)?;
        let skip = read_i16(bytes, &mut pos)?;
        if count <= 0 {
            return Err(CadError::CorruptData("SHX bigfont count is invalid".into()));
        }
        pos += (skip.max(0) as usize) * 4;
        let mut entries = Vec::new();
        for _ in 0..count {
            let code = read_u16(bytes, &mut pos)? as u32;
            let len = read_u16(bytes, &mut pos)? as usize;
            let offset = read_u32(bytes, &mut pos)? as usize;
            if len > 0 {
                entries.push((code, len, offset));
            }
        }
        for (code, len, offset) in entries {
            if offset + len > bytes.len() {
                continue;
            }
            self.data
                .insert(code, strip_name(&bytes[offset..offset + len]).to_vec());
        }
        if let Some(info) = self.data.get(&0).cloned() {
            self.read_bigfont_metrics(&info);
        }
        Ok(())
    }

    fn read_shapes_metrics(&mut self) {
        let Some(info) = self.data.get(&0).cloned() else {
            return;
        };
        if let Some(idx) = info.iter().position(|b| [13u8, 10, 0].contains(b)) {
            if idx + 2 < info.len() {
                let base_up = info[idx + 1] as f64;
                let base_down = info[idx + 2] as f64;
                self.height = (base_up + base_down).max(1.0);
            }
        }
    }

    fn read_info_metrics(&mut self, info: &[u8]) {
        if let Some(idx) = info.iter().position(|&b| b == 0) {
            if idx + 2 < info.len() {
                let base_up = info[idx + 1] as f64;
                let base_down = info[idx + 2] as f64;
                self.height = (base_up + base_down).max(1.0);
            }
        }
    }

    fn read_bigfont_metrics(&mut self, info: &[u8]) {
        // Bigfont info blocks are UTF-8-ish text followed by metrics; use the
        // declared height when present, otherwise the ASCII-cell default.
        if let Some(idx) = info.iter().position(|&b| b == 0) {
            let rest = &info[idx + 1..];
            if rest.len() >= 4 {
                let base_up = rest[0] as f64;
                let base_down = rest[1] as f64;
                self.height = (base_up + base_down).max(1.0);
            }
        }
    }

    // ---- bytecode interpreter ----

    fn parse_shape(&self, data: &[u8], initial_pen_down: bool, flush_on_end: bool) -> Shape {
        let mut state = State::new(initial_pen_down, flush_on_end);
        let mut i = 0usize;
        while i < data.len() {
            let cmd = data[i];
            if cmd <= 15 {
                i = self.special(cmd, data, i, &mut state);
            } else {
                state.clear_pending();
                state.vector(cmd);
            }
            i += 1;
        }
        state.finalize();
        Shape {
            polylines: state.finish_polylines(),
            last: state.current,
            explicit: state.explicit,
        }
    }

    fn special(&self, cmd: u8, data: &[u8], mut i: usize, s: &mut State) -> usize {
        match cmd {
            0 => {
                s.finalize();
                if s.flush_on_end && s.cur.len() > 1 {
                    s.polylines.push(std::mem::take(&mut s.cur));
                } else if s.flush_on_end {
                    s.cur.clear();
                }
                s.pen = false;
            }
            1 => {
                s.clear_pending();
                if !s.pen {
                    s.cur.push(s.current);
                }
                s.pen = true;
            }
            2 => {
                s.pen = false;
                if s.cur.len() > 1 {
                    s.polylines.push(std::mem::take(&mut s.cur));
                } else {
                    s.cur.clear();
                }
            }
            3 => {
                s.clear_pending();
                i += 1;
                if let Some(&d) = data.get(i) {
                    if d != 0 {
                        s.scale /= d as f64;
                    }
                }
            }
            4 => {
                s.clear_pending();
                i += 1;
                if let Some(&m) = data.get(i) {
                    s.scale *= m as f64;
                }
            }
            5 => {
                s.clear_pending();
                if s.stack.len() >= 4 {
                    return i;
                }
                s.stack.push(s.current);
            }
            6 => {
                s.clear_pending();
                s.current = s.stack.pop().unwrap_or(s.current);
                if s.cur.len() > 1 {
                    s.polylines.push(std::mem::take(&mut s.cur));
                }
                if s.pen {
                    s.cur.push(s.current);
                }
            }
            7 => {
                s.clear_pending();
                i = self.subshape(data, i, s);
            }
            8 => {
                i += 1;
                let dx = sbyte(data.get(i).copied().unwrap_or(0));
                i += 1;
                let dy = sbyte(data.get(i).copied().unwrap_or(0));
                s.current[0] += dx * s.scale;
                s.current[1] += dy * s.scale;
                if s.pen {
                    s.cur.push(s.current);
                } else {
                    s.pending = true;
                }
            }
            9 => {
                while i + 1 < data.len() {
                    i += 1;
                    let dx = sbyte(data.get(i).copied().unwrap_or(0));
                    i += 1;
                    let dy = sbyte(data.get(i).copied().unwrap_or(0));
                    if dx == 0.0 && dy == 0.0 {
                        break;
                    }
                    s.current[0] += dx * s.scale;
                    s.current[1] += dy * s.scale;
                    if s.pen {
                        s.cur.push(s.current);
                    } else {
                        s.pending = true;
                    }
                }
            }
            10 => {
                s.clear_pending();
                i += 1;
                let radius = data.get(i).copied().unwrap_or(0) as f64 * s.scale;
                i += 1;
                let raw = sbyte(data.get(i).copied().unwrap_or(0));
                let start_octant = ((raw as i32 & 112) >> 4) as u8;
                let mut octants = (raw as i32 & 7) as u8;
                let clockwise = raw < 0.0;
                if octants == 0 {
                    octants = 8;
                }
                let angle = std::f64::consts::FRAC_PI_4 * start_octant as f64;
                let center = [
                    s.current[0] - angle.cos() * radius,
                    s.current[1] - angle.sin() * radius,
                ];
                let pts = tessellate_octant(center, radius, start_octant, octants, clockwise);
                if s.pen {
                    s.cur.pop();
                    s.cur.extend_from_slice(&pts);
                }
                s.current = *pts.last().unwrap_or(&s.current);
            }
            11 => {
                s.clear_pending();
                i += 1;
                let start_off = data.get(i).copied().unwrap_or(0) as f64;
                i += 1;
                let end_off = data.get(i).copied().unwrap_or(0) as f64;
                i += 1;
                let b1 = data.get(i).copied().unwrap_or(0) as f64;
                i += 1;
                let b2 = data.get(i).copied().unwrap_or(0) as f64;
                let radius = (b1 * 255.0 + b2) * s.scale;
                i += 1;
                let raw = sbyte(data.get(i).copied().unwrap_or(0));
                let start_octant = ((raw as i32 & 112) >> 4) as f64;
                let mut count = (raw as i32 & 7) as f64;
                if count == 0.0 {
                    count = 8.0;
                }
                if end_off != 0.0 {
                    count -= 1.0;
                }
                let step = std::f64::consts::FRAC_PI_4;
                let mut signed_step = step;
                let mut sign = 1.0;
                let mut sweep = step * count;
                if raw < 0.0 {
                    signed_step = -signed_step;
                    sweep = -sweep;
                    sign = -1.0;
                }
                let mut start = step * start_octant;
                let mut end = start + sweep;
                start += step * start_off / 256.0 * sign;
                end += step * end_off / 256.0 * sign;
                let center = [
                    s.current[0] - radius * start.cos(),
                    s.current[1] - radius * start.sin(),
                ];
                s.current = [
                    center[0] + radius * end.cos(),
                    center[1] + radius * end.sin(),
                ];
                if s.pen {
                    let mut pts = Vec::new();
                    let mut a = start;
                    pts.push([center[0] + radius * a.cos(), center[1] + radius * a.sin()]);
                    if signed_step > 0.0 {
                        while a + signed_step < end {
                            a += signed_step;
                            pts.push([center[0] + radius * a.cos(), center[1] + radius * a.sin()]);
                        }
                    } else {
                        while a + signed_step > end {
                            a += signed_step;
                            pts.push([center[0] + radius * a.cos(), center[1] + radius * a.sin()]);
                        }
                    }
                    pts.push([
                        center[0] + radius * end.cos(),
                        center[1] + radius * end.sin(),
                    ]);
                    s.cur.extend_from_slice(&pts);
                }
            }
            12 => {
                i += 1;
                let dx = sbyte(data.get(i).copied().unwrap_or(0));
                i += 1;
                let dy = sbyte(data.get(i).copied().unwrap_or(0));
                i += 1;
                let bulge = sbyte(data.get(i).copied().unwrap_or(0));
                s.arc(dx, dy, bulge);
            }
            13 => {
                while i + 1 < data.len() {
                    i += 1;
                    let dx = sbyte(data.get(i).copied().unwrap_or(0));
                    i += 1;
                    let dy = sbyte(data.get(i).copied().unwrap_or(0));
                    if dx == 0.0 && dy == 0.0 || i + 1 >= data.len() {
                        break;
                    }
                    i += 1;
                    let bulge = sbyte(data.get(i).copied().unwrap_or(0));
                    s.arc(dx, dy, bulge);
                }
            }
            14 => {
                s.clear_pending();
                // Vertical-only flag: in horizontal layout skip the next command.
                if self.font_type != ShxType::Bigfont {
                    i = skip_code(data, i + 1);
                }
            }
            _ => {}
        }
        i
    }

    fn subshape(&self, data: &[u8], mut i: usize, s: &mut State) -> usize {
        let code = match self.font_type {
            ShxType::Unifont => {
                i += 1;
                let hi = data.get(i).copied().unwrap_or(0) as u32;
                i += 1;
                let lo = data.get(i).copied().unwrap_or(0) as u32;
                (hi << 8) | lo
            }
            _ => {
                i += 1;
                data.get(i).copied().unwrap_or(0) as u32
            }
        };
        if code == 0 {
            return i;
        }
        if let Some(child) = self.data.get(&code) {
            let child_shape = self.parse_shape(child, true, false);
            let factor = if self.font_type == ShxType::Shapes && !self.data.contains_key(&0) {
                s.scale
            } else if self.height > 0.0 {
                s.scale * self.height / self.height
            } else {
                s.scale
            };
            if s.cur.len() > 1 {
                s.polylines.push(std::mem::take(&mut s.cur));
            }
            let base = s.current;
            for poly in child_shape.polylines {
                let mapped: Vec<[f64; 2]> = poly
                    .iter()
                    .map(|p| [base[0] + p[0] * factor, base[1] + p[1] * factor])
                    .collect();
                s.polylines.push(mapped);
            }
            s.current = [
                base[0] + child_shape.last[0] * factor,
                base[1] + child_shape.last[1] * factor,
            ];
            s.cur.clear();
            if s.pen {
                s.cur.push(s.current);
            }
        }
        i
    }
}

/// Interpreter state.
struct State {
    current: [f64; 2],
    polylines: Vec<Vec<[f64; 2]>>,
    cur: Vec<[f64; 2]>,
    stack: Vec<[f64; 2]>,
    pen: bool,
    scale: f64,
    explicit: bool,
    pending: bool,
    flush_on_end: bool,
}

struct Shape {
    polylines: Vec<Vec<[f64; 2]>>,
    last: [f64; 2],
    explicit: bool,
}

impl State {
    fn new(initial_pen_down: bool, flush_on_end: bool) -> Self {
        let mut state = State {
            current: [0.0, 0.0],
            polylines: Vec::new(),
            cur: Vec::new(),
            stack: Vec::new(),
            pen: initial_pen_down,
            scale: 1.0,
            explicit: false,
            pending: false,
            flush_on_end,
        };
        if initial_pen_down {
            state.cur.push([0.0, 0.0]);
        }
        state
    }

    fn vector(&mut self, cmd: u8) {
        let len = (cmd >> 4) as f64;
        let dir = cmd & 15;
        let v = direction(dir);
        self.current[0] += v[0] * len * self.scale;
        self.current[1] += v[1] * len * self.scale;
        if self.pen {
            self.cur.push(self.current);
        }
    }

    fn arc(&mut self, dx: f64, dy: f64, bulge: f64) {
        let start = self.current;
        let end = [start[0] + dx * self.scale, start[1] + dy * self.scale];
        if self.pen {
            if bulge == 0.0 {
                self.cur.push(end);
            } else {
                let pts = tessellate_bulge(start, end, bulge / 127.0);
                self.cur.extend_from_slice(&pts[1..]);
            }
        }
        self.current = end;
    }

    fn clear_pending(&mut self) {
        self.pending = false;
    }

    fn finalize(&mut self) {
        if !self.pending {
            return;
        }
        let has_ink = self.cur.len() > 1 || self.polylines.iter().any(|p| p.len() >= 2);
        if self.current[0].abs() > 1e-6 || !has_ink {
            self.explicit = true;
        }
    }

    fn finish_polylines(&self) -> Vec<Vec<[f64; 2]>> {
        let mut out = self.polylines.clone();
        if self.cur.len() > 1 {
            out.push(self.cur.clone());
        }
        out
    }
}

fn direction(dir: u8) -> [f64; 2] {
    match dir {
        0 => [1.0, 0.0],
        1 => [1.0, 0.5],
        2 => [1.0, 1.0],
        3 => [0.5, 1.0],
        4 => [0.0, 1.0],
        5 => [-0.5, 1.0],
        6 => [-1.0, 1.0],
        7 => [-1.0, 0.5],
        8 => [-1.0, 0.0],
        9 => [-1.0, -0.5],
        10 => [-1.0, -1.0],
        11 => [-0.5, -1.0],
        12 => [0.0, -1.0],
        13 => [0.5, -1.0],
        14 => [1.0, -1.0],
        _ => [1.0, -0.5],
    }
}

fn resolve_advance(
    polylines: &[Vec<[f64; 2]>],
    last_x: f64,
    explicit: bool,
    cell_width: f64,
) -> f64 {
    let has_ink = polylines.iter().any(|p| p.len() >= 2);
    if explicit {
        return last_x;
    }
    if !has_ink {
        return cell_width * 0.2;
    }
    let max_x = polylines
        .iter()
        .flat_map(|p| p.iter().map(|q| q[0]))
        .fold(f64::NEG_INFINITY, f64::max);
    let min_x = polylines
        .iter()
        .flat_map(|p| p.iter().map(|q| q[0]))
        .fold(f64::INFINITY, f64::min);
    let padding = cell_width * 0.2;
    if min_x < -1e-6 {
        max_x.max(cell_width / 2.0) + padding
    } else {
        max_x + padding
    }
}

fn tessellate_octant(
    center: [f64; 2],
    radius: f64,
    start: u8,
    count: u8,
    clockwise: bool,
) -> Vec<[f64; 2]> {
    let step = std::f64::consts::FRAC_PI_4;
    let start_angle = start as f64 * step;
    let sweep = (if count == 0 { 8 } else { count }) as f64 * step;
    tessellate_arc(
        center,
        radius,
        start_angle,
        start_angle - sweep,
        start_angle + sweep,
        clockwise,
    )
}

fn tessellate_arc(
    center: [f64; 2],
    radius: f64,
    start_angle: f64,
    cw_end: f64,
    ccw_end: f64,
    clockwise: bool,
) -> Vec<[f64; 2]> {
    let end = if clockwise { cw_end } else { ccw_end };
    let span = (end - start_angle).abs();
    let steps = (span / (std::f64::consts::PI / 18.0)).floor().max(1.0) as usize;
    let mut pts = Vec::with_capacity(steps + 1);
    pts.push([
        center[0] + radius * start_angle.cos(),
        center[1] + radius * start_angle.sin(),
    ]);
    for i in 1..steps {
        let t = i as f64 / steps as f64;
        let a = start_angle + (end - start_angle) * t;
        pts.push([center[0] + radius * a.cos(), center[1] + radius * a.sin()]);
    }
    pts.push([
        center[0] + radius * end.cos(),
        center[1] + radius * end.sin(),
    ]);
    pts
}

fn tessellate_bulge(start: [f64; 2], end: [f64; 2], bulge: f64) -> Vec<[f64; 2]> {
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    let chord = (dx * dx + dy * dy).sqrt();
    if chord <= 1e-12 {
        return vec![start, end];
    }
    let b = bulge.clamp(-1.0, 1.0);
    let sweep = 4.0 * b.atan();
    let radius = chord / (2.0 * (sweep / 2.0).sin().abs()).max(1e-12);
    let mid = [(start[0] + end[0]) / 2.0, (start[1] + end[1]) / 2.0];
    let mut normal = [-dy / chord, dx / chord];
    normal[0] *= (radius * (sweep / 2.0).cos()).abs();
    normal[1] *= (radius * (sweep / 2.0).cos()).abs();
    let clockwise = b < 0.0;
    let center = if clockwise {
        [mid[0] - normal[0], mid[1] - normal[1]]
    } else {
        [mid[0] + normal[0], mid[1] + normal[1]]
    };
    let start_angle = (start[1] - center[1]).atan2(start[0] - center[0]);
    let end_angle = (end[1] - center[1]).atan2(end[0] - center[0]);
    tessellate_arc(center, radius, start_angle, end_angle, end_angle, clockwise)
}

// ---- byte helpers ----

fn read_u16(bytes: &[u8], pos: &mut usize) -> CadResult<u16> {
    if *pos + 2 > bytes.len() {
        return Err(CadError::CorruptData("SHX read past end".into()));
    }
    let v = u16::from_le_bytes([bytes[*pos], bytes[*pos + 1]]);
    *pos += 2;
    Ok(v)
}

fn read_i16(bytes: &[u8], pos: &mut usize) -> CadResult<i16> {
    Ok(read_u16(bytes, pos)? as i16)
}

fn read_i32(bytes: &[u8], pos: &mut usize) -> CadResult<i32> {
    Ok(read_u32(bytes, pos)? as i32)
}

fn read_u32(bytes: &[u8], pos: &mut usize) -> CadResult<u32> {
    if *pos + 4 > bytes.len() {
        return Err(CadError::CorruptData("SHX read past end".into()));
    }
    let v = u32::from_le_bytes([
        bytes[*pos],
        bytes[*pos + 1],
        bytes[*pos + 2],
        bytes[*pos + 3],
    ]);
    *pos += 4;
    Ok(v)
}

fn take<'a>(bytes: &'a [u8], pos: &mut usize, len: usize) -> CadResult<&'a [u8]> {
    if *pos + len > bytes.len() {
        return Err(CadError::CorruptData("SHX section is truncated".into()));
    }
    let slice = &bytes[*pos..*pos + len];
    *pos += len;
    Ok(slice)
}

fn strip_name(raw: &[u8]) -> Vec<u8> {
    match raw.iter().position(|&b| b == 0) {
        None => raw.to_vec(),
        Some(0) => raw.to_vec(),
        Some(idx) => raw[idx + 1..].to_vec(),
    }
}

fn sbyte(b: u8) -> f64 {
    ((b & 127) as i32 - if b & 128 != 0 { 128 } else { 0 }) as f64
}

fn parse_header(bytes: &[u8]) -> CadResult<(String, usize)> {
    let mut text = String::new();
    let mut pos = 0usize;
    while pos < bytes.len().saturating_sub(2) && text.len() < 1024 {
        let b = bytes[pos];
        pos += 1;
        if b == 13 {
            let save = pos;
            let r = bytes.get(pos).copied().unwrap_or(0);
            let h = bytes.get(pos + 1).copied().unwrap_or(0);
            if r == 10 && h == 26 {
                pos += 2;
                break;
            }
            pos = save;
        }
        text.push(b as char);
    }
    Ok((text.trim().to_string(), pos))
}

fn skip_code(data: &[u8], start: usize) -> usize {
    let mut i = start;
    if i >= data.len() {
        return i;
    }
    match data[i] {
        3 | 4 => i + 1,
        7 => i + 1,
        8 => i + 2,
        9 => {
            while i + 1 < data.len() {
                i += 1;
                let dx = data[i];
                i += 1;
                let dy = data[i];
                if dx == 0 && dy == 0 {
                    break;
                }
            }
            i
        }
        10 => i + 2,
        11 => i + 5,
        12 => i + 3,
        13 => {
            while i + 1 < data.len() {
                i += 1;
                let dx = data[i];
                i += 1;
                let dy = data[i];
                if dx == 0 && dy == 0 {
                    break;
                }
                i += 1;
            }
            i
        }
        _ => i,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_shx_bytes() {
        assert!(ShxFont::parse(b"not a font", None).is_err());
        assert!(ShxFont::parse(&[0u8; 64], None).is_err());
    }

    #[test]
    fn direction_table_is_unit_octant() {
        let d = direction(0);
        assert_eq!(d, [1.0, 0.0]);
        let d = direction(12);
        assert_eq!(d, [0.0, -1.0]);
        let d = direction(2);
        assert_eq!(d, [1.0, 1.0]);
    }

    #[test]
    fn advance_falls_back_to_cell_width_without_ink() {
        assert_eq!(resolve_advance(&[], 0.0, false, 10.0), 2.0);
    }

    #[test]
    fn advance_uses_ink_width_with_padding() {
        let poly = vec![vec![[0.0, 0.0], [4.0, 0.0]]];
        // maxX 4 + 0.2 * cellWidth 10 = 6
        assert_eq!(resolve_advance(&poly, 0.0, false, 10.0), 6.0);
        // explicit advance wins
        assert_eq!(resolve_advance(&poly, 7.0, true, 10.0), 7.0);
    }
}
