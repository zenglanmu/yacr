//! The DIMSTYLE table (ADR 0017 B.1/B.2).
//!
//! A [`DimStyle`] is the stored style model: the full B.2(a) display-driving
//! variable set plus the B.2(b) "snapshot only" set. The entity-level snapshot
//! ([`cad_domain::DimensionStyleValues`]) carries only B.2(a); the B.2(b) fields
//! exist here so the DIMSTYLE table can be written and resolved without a
//! dangling group-3 reference, and they are declared as not driving the display
//! (ADR 0017 B.2(b), decision B.3).
//!
//! ## Pinned defaults (ADR 0017 N7)
//!
//! The repository previously carried **two contradictory** default sets
//! (acadrust's `DimStyle::dxf_defaults` and `DimStyle::new`) plus a third set in
//! the importer's fallbacks. This table pins **one** source: the Autodesk
//! *Standard* dimension-style initial values, **metric** — the rest of the
//! repository is metric. [`DimStyle::standard_metric`] is that set.
//!
//! Rows the ADR names as known-wrong are reconciled here and why:
//!
//! - `DIMTIH`/`DIMTOH` are **not** "ON/OFF"-by-name and not acadrust's split
//!   (`dxf_defaults` `true/true`, `DimStyle::new` `false/false`): the Autodesk
//!   metric Standard initial values are both **On**, so `text_inside_horizontal`
//!   and `text_outside_horizontal` are `true`.
//! - `DIMPOST`'s Autodesk Standard initial value is the **empty** template (no
//!   prefix/suffix); acadrust's `"<>"` renders the same bare measurement, so we
//!   pin `""` and let the renderer treat `""` and `"<>"` identically.
//! - `DIMTOFL`'s "OFF–ON" notation is ambiguous, so it is pinned to a **single**
//!   value (`true`, the metric Standard).
//! - `DIMCEN`'s metric default is **unverified** (ADR 0017 U-4: the variable page
//!   and command pages disagree), so it is pinned to `0.0` — the neutral value
//!   that does not add a centre mark to an otherwise unchanged display — rather
//!   than inventing an unverified constant. Its consumption is pinned by a
//!   non-default test in `cad-domain`.

use cad_domain::{DimStyleId, DimensionStyleValues, StyleId};

/// One DIMSTYLE table entry: the B.2(a) display-driving variables plus the
/// B.2(b) snapshot-only variables (ADR 0017 B.2).
#[derive(Debug, Clone, PartialEq)]
pub struct DimStyle {
    pub id: DimStyleId,
    pub name: String,

    // ─── B.2(a): display-driving ───
    /// `DIMTXT` (140): measurement text height.
    pub text_height: f64,
    /// `DIMASZ` (41): arrowhead size.
    pub arrow_size: f64,
    /// `DIMEXO` (42): extension-line offset from the origin point.
    pub ext_offset: f64,
    /// `DIMEXE` (44): extension-line extension past the dimension line.
    pub ext_beyond: f64,
    /// `DIMGAP` (147): gap between the dimension line and the measurement text.
    pub text_gap: f64,
    /// `DIMTAD` (77): vertical text position (0..4).
    pub text_vertical: i16,
    /// `DIMTVP` (145): text vertical offset as a fraction of the text height.
    pub text_vertical_position: f64,
    /// `DIMSCALE` (40): overall size/offset scale (not the measurement).
    pub scale: f64,
    /// `DIMDEC` (271): primary-unit decimal places.
    pub decimals: i16,
    /// `DIMZIN` (78): zero-suppression bit field.
    pub zero_suppression: i16,
    /// `DIMLFAC` (144): linear measurement scale factor.
    pub linear_factor: f64,
    /// `DIMRND` (45): distance rounding.
    pub rounding: f64,
    /// `DIMPOST` (3): measurement prefix/suffix template.
    pub postfix: String,
    /// `DIMCEN` (141): centre mark/line size (`>0` mark, `<0` lines, `0` none).
    pub center_mark: f64,
    /// `DIMTXSTY` (340): text-style name.
    pub text_style_name: String,
    /// `DIMSE1` (75): suppress the first extension line.
    pub suppress_ext1: bool,
    /// `DIMSE2` (76): suppress the second extension line.
    pub suppress_ext2: bool,
    /// `DIMSD1` (281): suppress the first dimension line and its arrow.
    pub suppress_dim1: bool,
    /// `DIMSD2` (282): suppress the second dimension line and its arrow.
    pub suppress_dim2: bool,

