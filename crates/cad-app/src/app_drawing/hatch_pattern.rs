//! AutoCAD `.pat` pattern library (`HATCH` pattern definitions by name).
//!
//! This module is the single reader of `.pat` definition text:
//!
//! ```text
//! ; a comment
//! *NAME, description
//! angle, base-x, base-y, delta-x, delta-y [, dash ...]
//! ```
//!
//! The three built-in HATCH patterns are themselves `.pat` text
//! ([`BUILTIN_PATTERN_TEXT`], our ADR U-1 transcription), parsed once per process
//! by [`PatternLibrary::builtin`], so **every** predefined HATCH resolves its rows
//! through this loader (header name → rows → hatch-plane frame). A user
//! `acad.pat`/`acadiso.pat` file goes through the same [`PatternLibrary::parse`]
//! and [`PatternLibrary::resolve`].
//!
//! **Declared boundary**: the HATCH tool parameter `hatchPattern` is a closed
//! **1-based index** over the implemented library, and `HATCHEDIT PATTERN
//! <name>` selects a **built-in** name from it. A user `.pat` entry cannot yet
//! be *selected*: it needs host file I/O (there is no pattern-file parameter or
//! session library), so the loader, the parse refusals
//! (`hatch.pattern_file_*`) and the by-name resolution
//! (`hatch.pattern_missing`) are complete and exercised but only the built-in
//! library is reachable from the command line. Until that channel exists no user
//! pattern is silently accepted, approximated or clamped onto a built-in.
//!
//! Split into its own file to keep every code file inside the 1000-line gate.

use super::*;

/// The three built-in patterns in **AutoCAD `.pat` syntax**.
///
/// The built-ins are `.pat` transcriptions (ADR U-1: our values, not
/// vendor-verified), so they are stored in the format they came from: every
/// predefined HATCH therefore exercises the parser below. Row syntax is
/// `angle, base-x, base-y, delta-x, delta-y [, dash ...]`, with the base point
/// and the delta in the row's **own rotated frame** (`;` starts a comment).
/// `hatch_builtin_pat_library_matches_the_predefined_table` pins these rows
/// against `cad_domain::hatch::predefined_patterns()` so the two transcriptions
/// cannot drift.
const BUILTIN_PATTERN_TEXT: &str = "\
; The implemented predefined HATCH patterns (index order). Our transcription
; (ADR U-1), not a vendor-verified acad.pat image.
*ANSI31, Iron, brick and stone masonry: 45 degree lines
45, 0,0, 0,.125
*ANSI32, Steel: dashed 45 degree lines
45, 0,0, 0,.375, .125,-.0625
*ANSI37, Lead, zinc and sound insulation: crossed 45 and 135 degree lines
45, 0,0, 0,.125
135, 0,0, 0,.125
";

/// The largest pattern count one `.pat` library may hold.
const PATTERN_LIBRARY_MAX_PATTERNS: usize = 4096;
/// The largest number of definition rows one pattern may hold.
const PATTERN_LIBRARY_MAX_ROWS: usize = 64;
/// AutoCAD's pattern-name length limit (31 characters).
const PATTERN_LIBRARY_MAX_NAME: usize = 31;

/// One `.pat` definition row, in the frame the file states it (degrees,
/// un-rotated delta).
#[derive(Debug, Clone, PartialEq)]
struct PatternRow {
    /// Line direction in **degrees**, as written.
    angle_deg: f64,
    /// A point on line `k = 0`, in the row's rotated frame.
    base: [f64; 2],
    /// Step to the next line, in the row's rotated frame.
    offset: [f64; 2],
    /// Dash pattern: positive = dash, negative = gap, empty = solid.
    dashes: Vec<f64>,
}

