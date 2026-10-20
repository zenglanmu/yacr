//! The MLEADERSTYLE table (ADR 0019 decision D, P7c).
//!
//! A [`MLeaderStyle`] is the stored style model the `MLEADER` creation command
//! resolves by name, the symmetric counterpart of the `DIMSTYLE` table's
//! `resolve_dim_style`. The table deliberately carries **only** the fields
//! creation reads — text height, arrowhead size, landing gap, dogleg length, the
//! text style reference, the vertical text attachment, the path type and the
//! content type — plus an identity and a name.
//!
//! ## The pinned `Standard` is a constant set, not a real style table
//!
//! [`MLeaderStyle::standard_metric`] is the same **declared debt** `TEXT`'s
//! 2.5 height carries (ADR 0019 decision H): there is no style dialog and no
//! style-edit command, so a drawing built by hand (or one whose source carried no
//! `MLEADERSTYLE`) resolves a constant row. The values are exactly the ones a
//! freshly constructed `acadrust::objects::MultiLeaderStyle` — the record the
//! P7b-2 writer starts from — carries, so a created leader and the style record
//! written for it agree field for field:
//!
//! | field | constant | DXF code |
//! |---|---|---|
//! | dogleg (landing) length | `0.36` | style `43` |
//! | landing gap | `0.09` | style `42` |
//! | arrowhead size | `0.18` | style `44` |
//! | text height | `0.18` | style `45` |
//! | text style | `Standard` | style `342` |
//! | text left attachment | `1` (middle of top line) | style `174` |
//! | path type | straight-line segments | style `173` |
//! | content type | `2` (MText) | style `170` |
//!
//! These are **not** a verified metric AutoCAD *Standard* MLEADERSTYLE (unlike
//! the DIMSTYLE table's reconciled N7 rows); nobody has checked them against a
//! vendor file, and the numbers are imperial-flavoured. They are the writer's
//! base values, and that is the whole claim.
//!
//! ## What the import does and does not feed
//!
//! The importer reads the source `MLEADERSTYLE` objects into this table
//! (`cad-import-acadrust`'s `read_mleader_styles`), so the table is authoritative
//! for creation even for an imported drawing. Fields the entity model does not
//! carry — leader colour/linetype/lineweight, the arrowhead block handle, block
//! content parameters, draw order, annotative flags and the R2013+ flag — are
//! **not** stored here: nothing reads them, and inventing homes for them would
//! create a second contradictory source. That is a declared limitation, not a
//! silent drop: the writer still emits them from the entity snapshot's
//! documented defaults (ADR 0019 decision E).
//!
//! Per-line override data is not part of the style at all: `MLeaderSTYLE` has no
//! per-line overrides, and the created entity writes `override_mask = 0`.

use cad_domain::{MLeaderPathType, MLeaderStyleId, MLeaderStyleValues, StyleId};

/// One MLEADERSTYLE table entry: the fields the `MLEADER` creation path reads.
#[derive(Debug, Clone, PartialEq)]
pub struct MLeaderStyle {
    pub id: MLeaderStyleId,
    pub name: String,
    /// DXF style `173`: the leader path type.
    pub path_type: MLeaderPathType,
    /// DXF style `170`: the content kind ([`Self::CONTENT_MTEXT`] and friends).
    /// Creation accepts only [`Self::CONTENT_MTEXT`]; a content-less row is
    /// refused as such and block/tolerance rows are refused by name.
    pub content_type: i16,
    /// DXF style `45`: the annotation text height.
    pub text_height: f64,
    /// DXF style `44`: the arrowhead size.
    pub arrowhead_size: f64,
    /// DXF style `42`: the gap between the dogleg end and the content.
    pub landing_gap: f64,
    /// DXF style `43`: the dogleg (landing) length.
    pub dogleg_length: f64,
    /// DXF style `342`: the text-style name (`Standard` when the source handle
    /// does not resolve).
    pub text_style_name: String,
    /// DXF style `174`: the vertical text attachment (`0`..=`10`).
    pub left_attachment: i16,
}

/// The resolved creation snapshot: the effective values a created entity stores
/// plus the vertical attachment that lives on the text, not on the style values.
///
/// This is the **single** resolution result — the command builds the entity from
/// it and nothing re-resolves the table afterwards, so the entity snapshot and
/// the created leader cannot disagree.
#[derive(Debug, Clone, PartialEq)]
pub struct MLeaderStyleResolution {
    /// The effective style values the entity carries (P7b-1's resolved snapshot).
    pub values: MLeaderStyleValues,
    /// The vertical text attachment (DXF `174`), normalised to `0`..=`10`.
    pub left_attachment: i16,
}