    // ─── B.2(b): snapshot only, display explicitly ignored ───
    /// `DIMLUNIT` (277): linear unit format (`2` = decimal).
    pub linear_unit_format: i16,
    /// `DIMDSEP` (278): decimal separator character code.
    pub decimal_separator: i16,
    /// `DIMJUST` (280): horizontal text justification.
    pub text_justification: i16,
    /// `DIMTIH` (73): text inside extensions is horizontal.
    pub text_inside_horizontal: bool,
    /// `DIMTOH` (74): text outside extensions is horizontal.
    pub text_outside_horizontal: bool,
    /// `DIMATFIT` (289): text/arrow fit.
    pub text_fit: i16,
    /// `DIMTIX` (174): force text inside the extension lines.
    pub text_inside_extensions: bool,
    /// `DIMTOFL` (172): draw the dimension line inside even when text is outside.
    pub text_outside_flow_line: bool,
    /// `DIMCLRD` (176): dimension-line colour.
    pub dim_line_color: i16,
    /// `DIMCLRE` (177): extension-line colour.
    pub ext_line_color: i16,
    /// `DIMCLRT` (178): text colour.
    pub text_color: i16,
    /// `DIMDLE` (46): dimension-line extension past the arrows.
    pub dim_line_extension: f64,
    /// `DIMDLI` (43): baseline spacing (P5f demoted; stored for fidelity).
    pub dim_line_increment: f64,
    /// `DIMCONTINUEMODE` (drawing-level; not a DIMSTYLE member acadrust models).
    pub continue_mode: bool,
}

impl DimStyle {
    /// The pinned Autodesk *Standard* dimension style, **metric** (ADR 0017 N7).
    ///
    /// This is the single default set: it is what a hand-built database's
    /// `Standard` uses and what the importer falls back to when a referenced
    /// style is absent. See the module docs for the reconciled rows.
    pub fn standard_metric(id: DimStyleId) -> Self {
        DimStyle {
            id,
            name: "Standard".to_string(),
            text_height: 2.5,
            arrow_size: 2.5,
            ext_offset: 0.625,
            ext_beyond: 1.25,
            text_gap: 0.625,
            text_vertical: 1,
            text_vertical_position: 0.0,
            scale: 1.0,
            decimals: 2,
            zero_suppression: 8,
            linear_factor: 1.0,
            rounding: 0.0,
            postfix: String::new(),
            center_mark: 0.0,
            text_style_name: "Standard".to_string(),
            suppress_ext1: false,
            suppress_ext2: false,
            suppress_dim1: false,
            suppress_dim2: false,
            linear_unit_format: 2,
            decimal_separator: 46,
            text_justification: 0,
            text_inside_horizontal: true,
            text_outside_horizontal: true,
            text_fit: 3,
            text_inside_extensions: false,
            text_outside_flow_line: true,
            dim_line_color: 0,
            ext_line_color: 0,
            text_color: 0,
            dim_line_extension: 0.0,
            dim_line_increment: 3.75,
            continue_mode: true,
        }
    }