impl PatternRow {
    /// Rotate the row into the hatch-plane frame that
    /// [`HatchPattern::families`] consumes.
    ///
    /// This is the exact conversion
    /// `cad_domain::hatch::PredefinedPatternLine::to_hatch_pattern_line` applies,
    /// so a `.pat` pattern and its transcribed table entry compose to the same
    /// geometry: the family angle is `row angle + hatch angle`, and the
    /// perpendicular spacing is `offset * scale`.
    fn to_hatch_pattern_line(&self) -> HatchPatternLine {
        let angle = self.angle_deg.to_radians();
        let (sin, cos) = angle.sin_cos();
        HatchPatternLine {
            angle,
            base_x: self.base[0],
            base_y: self.base[1],
            offset_x: cos * self.offset[0] - sin * self.offset[1],
            offset_y: sin * self.offset[0] + cos * self.offset[1],
            dashes: self.dashes.clone(),
        }
    }
}

/// One named pattern: the `.pat` header name plus its definition rows.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct PatternDefinition {
    /// The name exactly as the file/table wrote it (matching is trimmed and
    /// ASCII case-insensitive, like the import path's name lookup).
    name: String,
    rows: Vec<PatternRow>,
}

impl PatternDefinition {
    /// The pattern name (the value written to DXF group 2).
    pub(super) fn name(&self) -> &str {
        &self.name
    }

    /// Build the domain pattern, applying the hatch-level scale, angle (degrees
    /// on the channel) and `HPDOUBLE` flag exactly like
    /// `PredefinedPattern::to_hatch_pattern`.
    ///
    /// A definition resolved from a user `.pat` file is a **custom** pattern in
    /// the DXF sense (group 76 = 2); the built-in library is written as
    /// **predefined** (76 = 1). The origin is the caller's decision, so this
    /// method never guesses it.
    pub(super) fn to_hatch_pattern(
        &self,
        scale: f64,
        angle_deg: f64,
        double: bool,
    ) -> HatchPattern {
        self.to_hatch_pattern_radians(scale, angle_deg.to_radians(), double)
    }

    /// Build the domain pattern from an **already-stored** angle in radians.
    ///
    /// `HATCHEDIT PATTERN` keeps the hatch's stored group 52 verbatim, so the
    /// degrees → radians conversion of the creation path must not be applied
    /// twice (a round trip through `to_degrees` is not guaranteed bit-identical,
    /// which would also defeat the "repeating the same edit changes nothing"
    /// contract). Everything else matches [`Self::to_hatch_pattern`].
    pub(super) fn to_hatch_pattern_radians(
        &self,
        scale: f64,
        angle_radians: f64,
        double: bool,
    ) -> HatchPattern {
        HatchPattern {
            name: self.name.clone(),
            angle: angle_radians,
            scale,
            double,
            lines: self
                .rows
                .iter()
                .map(PatternRow::to_hatch_pattern_line)
                .collect(),
        }
    }
}

/// A parsed pattern library: an ordered, name-addressable set of `.pat`
/// definitions.
///
/// [`Self::parse`] is the single `.pat` reader: the built-in library is parsed
/// from [`BUILTIN_PATTERN_TEXT`] with it, and a user `acad.pat`/`acadiso.pat`
/// file goes through the same function (see the module docs for the channel
/// this build still lacks).
#[derive(Debug, Clone, PartialEq)]
pub(super) struct PatternLibrary {
    definitions: Vec<PatternDefinition>,
}