impl MLeaderStyle {
    /// The pinned `Standard` row ([`Self::standard_metric`]) with no id, for a
    /// caller that only needs the constants.
    pub const STANDARD_NAME: &'static str = "Standard";
    /// DXF style `170 = 0`: leaders without annotation content (not creatable).
    pub const CONTENT_NONE: i16 = 0;
    /// DXF style `170 = 1`: block content (not creatable here; block content
    /// stays on the import/display/export path).
    pub const CONTENT_BLOCK: i16 = 1;
    /// DXF style `170 = 2`: MText content — the only creatable kind.
    pub const CONTENT_MTEXT: i16 = 2;
    /// DXF style `170 = 3`: tolerance content (not creatable here).
    pub const CONTENT_TOLERANCE: i16 = 3;

    /// The pinned default MLEADERSTYLE row (see the module docs).
    ///
    /// This is a **constant set**, not a real style table: there is no style
    /// dialog, and the values are the P7b writer's `MultiLeaderStyle` base values
    /// rather than a verified metric AutoCAD default.
    pub fn standard_metric(id: MLeaderStyleId) -> Self {
        MLeaderStyle {
            id,
            name: Self::STANDARD_NAME.to_string(),
            path_type: MLeaderPathType::StraightLineSegments,
            content_type: Self::CONTENT_MTEXT,
            text_height: 0.18,
            arrowhead_size: 0.18,
            landing_gap: 0.09,
            dogleg_length: 0.36,
            text_style_name: "Standard".to_string(),
            left_attachment: 1,
        }
    }

    /// Resolve this stored style into the creation snapshot the entity stores.
    ///
    /// `text_style_name`/`text_style`/`text_font` are the **resolved** text-style
    /// identity, supplied by the caller because resolving a name to an identity
    /// needs the STYLE table (the `DimStyle::resolved` precedent). Sizes are
    /// normalised to finite positive values with the pinned constants as the
    /// fallback, so a corrupt table row can never poison the created geometry.
    ///
    /// The two values the table has no column for are the creation-path
    /// constants, declared here rather than read from a second source:
    /// landing/dogleg are always enabled (the created leader draws its dogleg)
    /// and the scale factor is `1.0` (creation has no annotative scale step).
    pub fn resolved(
        &self,
        text_style_name: String,
        text_style: StyleId,
        text_font: Option<String>,
    ) -> MLeaderStyleResolution {
        let defaults = MLeaderStyle::standard_metric(self.id);
        MLeaderStyleResolution {
            values: MLeaderStyleValues {
                path_type: self.path_type,
                enable_landing: true,
                enable_dogleg: true,
                landing_distance: positive(self.dogleg_length, defaults.dogleg_length),
                landing_gap: non_negative(self.landing_gap, defaults.landing_gap),
                arrowhead_size: positive(self.arrowhead_size, defaults.arrowhead_size),
                text_height: positive(self.text_height, defaults.text_height),
                text_style,
                text_style_name,
                text_font,
                scale_factor: 1.0,
            },
            left_attachment: if (0..=10).contains(&self.left_attachment) {
                self.left_attachment
            } else {
                defaults.left_attachment
            },
        }
    }
}

/// A finite, strictly positive value, or `fallback`.
fn positive(value: f64, fallback: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        fallback
    }
}