    /// Resolve this stored style into the entity-level snapshot the synthesis
    /// consumes, applying `DIMSCALE` to the sizes AutoCAD scales (ADR 0017
    /// B.2(a)). `text_style_name`/`text_style`/`text_font` are the resolved
    /// text-style identity, supplied by the caller because resolving a name to
    /// an identity needs the STYLE table.
    pub fn resolved(
        &self,
        text_style_name: String,
        text_style: StyleId,
        text_font: Option<String>,
    ) -> DimensionStyleValues {
        let scale = positive(self.scale, 1.0);
        DimensionStyleValues {
            arrow_size: positive(self.arrow_size, 2.5) * scale,
            ext_offset: positive(self.ext_offset, 0.625) * scale,
            ext_beyond: positive(self.ext_beyond, 1.25) * scale,
            text_height: positive(self.text_height, 2.5) * scale,
            text_gap: positive(self.text_gap, 0.625) * scale,
            center_mark: if self.center_mark.is_finite() {
                self.center_mark * scale
            } else {
                0.0
            },
            // DIMDLI: the baseline row spacing. Scaled by DIMSCALE like the other
            // distance variables (AutoCAD lists DIMDLI among them), and a
            // non-positive/non-finite table value falls back to the documented
            // 3.75 default rather than producing a zero-spacing stack.
            dim_line_increment: positive(self.dim_line_increment, 3.75) * scale,
            text_vertical: self.text_vertical,
            text_vertical_position: if self.text_vertical_position.is_finite() {
                self.text_vertical_position
            } else {
                0.0
            },
            suppress_ext1: self.suppress_ext1,
            suppress_ext2: self.suppress_ext2,
            suppress_dim1: self.suppress_dim1,
            suppress_dim2: self.suppress_dim2,
            linear_factor: positive_abs(self.linear_factor, 1.0),
            decimals: self.decimals.clamp(0, 8) as usize,
            rounding: if self.rounding.is_finite() && self.rounding > 0.0 {
                self.rounding
            } else {
                0.0
            },
            postfix: self.postfix.clone(),
            zero_suppression: self.zero_suppression,
            linear_unit_format: self.linear_unit_format,
            decimal_separator: self.decimal_separator,
            text_style_name,
            text_style,
            text_font,
        }
    }
}

/// A finite, strictly positive value, or `fallback` (for sizes a non-positive
/// value would degenerate).
fn positive(value: f64, fallback: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        fallback
    }
}