impl PatternLibrary {
    /// Parse `.pat` text.
    ///
    /// Supported constructs: `;` comments, `*NAME, description` headers and
    /// `angle, base-x, base-y, delta-x, delta-y [, dash ...]` rows. Everything
    /// else is an explicit refusal (never skipped, never approximated):
    ///
    /// * `hatch.pattern_file_syntax` — a malformed row/header, a non-numeric or
    ///   non-finite field, a row before the first header, an all-zero dash list;
    /// * `hatch.pattern_file_name` — an empty or over-long pattern name, or a
    ///   name with characters a `.pat` header cannot carry;
    /// * `hatch.pattern_file_duplicate` — the same name (ASCII case-insensitive)
    ///   twice in one library;
    /// * `hatch.pattern_file_spacing` — a row whose delta is zero (no line
    ///   family: the family would be a single infinitely repeated line);
    /// * `hatch.pattern_file_no_rows` — a header with no definition rows (a
    ///   name-only pattern such as `*SOLID` has no line family, and this build
    ///   draws patterns as line families only);
    /// * `hatch.pattern_file_budget` — more than
    ///   [`PATTERN_LIBRARY_MAX_PATTERNS`] patterns or
    ///   [`PATTERN_LIBRARY_MAX_ROWS`] rows in one pattern.
    pub(super) fn parse(text: &str) -> CadResult<Self> {
        let mut definitions: Vec<PatternDefinition> = Vec::new();
        for (index, raw) in text.lines().enumerate() {
            let line = index + 1;
            let content = raw.split(';').next().unwrap_or_default().trim();
            if content.is_empty() {
                continue;
            }
            let Some(rest) = content.strip_prefix('*') else {
                let Some(definition) = definitions.last_mut() else {
                    return Err(pattern_file_refusal(
                        "syntax",
                        format!(
                            "line {line}: a definition row appears before any `*NAME` header: \
                             {content}"
                        ),
                    ));
                };
                if definition.rows.len() >= PATTERN_LIBRARY_MAX_ROWS {
                    return Err(pattern_file_refusal(
                        "budget",
                        format!(
                            "line {line}: pattern '{}' already has {} definition rows (the limit \
                             is {PATTERN_LIBRARY_MAX_ROWS})",
                            definition.name,
                            definition.rows.len()
                        ),
                    ));
                }
                let row = parse_pattern_row(content, line, &definition.name)?;
                definition.rows.push(row);
                continue;
            };
            let name = rest.split(',').next().unwrap_or_default().trim();
            validate_pattern_name(name, line)?;
            if definitions.len() >= PATTERN_LIBRARY_MAX_PATTERNS {
                return Err(pattern_file_refusal(
                    "budget",
                    format!(
                        "line {line}: the library already holds {PATTERN_LIBRARY_MAX_PATTERNS} \
                         patterns (the limit)"
                    ),
                ));
            }
            if definitions
                .iter()
                .any(|definition| definition.name.eq_ignore_ascii_case(name))
            {
                return Err(pattern_file_refusal(
                    "duplicate",
                    format!("line {line}: pattern '{}' is defined twice", name.trim()),
                ));
            }
            definitions.push(PatternDefinition {
                name: name.to_string(),
                rows: Vec::new(),
            });
        }
        if definitions.is_empty() {
            return Err(pattern_file_refusal(
                "syntax",
                "the pattern library holds no `*NAME` definition".to_string(),
            ));
        }
        for definition in &definitions {
            if definition.rows.is_empty() {
                return Err(pattern_file_refusal(
                    "no_rows",
                    format!(
                        "pattern '{}' has no definition rows; a name-only pattern has no line \
                         family this build can draw",
                        definition.name
                    ),
                ));
            }
        }
        Ok(Self { definitions })
    }