/// A finite, non-negative value, or `fallback`.
fn non_negative(value: f64, fallback: f64) -> f64 {
    if value.is_finite() && value >= 0.0 {
        value
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
        (a - b).abs() < 1e-12
    }

    #[test]
    fn standard_metric_pins_the_writer_base_constants() {
        let style = MLeaderStyle::standard_metric(MLeaderStyleId(0));
        assert_eq!(style.name, "Standard");
        assert_eq!(style.path_type, MLeaderPathType::StraightLineSegments);
        assert_eq!(style.content_type, MLeaderStyle::CONTENT_MTEXT);
        assert!(approx(style.text_height, 0.18));
        assert!(approx(style.arrowhead_size, 0.18));
        assert!(approx(style.landing_gap, 0.09));
        assert!(approx(style.dogleg_length, 0.36));
        assert_eq!(style.text_style_name, "Standard");
        assert_eq!(style.left_attachment, 1);
    }

    #[test]
    fn a_hand_built_database_always_has_a_standard_that_resolves() {
        let db = DrawingDatabaseBuilder::new(DatabaseId(1))
            .finish()
            .expect("finished");
        let standard = db
            .mleader_style_by_name(MLeaderStyle::STANDARD_NAME)
            .expect("Standard is pinned");
        assert!(approx(standard.dogleg_length, 0.36));
        // Case-insensitive fallback, like the other name lookups.
        assert!(db.mleader_style_by_name("  standard ").is_some());
        let resolved = db
            .resolve_mleader_style(MLeaderStyle::STANDARD_NAME)
            .expect("the default resolves");
        assert!(approx(resolved.values.text_height, 0.18));
        assert!(approx(resolved.values.landing_distance, 0.36));
        assert!(approx(resolved.values.landing_gap, 0.09));
        assert!(approx(resolved.values.arrowhead_size, 0.18));
        assert!(approx(resolved.values.scale_factor, 1.0));
        assert!(resolved.values.enable_landing && resolved.values.enable_dogleg);
        assert_eq!(resolved.left_attachment, 1);
        assert_eq!(resolved.values.text_style, StyleId(0));
        assert!(resolved.values.text_font.is_none());
        assert_eq!(resolved.values.text_style_name, "Standard");
    }

    #[test]
    fn an_unknown_name_resolves_to_nothing_instead_of_inventing_a_style() {
        let db = DrawingDatabaseBuilder::new(DatabaseId(2))
            .finish()
            .expect("finished");
        assert!(db.mleader_style_by_name("Missing").is_none());
        assert!(db.resolve_mleader_style("Missing").is_none());
    }

    #[test]
    fn the_text_style_identity_and_font_resolve_from_the_style_table() {
        let mut builder = DrawingDatabaseBuilder::new(DatabaseId(3));
        builder
            .insert_style(crate::Style {
                id: StyleId(4),
                name: "Note".to_string(),
                resource_keys: vec!["arial.ttf".to_string()],
            })
            .expect("style");
        let mut style = MLeaderStyle::standard_metric(MLeaderStyleId(1));
        style.name = "Fancy".to_string();
        style.text_style_name = "Note".to_string();
        style.text_height = 3.0;
        builder.insert_mleader_style(style).expect("style");
        let db = builder.finish().expect("finished");
        let resolved = db.resolve_mleader_style("Fancy").expect("resolved");
        assert_eq!(resolved.values.text_style, StyleId(4));
        assert_eq!(resolved.values.text_font.as_deref(), Some("arial.ttf"));
        assert_eq!(resolved.values.text_style_name, "Note");
        assert!(approx(resolved.values.text_height, 3.0));
    }

    #[test]
    fn a_source_standard_replaces_the_pinned_row_by_name() {
        let mut builder = DrawingDatabaseBuilder::new(DatabaseId(4));
        let mut source = MLeaderStyle::standard_metric(MLeaderStyleId(9));
        source.dogleg_length = 5.0;
        builder.insert_mleader_style(source).expect("insert");
        let db = builder.finish().expect("finished");
        let standard = db.mleader_style_by_name("Standard").expect("Standard");
        assert!(approx(standard.dogleg_length, 5.0), "the source wins");
        assert_eq!(db.mleader_styles().count(), 1, "no duplicate Standard");
    }

    #[test]
    fn a_style_id_owned_by_another_name_is_rejected() {
        let mut builder = DrawingDatabaseBuilder::new(DatabaseId(5));
        let mut first = MLeaderStyle::standard_metric(MLeaderStyleId(1));
        first.name = "First".to_string();
        builder.insert_mleader_style(first).expect("first insert");
        let mut second = MLeaderStyle::standard_metric(MLeaderStyleId(1));
        second.name = "Second".to_string();
        assert!(
            builder.insert_mleader_style(second).is_err(),
            "an id already owned by another name must be rejected"
        );
        let db = builder.finish().expect("finished");
        assert!(db.mleader_style_by_name("First").is_some());
        assert!(db.mleader_style_by_name("Second").is_none());
    }

    #[test]
    fn resolved_normalises_corrupt_numbers_to_the_pinned_constants() {
        let mut style = MLeaderStyle::standard_metric(MLeaderStyleId(0));
        style.text_height = f64::NAN;
        style.arrowhead_size = 0.0;
        style.landing_gap = -1.0;
        style.dogleg_length = f64::INFINITY;
        style.left_attachment = 99;
        let resolved = style.resolved("Standard".to_string(), StyleId(0), None);
        assert!(approx(resolved.values.text_height, 0.18));
        assert!(approx(resolved.values.arrowhead_size, 0.18));
        assert!(approx(resolved.values.landing_gap, 0.09));
        assert!(approx(resolved.values.landing_distance, 0.36));
        assert_eq!(resolved.left_attachment, 1);
    }
}
