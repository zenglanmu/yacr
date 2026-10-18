use super::finalize::dedupe_issues;
use super::model::{ParsedText, StackedFraction, TextColor, TextFormatIssue, TextLine, TextRun};
use super::text_issue;

/// Spaces a tab expands to.
///
/// Tabs are rendered as a fixed run of spaces rather than snapping the pen to
/// proportional tab stops, so `\t`/`^I` spacing is an explicit approximation
/// (see the module-level limitations).
pub(super) const TAB_STEP: usize = 4;

/// Consume a `\X...;` parameter code whose value begins at `start` (the index
/// just after the code letter), returning the index just past the terminating
/// `;` — or the end of input when no `;` is present.
pub(super) fn skip_param_code(chars: &[char], start: usize) -> usize {
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

/// Parse MTEXT/TEXT control content into a structured run list.
///
/// `base_height` is the entity's nominal em height in world units; `\H` values
/// are resolved against it (absolute heights are used as-is, relative factors
/// multiply the current height). The result always has at least one paragraph.
///
/// Malformed or unrecognised escapes never panic: an unrecognised `\X` is
/// rendered literally and reported through [`TextFormatIssue`], while a
/// recognised code with a bad value is consumed and reported.
pub fn parse_mtext(raw: &str, base_height: f64) -> ParsedText {
    MTextParser::new(raw, base_height).run()
}

/// Active span style while parsing; `{`/`}` push and pop a clone of this.
#[derive(Clone)]
struct ParseState {
    font: Option<String>,
    /// Absolute height override from a non-relative `\H`.
    height_abs: Option<f64>,
    /// Relative height multiplier (on the base height, or on `height_abs`).
    height_mul: f64,
    width_factor: f64,
    oblique_deg: f64,
    color: Option<TextColor>,
    line_align: Option<u8>,
    underline: bool,
    overline: bool,
    strike: bool,
}

impl ParseState {
    fn new() -> Self {
        ParseState {
            font: None,
            height_abs: None,
            height_mul: 1.0,
            width_factor: 1.0,
            oblique_deg: 0.0,
            color: None,
            line_align: None,
            underline: false,
            overline: false,
            strike: false,
        }
    }
}

struct MTextParser {
    chars: Vec<char>,
    pos: usize,
    base_height: f64,
    stack: Vec<ParseState>,
    current: ParseState,
    lines: Vec<TextLine>,
    runs: Vec<TextRun>,
    buf: String,
    issues: Vec<TextFormatIssue>,
}

impl MTextParser {
    fn new(raw: &str, base_height: f64) -> Self {
        let base = if base_height.is_finite() && base_height > 0.0 {
            base_height
        } else {
            1.0
        };
        MTextParser {
            chars: raw.chars().collect(),
            pos: 0,
            base_height: base,
            stack: Vec::new(),
            current: ParseState::new(),
            lines: Vec::new(),
            runs: Vec::new(),
            buf: String::new(),
            issues: Vec::new(),
        }
    }

    fn run(mut self) -> ParsedText {
        self.parse_all();
        ParsedText {
            lines: self.lines,
            issues: dedupe_issues(self.issues),
        }
    }

    fn issue(&mut self, code: &'static str, message: impl Into<String>) {
        self.issues.push(TextFormatIssue::new(code, message));
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn parse_all(&mut self) {
        while self.pos < self.chars.len() {
            let ch = self.chars[self.pos];
            match ch {
                '\\' => {
                    self.flush_run();
                    self.parse_code();
                }
                '{' => {
                    self.stack.push(self.current.clone());
                    self.pos += 1;
                }
                '}' => {
                    self.flush_run();
                    if let Some(previous) = self.stack.pop() {
                        self.current = previous;
                    }
                    self.pos += 1;
                }
                '\n' => {
                    self.pos += 1;
                    self.paragraph();
                }
                '%' => self.special_char(),
                '^' => self.caret(),
                c if (c as u32) < 0x20 => {
                    // Any other control character renders as a space.
                    self.buf.push(' ');
                    self.pos += 1;
                }
                _ => {
                    self.buf.push(ch);
                    self.pos += 1;
                }
            }
        }
        self.flush_run();
        // Always terminate the current paragraph, even when empty.
        self.lines.push(TextLine {
            runs: std::mem::take(&mut self.runs),
        });
    }

    fn paragraph(&mut self) {
        self.flush_run();
        self.lines.push(TextLine {
            runs: std::mem::take(&mut self.runs),
        });
    }

    fn flush_run(&mut self) {
        if self.buf.is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.buf);
        let run = self.make_run(text, None);
        self.push_run(run);
    }

    fn push_run(&mut self, run: TextRun) {
        if run.fraction.is_none() {
            if let Some(last) = self.runs.last_mut() {
                if last.fraction.is_none() && last.same_style(&run) {
                    last.text.push_str(&run.text);
                    return;
                }
            }
        }
        self.runs.push(run);
    }

    fn make_run(&self, text: String, fraction: Option<StackedFraction>) -> TextRun {
        let height = self
            .current
            .height_abs
            .unwrap_or(self.base_height * self.current.height_mul);
        TextRun {
            text,
            fraction,
            font: self.current.font.clone(),
            height: if height.is_finite() && height > 0.0 {
                height
            } else {
                self.base_height
            },
            width_factor: self.current.width_factor,
            oblique_deg: self.current.oblique_deg,
            color: self.current.color,
            line_align: self.current.line_align,
            underline: self.current.underline,
            overline: self.current.overline,
            strike: self.current.strike,
        }
    }

    /// Read a `;`-terminated value from the current position. Returns the raw
    /// value and whether the terminating `;` was present.
    fn read_value(&mut self) -> (String, bool) {
        let start = self.pos;
        let mut end = start;
        while end < self.chars.len() && self.chars[end] != ';' {
            end += 1;
        }
        let value: String = self.chars[start..end].iter().collect();
        let terminated = end < self.chars.len();
        self.pos = if terminated { end + 1 } else { end };
        (value, terminated)
    }

    fn parse_code(&mut self) {
        let next = self.pos + 1;
        if next >= self.chars.len() {
            self.buf.push('\\');
            self.issue(text_issue::MALFORMED_CODE, "trailing '\\' with no code");
            self.pos += 1;
            return;
        }
        let code = self.chars[next];
        self.pos = next + 1;
        match code {
            '\\' => self.buf.push('\\'),
            '{' => self.buf.push('{'),
            '}' => self.buf.push('}'),
            ';' => self.buf.push(';'),
            '~' => self.buf.push('\u{00A0}'),
            'P' => self.paragraph(),
            'p' => self.paragraph_code(),
            'N' => {
                self.issue(
                    text_issue::COLUMN_BREAK,
                    "column break '\\N' treated as a plain line break",
                );
                self.paragraph();
            }
            'X' => {}
            'L' => self.current.underline = true,
            'l' => self.current.underline = false,
            'O' => self.current.overline = true,
            'o' => self.current.overline = false,
            'K' => self.current.strike = true,
            'k' => self.current.strike = false,
            'C' => {
                let (value, terminated) = self.read_value();
                self.apply_aci(&value, terminated);
                self.consume_optional_aci_end();
            }
            'c' => {
                let (value, terminated) = self.read_value();
                self.apply_rgb(&value, terminated);
            }
            'f' => {
                let (value, terminated) = self.read_value();
                self.apply_font(&value, false, terminated);
            }
            'F' => {
                let (value, terminated) = self.read_value();
                self.apply_font(&value, true, terminated);
            }
            'H' => {
                let (value, terminated) = self.read_value();
                self.apply_height(&value, terminated);
            }
            'W' | 'w' => {
                let (value, terminated) = self.read_value();
                self.apply_width(&value, terminated);
            }
            'Q' => {
                let (value, terminated) = self.read_value();
                self.apply_oblique(&value, terminated);
            }
            'A' => {
                let (value, terminated) = self.read_value();
                self.apply_align(&value, terminated);
            }
            'S' | 's' => {
                let (value, terminated) = self.read_value();
                self.apply_fraction(&value, terminated);
            }
            'T' => {
                let (_, terminated) = self.read_value();
                self.issue(
                    text_issue::TRACKING_NOT_APPLIED,
                    "character tracking '\\T' parsed but not applied",
                );
                if !terminated {
                    self.issue(
                        text_issue::MALFORMED_CODE,
                        "\\T code is missing its terminating ';'",
                    );
                }
            }
            'B' => {
                let _ = self.read_value();
                self.issue(
                    text_issue::BACKGROUND_MASK,
                    "background mask '\\B' parsed but not drawn",
                );
            }
            'b' => {
                let (value, terminated) = self.read_value();
                match value.trim() {
                    "1" => self.current.strike = true,
                    "0" => self.current.strike = false,
                    _ => self.issue(
                        text_issue::MALFORMED_CODE,
                        format!("\\b value '{value}' is not 0 or 1"),
                    ),
                }
                if !terminated {
                    self.issue(
                        text_issue::MALFORMED_CODE,
                        "\\b code is missing its terminating ';'",
                    );
                }
            }
            'U' if self.peek() == Some('+') => self.unicode_escape(),
            _ => {
                self.buf.push('\\');
                self.buf.push(code);
                self.issue(
                    text_issue::UNKNOWN_CODE,
                    format!("unrecognised MTEXT code '\\{code}' rendered literally"),
                );
            }
        }
    }

    /// `\p`: a bare `\p`/`\p;` is a paragraph break; `\p...;` holds paragraph
    /// properties this build does not lay out.
    fn paragraph_code(&mut self) {
        match self.peek() {
            None | Some(';') => {
                if self.peek() == Some(';') {
                    self.pos += 1;
                }
                self.paragraph();
            }
            Some(_) => {
                let (_, terminated) = self.read_value();
                self.issue(
                    text_issue::PARAGRAPH_PROPERTIES,
                    "\\p paragraph properties (indent/alignment/tabs) ignored",
                );
                if !terminated {
                    self.issue(
                        text_issue::MALFORMED_CODE,
                        "\\p code is missing its terminating ';'",
                    );
                }
            }
        }
    }

    fn special_char(&mut self) {
        let first = self.pos;
        if self.chars.get(first + 1) != Some(&'%') {
            self.buf.push('%');
            self.pos = first + 1;
            return;
        }
        let code_index = first + 2;
        if self.chars.get(code_index) == Some(&'%') && self.chars.get(code_index + 1) == Some(&'%')
        {
            self.buf.push('%');
            self.pos = code_index + 2;
            return;
        }
        if let Some(&code) = self.chars.get(code_index) {
            match code.to_ascii_lowercase() {
                'd' => {
                    self.buf.push('°');
                    self.pos = code_index + 1;
                    return;
                }
                'p' => {
                    self.buf.push('±');
                    self.pos = code_index + 1;
                    return;
                }
                'c' => {
                    self.buf.push('⌀');
                    self.pos = code_index + 1;
                    return;
                }
                _ => {}
            }
            if code.is_ascii_digit() {
                let mut end = code_index;
                while end < self.chars.len() && self.chars[end].is_ascii_digit() {
                    end += 1;
                }
                let digits: String = self.chars[code_index..end].iter().collect();
                if let Ok(value) = digits.parse::<u32>() {
                    if let Some(ch) = char::from_u32(value) {
                        self.buf.push(ch);
                        self.pos = end;
                        return;
                    }
                }
            }
        }
        // Unknown `%%x` — emit both percent signs literally and let the code
        // character fall through as normal text.
        self.buf.push('%');
        self.buf.push('%');
        self.pos = first + 2;
    }

    fn caret(&mut self) {
        match self.chars.get(self.pos + 1).copied() {
            Some('I') => {
                for _ in 0..TAB_STEP {
                    self.buf.push(' ');
                }
                self.pos += 2;
            }
            Some('J') => {
                self.pos += 2;
                self.paragraph();
            }
            Some('M') => self.pos += 2,
            _ => {
                self.buf.push('^');
                self.pos += 1;
            }
        }
    }

    fn unicode_escape(&mut self) {
        // `self.pos` is at the '+' following `\U`.
        self.pos += 1;
        let start = self.pos;
        while self.pos < self.chars.len() && self.chars[self.pos].is_ascii_hexdigit() {
            self.pos += 1;
        }
        let hex: String = self.chars[start..self.pos].iter().collect();
        match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
            Some(ch) => self.buf.push(ch),
            None => self.issue(
                text_issue::MALFORMED_CODE,
                format!("\\U+{hex} is not a valid code point"),
            ),
        }
    }

    fn apply_height(&mut self, value: &str, terminated: bool) {
        let (number, relative) = split_relative(value);
        match number.trim().parse::<f64>() {
            Ok(factor) if factor.is_finite() => {
                let factor = factor.abs();
                if relative {
                    if let Some(absolute) = self.current.height_abs {
                        self.current.height_abs = Some(absolute * factor);
                    } else {
                        self.current.height_mul *= factor;
                    }
                } else {
                    self.current.height_abs = Some(factor);
                }
            }
            _ => self.issue(
                text_issue::MALFORMED_CODE,
                format!("\\H value '{value}' is not a number"),
            ),
        }
        if !terminated {
            self.issue(
                text_issue::MALFORMED_CODE,
                "\\H code is missing its terminating ';'",
            );
        }
    }

    fn apply_width(&mut self, value: &str, terminated: bool) {
        let (number, relative) = split_relative(value);
        match number.trim().parse::<f64>() {
            Ok(factor) if factor.is_finite() => {
                let factor = factor.abs();
                if relative {
                    self.current.width_factor *= factor;
                } else {
                    self.current.width_factor = factor;
                }
            }
            _ => self.issue(
                text_issue::MALFORMED_CODE,
                format!("\\W value '{value}' is not a number"),
            ),
        }
        if !terminated {
            self.issue(
                text_issue::MALFORMED_CODE,
                "\\W code is missing its terminating ';'",
            );
        }
    }

    fn apply_oblique(&mut self, value: &str, terminated: bool) {
        match value.trim().parse::<f64>() {
            Ok(angle) if angle.is_finite() => self.current.oblique_deg = angle,
            _ => self.issue(
                text_issue::MALFORMED_CODE,
                format!("\\Q value '{value}' is not an angle"),
            ),
        }
        if !terminated {
            self.issue(
                text_issue::MALFORMED_CODE,
                "\\Q code is missing its terminating ';'",
            );
        }
    }

    fn apply_align(&mut self, value: &str, terminated: bool) {
        match value.trim().parse::<u8>() {
            Ok(code) if code <= 2 => self.current.line_align = Some(code),
            _ => self.issue(
                text_issue::MALFORMED_CODE,
                format!("\\A value '{value}' is not 0, 1 or 2"),
            ),
        }
        self.issue(
            text_issue::LINE_ALIGNMENT,
            "\\A run alignment parsed but not applied",
        );
        if !terminated {
            self.issue(
                text_issue::MALFORMED_CODE,
                "\\A code is missing its terminating ';'",
            );
        }
    }

    fn apply_font(&mut self, value: &str, is_f: bool, terminated: bool) {
        let mut spec = value.trim();
        if is_f {
            // `\FN{name}.shx;` selects a compiled shape font.
            if let Some(rest) = spec.strip_prefix('N') {
                spec = rest;
            }
        }
        let name = spec.split(['|', ',']).next().unwrap_or("").trim();
        if !name.is_empty() {
            self.current.font = Some(name.to_string());
        }
        if !terminated {
            self.issue(
                text_issue::MALFORMED_CODE,
                "font code is missing its terminating ';'",
            );
        }
    }

    fn apply_aci(&mut self, value: &str, terminated: bool) {
        match value.trim().parse::<i64>() {
            Ok(index) if (0..=256).contains(&index) => {
                self.current.color = Some(TextColor::Aci(index as u16));
            }
            Ok(index) => self.issue(
                text_issue::MALFORMED_CODE,
                format!("ACI color {index} is out of range 0..=256"),
            ),
            Err(_) => self.issue(
                text_issue::MALFORMED_CODE,
                format!("\\C value '{value}' is not an ACI index"),
            ),
        }
        if !terminated {
            self.issue(
                text_issue::MALFORMED_CODE,
                "\\C code is missing its terminating ';'",
            );
        }
    }

    /// Consume an optional second (`;`-terminated) ACI value from `\C1;2;`,
    /// which this build does not use (no gradient rendering).
    fn consume_optional_aci_end(&mut self) {
        let start = self.pos;
        let mut end = start;
        while end < self.chars.len() && (self.chars[end].is_ascii_digit() || self.chars[end] == '-')
        {
            end += 1;
        }
        if end > start && self.chars.get(end) == Some(&';') {
            self.pos = end + 1;
        }
    }

    fn apply_rgb(&mut self, value: &str, terminated: bool) {
        match value.trim().parse::<u32>() {
            Ok(packed) => {
                // MTEXT true color is byte-reversed: low byte is red.
                let red = (packed & 0xFF) as u8;
                let green = ((packed >> 8) & 0xFF) as u8;
                let blue = ((packed >> 16) & 0xFF) as u8;
                self.current.color = Some(TextColor::Rgb([red, green, blue]));
            }
            Err(_) => self.issue(
                text_issue::MALFORMED_CODE,
                format!("\\c value '{value}' is not packed RGB"),
            ),
        }
        if !terminated {
            self.issue(
                text_issue::MALFORMED_CODE,
                "\\c code is missing its terminating ';'",
            );
        }
    }

    fn apply_fraction(&mut self, value: &str, terminated: bool) {
        match split_fraction(value) {
            Some((numerator, denominator, separator)) => {
                let run = self.make_run(
                    String::new(),
                    Some(StackedFraction {
                        numerator,
                        denominator,
                        separator,
                    }),
                );
                self.runs.push(run);
                self.issue(
                    text_issue::STACKED_FRACTION_FLAT,
                    "stacked fraction drawn inline as 'num<sep>den' (no bar/stacking)",
                );
            }
            None => {
                self.issue(
                    text_issue::MALFORMED_CODE,
                    format!("\\S value '{value}' has no '/', '#' or '^' separator"),
                );
                self.buf.push_str(value);
            }
        }
        if !terminated {
            self.issue(
                text_issue::MALFORMED_CODE,
                "\\S code is missing its terminating ';'",
            );
        }
    }
}

/// Split a `\H`/`\W` value into its number and whether it was relative (`x`).
fn split_relative(value: &str) -> (&str, bool) {
    if value.ends_with('x') || value.ends_with('X') {
        (&value[..value.len() - 1], true)
    } else {
        (value, false)
    }
}

/// Split a `\S` payload at its first `/`, `#` or `^` separator.
fn split_fraction(value: &str) -> Option<(String, String, char)> {
    for (index, ch) in value.char_indices() {
        if matches!(ch, '/' | '#' | '^') {
            let numerator = value[..index].to_string();
            let mut denominator = &value[index + ch.len_utf8()..];
            if ch == '^' {
                denominator = denominator.strip_prefix(' ').unwrap_or(denominator);
            }
            return Some((numerator, denominator.to_string(), ch));
        }
    }
    None
}