    /// The built-in library (the three transcribed patterns), parsed from
    /// [`BUILTIN_PATTERN_TEXT`] once per process.
    pub(super) fn builtin() -> &'static Self {
        static LIBRARY: std::sync::OnceLock<PatternLibrary> = std::sync::OnceLock::new();
        LIBRARY.get_or_init(|| {
            Self::parse(BUILTIN_PATTERN_TEXT)
                .expect("the built-in `.pat` fixture must parse (pinned by the pattern tests)")
        })
    }

    /// The patterns in library order.
    pub(super) fn definitions(&self) -> &[PatternDefinition] {
        &self.definitions
    }

    /// The library's names, in order (used by the refusal messages).
    fn names(&self) -> String {
        self.definitions()
            .iter()
            .map(|definition| definition.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The definition matching `name` (trimmed, ASCII case-insensitive), or
    /// `None`.
    pub(super) fn find(&self, name: &str) -> Option<&PatternDefinition> {
        let wanted = name.trim();
        self.definitions
            .iter()
            .find(|definition| definition.name.eq_ignore_ascii_case(wanted))
    }

    /// Resolve `name` by name, refusing a name the library does not hold.
    ///
    /// The refusal is `hatch.pattern_missing` and names the requested pattern
    /// plus every name the library does hold: nothing is approximated onto a
    /// neighbouring pattern and no default is substituted.
    pub(super) fn resolve(&self, name: &str) -> CadResult<&PatternDefinition> {
        self.find(name).ok_or_else(|| {
            CadError::InvalidInput(format!(
                "hatch.pattern_missing: pattern '{}' is not in this pattern library ({} \
                 pattern(s): {})",
                name.trim(),
                self.definitions.len(),
                self.names()
            ))
        })
    }

    /// Resolve the session's **1-based** `hatchPattern` index.
    ///
    /// A fractional, zero, negative or past-the-end index is refused by name
    /// (`hatch.pattern_index`) with the implemented set listed; nothing is
    /// truncated or clamped onto a neighbouring pattern. The resolved entry then
    /// goes through [`Self::resolve`], so the positional index and the by-name
    /// entrance share one lookup rule.
    pub(super) fn pattern_at_index(&self, index: f64) -> CadResult<&PatternDefinition> {
        let name = if index.is_finite() && index.fract() == 0.0 && index >= 1.0 {
            self.definitions
                .get(index as usize - 1)
                .map(|definition| definition.name.clone())
        } else {
            None
        };
        let Some(name) = name else {
            return Err(CadError::InvalidInput(format!(
                "HATCH pattern index {index} is not in the implemented predefined set (1..={}: \
                 {})",
                self.definitions.len(),
                self.names()
            )));
        };
        self.resolve(&name)
    }
}

/// A `hatch.pattern_file_<code>: <detail>` parse refusal.
fn pattern_file_refusal(code: &str, detail: String) -> CadError {
    CadError::InvalidInput(format!("hatch.pattern_file_{code}: {detail}"))
}

/// Validate one `.pat` header name.
fn validate_pattern_name(name: &str, line: usize) -> CadResult<()> {
    if name.is_empty() {
        return Err(pattern_file_refusal(
            "name",
            format!("line {line}: a `*NAME` header needs a non-empty pattern name"),
        ));
    }
    if name.chars().count() > PATTERN_LIBRARY_MAX_NAME {
        return Err(pattern_file_refusal(
            "name",
            format!(
                "line {line}: pattern name '{}' exceeds the {PATTERN_LIBRARY_MAX_NAME}-character \
                 limit",
                name.trim()
            ),
        ));
    }
    if !name
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '$'))
    {
        return Err(pattern_file_refusal(
            "name",
            format!(
                "line {line}: pattern name '{}' may only contain ASCII letters, digits and \
                 `-`/`_`/`$`",
                name.trim()
            ),
        ));
    }
    Ok(())
}

/// Parse one `.pat` definition row (`angle, base-x, base-y, delta-x, delta-y
/// [, dash ...]`).
fn parse_pattern_row(text: &str, line: usize, pattern: &str) -> CadResult<PatternRow> {
    let fields: Vec<&str> = text.split(',').map(str::trim).collect();
    if fields.len() < 5 {
        return Err(pattern_file_refusal(
            "syntax",
            format!(
                "line {line} (pattern '{pattern}'): a definition row needs at least 5 fields \
                 (angle, base-x, base-y, delta-x, delta-y), got {}",
                fields.len()
            ),
        ));
    }
    let mut numbers = Vec::with_capacity(fields.len());
    for (position, field) in fields.iter().enumerate() {
        let value: f64 = field.parse().map_err(|_| {
            pattern_file_refusal(
                "syntax",
                format!(
                    "line {line} (pattern '{pattern}'): field {} is not a number: '{field}'",
                    position + 1
                ),
            )
        })?;
        if !value.is_finite() {
            return Err(pattern_file_refusal(
                "syntax",
                format!(
                    "line {line} (pattern '{pattern}'): field {} is not a finite number: '{field}'",
                    position + 1
                ),
            ));
        }
        numbers.push(value);
    }
    let offset = [numbers[3], numbers[4]];
    if offset[0] == 0.0 && offset[1] == 0.0 {
        return Err(pattern_file_refusal(
            "spacing",
            format!(
                "line {line} (pattern '{pattern}'): the delta is (0, 0), so the family has no \
                 spacing between its lines"
            ),
        ));
    }
    let dashes = numbers[5..].to_vec();
    if !dashes.is_empty() && dashes.iter().all(|dash| *dash == 0.0) {
        return Err(pattern_file_refusal(
            "syntax",
            format!(
                "line {line} (pattern '{pattern}'): the dash list is all zeros, which is not a \
                 dash pattern"
            ),
        ));
    }
    Ok(PatternRow {
        angle_deg: numbers[0],
        base: [numbers[1], numbers[2]],
        offset,
        dashes,
    })
}