/// A finite, non-zero value's magnitude, or `fallback` (`DIMLFAC`; the sign is
/// not modelled, matching the importer's historical behaviour).
fn positive_abs(value: f64, fallback: f64) -> f64 {
    if value.is_finite() && value.abs() > 1e-12 {
        value.abs()
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DrawingDatabaseBuilder;
    use cad_domain::DatabaseId;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn standard_metric_pins_the_metric_defaults() {
        let style = DimStyle::standard_metric(DimStyleId(0));
        assert_eq!(style.name, "Standard");
        assert!(approx(style.text_height, 2.5));
        assert!(approx(style.arrow_size, 2.5));
        assert!(approx(style.ext_offset, 0.625));
        assert!(approx(style.ext_beyond, 1.25));
        assert!(approx(style.text_gap, 0.625));
        assert_eq!(style.text_vertical, 1);
        assert_eq!(style.decimals, 2);
        assert_eq!(style.zero_suppression, 8);
        // Reconciled rows (ADR 0017 N7).
        assert_eq!(style.postfix, "");
        assert!(style.text_inside_horizontal && style.text_outside_horizontal);
        assert!(style.text_outside_flow_line);
        // DIMCEN is unverified (U-4): pinned to the neutral 0.
        assert!(approx(style.center_mark, 0.0));
        assert_eq!(style.linear_unit_format, 2);
        assert_eq!(style.decimal_separator, 46);
    }

    #[test]
    fn resolved_applies_dimscale_to_sizes_but_not_the_measurement() {
        let mut style = DimStyle::standard_metric(DimStyleId(0));
        style.scale = 2.0;
        style.linear_factor = 1.0;
        let values = style.resolved("Standard".to_string(), StyleId(0), None);
        // DIMSCALE scales the sizes and offsets...
        assert!(approx(values.text_height, 5.0));
        assert!(approx(values.arrow_size, 5.0));
        assert!(approx(values.ext_offset, 1.25));
        // ...but not the measurement scale factor.
        assert!(approx(values.linear_factor, 1.0));
        // A signed DIMCEN keeps its sign (mark vs centre line).
        let mut negative = DimStyle::standard_metric(DimStyleId(0));
        negative.center_mark = -2.5;
        assert!(approx(
            negative
                .resolved("Standard".to_string(), StyleId(0), None)
                .center_mark,
            -2.5
        ));
    }

    #[test]
    fn resolved_carries_the_creation_only_dline_increment() {
        // DIMDLI is the DIMBASELINE row spacing (ADR 0017 decision C, P5f): it is
        // resolved like the other distance variables (DIMSCALE-scaled) and its
        // documented default is 3.75.
        let standard = DimStyle::standard_metric(DimStyleId(0));
        assert!(approx(
            standard
                .resolved("Standard".to_string(), StyleId(0), None)
                .dim_line_increment,
            3.75
        ));
        let mut wide = DimStyle::standard_metric(DimStyleId(0));
        wide.dim_line_increment = 7.5;
        assert!(approx(
            wide.resolved("Standard".to_string(), StyleId(0), None)
                .dim_line_increment,
            7.5
        ));
        // DIMSCALE scales it (it is a distance variable), and a non-positive or
        // non-finite table value falls back to the documented default instead of
        // collapsing the stack to zero spacing.
        let mut scaled = DimStyle::standard_metric(DimStyleId(0));
        scaled.scale = 2.0;
        assert!(approx(
            scaled
                .resolved("Standard".to_string(), StyleId(0), None)
                .dim_line_increment,
            7.5
        ));
        for bad in [0.0, -5.0, f64::NAN, f64::INFINITY] {
            let mut broken = DimStyle::standard_metric(DimStyleId(0));
            broken.dim_line_increment = bad;
            assert!(
                approx(
                    broken
                        .resolved("Standard".to_string(), StyleId(0), None)
                        .dim_line_increment,
                    3.75
                ),
                "DIMDLI {bad} must fall back to 3.75"
            );
        }
    }

    #[test]
    fn builder_always_has_a_standard_and_a_source_style_replaces_it_by_name() {
        // A hand-built database still gets the pinned Standard.
        let db = DrawingDatabaseBuilder::new(DatabaseId(1))
            .finish()
            .expect("finished");
        let standard = db.dim_style_by_name("Standard").expect("Standard present");
        assert_eq!(standard.linear_unit_format, 2);

        // A source Standard replaces the pinned one in place (same id, one name
        // entry) rather than leaving a duplicate.
        let mut builder = DrawingDatabaseBuilder::new(DatabaseId(2));
        let mut source = DimStyle::standard_metric(DimStyleId(7));
        source.decimals = 5;
        builder.insert_dim_style(source).expect("insert");
        let db = builder.finish().expect("finished");
        let standard = db.dim_style_by_name("Standard").expect("Standard present");
        assert_eq!(standard.decimals, 5, "the source Standard wins");
        assert_eq!(db.dim_styles().count(), 1, "no duplicate Standard");
    }

    #[test]
    fn a_style_id_owned_by_another_name_is_rejected() {
        // Review N9: reusing an id owned by a different name would overwrite the
        // entry and orphan the old name's index entry, so it is refused.
        let mut builder = DrawingDatabaseBuilder::new(DatabaseId(4));
        let mut first = DimStyle::standard_metric(DimStyleId(1));
        first.name = "First".to_string();
        builder.insert_dim_style(first).expect("first insert");

        let mut second = DimStyle::standard_metric(DimStyleId(1));
        second.name = "Second".to_string();
        assert!(
            builder.insert_dim_style(second).is_err(),
            "an id already owned by another name must be rejected"
        );

        let db = builder.finish().expect("finished");
        assert_eq!(
            db.dim_style_by_name("First")
                .map(|style| style.name.as_str()),
            Some("First"),
            "the original owner keeps its name and entry"
        );
        assert!(
            db.dim_style_by_name("Second").is_none(),
            "the rejected style is not inserted"
        );
    }

    #[test]
    fn resolve_dim_style_resolves_the_text_style_from_the_style_table() {
        let mut builder = DrawingDatabaseBuilder::new(DatabaseId(3));
        builder
            .insert_style(crate::Style {
                id: StyleId(4),
                name: "Note".to_string(),
                resource_keys: vec!["arial.ttf".to_string()],
            })
            .expect("style");
        let mut dim = DimStyle::standard_metric(DimStyleId(1));
        dim.name = "Fancy".to_string();
        dim.text_style_name = "Note".to_string();
        builder.insert_dim_style(dim).expect("dim style");
        let db = builder.finish().expect("finished");
        let values = db.resolve_dim_style("Fancy").expect("resolved");
        assert_eq!(values.text_style, StyleId(4));
        assert_eq!(values.text_font.as_deref(), Some("arial.ttf"));
        assert_eq!(values.text_style_name, "Note");
    }
}