#[cfg(test)]
mod pattern_library_tests {
    //! `.pat` library contracts (inline: the module is private to `app_drawing`,
    //! so the crate-level test modules cannot reach it).

    use super::*;
    use cad_domain::hatch::predefined_patterns;

    /// Parse a library and return its refusal message.
    fn refusal(text: &str) -> String {
        match PatternLibrary::parse(text) {
            Ok(library) => panic!("expected a refusal, got {library:?}"),
            Err(CadError::InvalidInput(message)) | Err(CadError::Unsupported(message)) => message,
            Err(other) => panic!("expected a named refusal, got {other:?}"),
        }
    }

    #[test]
    fn hatch_builtin_pat_library_matches_the_predefined_table() {
        let library = PatternLibrary::builtin();
        let table = predefined_patterns();
        assert_eq!(library.definitions().len(), table.len());
        for (definition, entry) in library.definitions().iter().zip(table) {
            assert_eq!(definition.name(), entry.name, "name and order");
            // The public table conversion and the parsed `.pat` rows must produce
            // the identical domain pattern (this is the anti-drift pin for the two
            // transcriptions of the same numbers).
            assert_eq!(
                definition.to_hatch_pattern(2.0, 30.0, true),
                entry.to_hatch_pattern(2.0, 30.0, true),
                "pattern {}",
                entry.name
            );
        }
    }

    #[test]
    fn hatch_pat_library_parses_comments_names_rows_and_resolves_by_name() {
        let library = PatternLibrary::parse(
            "; a comment line\r\n\
             *LADDER, rungs\r\n\
             0, 0,0, 0,.5\r\n\
             90, 0,0, 1,0, .25,-.25 ; a trailing comment\r\n\
             \r\n\
             *SOLIDISH, one solid family\r\n\
             45, 1,2, .5,.5\r\n",
        )
        .expect("a well-formed library parses");
        assert_eq!(library.definitions().len(), 2);

        let ladder = library
            .resolve("  ladder ")
            .expect("trimmed, case-insensitive");
        assert_eq!(ladder.name(), "LADDER");
        let pattern = ladder.to_hatch_pattern(2.0, 0.0, false);
        assert_eq!(pattern.name, "LADDER");
        assert_eq!(pattern.lines.len(), 2);
        // Row 1: angle 0, so the offset is stored unchanged.
        assert_eq!(pattern.lines[0].angle, 0.0);
        assert_eq!(pattern.lines[0].base_x, 0.0);
        assert_eq!(pattern.lines[0].offset_y, 0.5);
        assert!(pattern.lines[0].dashes.is_empty());
        // Row 2: angle 90, so the (1, 0) delta rotates onto -X in the plane frame.
        assert!((pattern.lines[1].angle - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
        assert!(pattern.lines[1].offset_x.abs() < 1e-12);
        assert!((pattern.lines[1].offset_y - 1.0).abs() < 1e-12);
        assert_eq!(pattern.lines[1].dashes, vec![0.25, -0.25]);

        // The double flag adds the perpendicular family (the same composition the
        // built-in patterns get).
        let doubled = ladder.to_hatch_pattern(1.0, 0.0, true);
        assert_eq!(doubled.families().len(), 4);
        assert!(library.resolve("solidish").is_ok());
    }

    #[test]
    fn hatch_pat_row_rotation_sets_the_plane_frame_offset() {
        let library =
            PatternLibrary::parse("*ROT, ninety\n90, 0,0, 0,.5\n").expect("one rotated row");
        let pattern = library
            .resolve("ROT")
            .unwrap()
            .to_hatch_pattern(1.0, 0.0, false);
        let families = pattern.families();
        assert_eq!(families.len(), 1);
        let spacing = families[0].offset[0].hypot(families[0].offset[1]);
        assert!((spacing - 0.5).abs() < 1e-12, "spacing {spacing}");
        // The family direction is the row angle plus the hatch angle.
        assert!((families[0].angle - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
    }

    #[test]
    fn hatch_pat_missing_name_and_out_of_range_index_are_refused_by_name() {
        let library = PatternLibrary::parse("*ONLY, one\n0, 0,0, 0,1\n").expect("one pattern");
        let message = match library.resolve("ANSI31") {
            Err(CadError::InvalidInput(message)) => message,
            Err(other) => panic!("expected InvalidInput, got {other:?}"),
            Ok(_) => panic!("a missing name must be refused"),
        };
        assert!(
            message.starts_with("hatch.pattern_missing: pattern 'ANSI31'"),
            "{message}"
        );
        assert!(
            message.contains("ONLY"),
            "the message lists the library: {message}"
        );
        for index in [0.0, 2.0, 1.5, f64::NAN] {
            let message = match library.pattern_at_index(index) {
                Err(CadError::InvalidInput(message)) => message,
                other => panic!("index {index}: expected a refusal, got {other:?}"),
            };
            assert!(
                message.starts_with(&format!("HATCH pattern index {index} is not"))
                    || message.starts_with("HATCH pattern index NaN"),
                "{message}"
            );
        }
        assert_eq!(library.pattern_at_index(1.0).unwrap().name(), "ONLY");
    }

    #[test]
    fn hatch_pat_unsupported_constructs_are_refused_not_skipped() {
        // (library text, expected stable code prefix)
        let cases = [
            ("0, 0,0, 0,1\n", "hatch.pattern_file_syntax: line 1"),
            (
                "*BAD, rows\n0, 0,0, 0\n",
                "hatch.pattern_file_syntax: line 2",
            ),
            (
                "*BAD, rows\n0, 0,0, x,1\n",
                "hatch.pattern_file_syntax: line 2",
            ),
            (
                "*BAD, rows\n0, 0,0, 0,NaN\n",
                "hatch.pattern_file_syntax: line 2",
            ),
            (
                "*BAD, rows\n0, 0,0, 0,0\n",
                "hatch.pattern_file_spacing: line 2",
            ),
            (
                "*BAD, rows\n0, 0,0, 0,1, 0,0\n",
                "hatch.pattern_file_syntax: line 2",
            ),
            (
                "*DUP, one\n0, 0,0, 0,1\n*dup, two\n0, 0,0, 0,1\n",
                "hatch.pattern_file_duplicate: line 3",
            ),
            (
                "*EMPTY, no rows\n",
                "hatch.pattern_file_no_rows: pattern 'EMPTY'",
            ),
            (
                "*, unnamed\n0, 0,0, 0,1\n",
                "hatch.pattern_file_name: line 1",
            ),
            (
                "*BAD NAME, spaces\n0, 0,0, 0,1\n",
                "hatch.pattern_file_name: line 1",
            ),
            (
                "; only a comment\n",
                "hatch.pattern_file_syntax: the pattern library holds no",
            ),
        ];
        for (text, expected) in cases {
            let message = refusal(text);
            assert!(
                message.starts_with(expected),
                "{message} (expected {expected})"
            );
        }
        // The pattern budget is a named refusal too, never a silent truncation.
        let mut many = String::from("*MANY, rows\n");
        for _ in 0..PATTERN_LIBRARY_MAX_ROWS {
            many.push_str("0, 0,0, 0,1\n");
        }
        many.push_str("0, 0,0, 0,1\n");
        assert!(refusal(&many).starts_with("hatch.pattern_file_budget:"));
    }
}
