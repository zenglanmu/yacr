//! Feed the source `MLEADERSTYLE` objects into the `cad-db` table (ADR 0019
//! decision D, P7c).
//!
//! `MLEADERSTYLE` records are DXF **objects** (they live in the
//! `ACAD_MLEADERSTYLE` dictionary, not a typed table), so the importer walks
//! `acad.objects` once and stores every record the model can represent. Before
//! P7c these records were read per entity (`style_object`) and then dropped once
//! the entity's resolved snapshot existed; now the table keeps them so a later
//! `MLEADER` creation on an imported drawing resolves the source style rather
//! than a constant.
//!
//! ## What is stored, and what is a declared limitation
//!
//! Only the [`cad_db::MLeaderStyle`] columns are stored — path type, content
//! kind, text height, arrowhead size, landing gap, dogleg length, the text-style
//! name and the left attachment. The remaining source fields (leader
//! colour/linetype/lineweight, the arrowhead block handle, block-content
//! parameters, draw order and the annotative/unknown flags) have **no reader** in
//! this build: they are not stored rather than given invented homes, and the P7b
//! export still writes their documented defaults from the entity snapshot. That
//! is the named limitation, not a silent loss of a value the model could honour.

use cad_db::MLeaderStyle;
use cad_domain::{MLeaderPathType, MLeaderStyleId};

use super::*;

impl ImporterBuilder<'_> {
    /// Read every source `MLEADERSTYLE` object into the `cad-db` table.
    ///
    /// The id is the source order (the `read_dim_styles` precedent); a source
    /// `Standard` replaces the builder's pinned constant row by name in place, so
    /// the table never carries two `Standard` rows.
    pub(crate) fn read_mleader_styles(&mut self) -> CadResult<()> {
        let styles: Vec<MLeaderStyle> = self
            .acad
            .objects
            .values()
            .filter_map(|object| match object {
                acadrust::objects::ObjectType::MultiLeaderStyle(style) => Some(style),
                _ => None,
            })
            .filter(|style| !style.name.trim().is_empty())
            .map(|style| MLeaderStyle {
                // The real id is assigned below from the source order; the
                // builder replaces it in place when the name is `Standard`.
                id: MLeaderStyleId(0),
                name: style.name.clone(),
                path_type: domain_path_type(style.path_type),
                content_type: style.content_type as i16,
                text_height: style.text_height,
                arrowhead_size: style.arrowhead_size,
                landing_gap: style.landing_gap,
                dogleg_length: style.landing_distance,
                text_style_name: style
                    .text_style_handle
                    .and_then(|handle| self.text_style_name_for_handle(handle))
                    .unwrap_or_else(|| "Standard".to_string()),
                left_attachment: style.text_left_attachment as i16,
            })
            .collect();
        for (index, mut style) in styles.into_iter().enumerate() {
            let id = MLeaderStyleId(index as u128 + 1);
            // A name already present keeps its id (the builder's replace rule);
            // assigning the source order here only gives a fresh row an identity.
            style.id = id;
            self.builder.insert_mleader_style(style)?;
        }
        Ok(())
    }
}

/// The domain mirror of the style object's path type.
fn domain_path_type(path_type: acadrust::objects::MultiLeaderPathType) -> MLeaderPathType {
    match path_type {
        acadrust::objects::MultiLeaderPathType::Invisible => MLeaderPathType::Invisible,
        acadrust::objects::MultiLeaderPathType::StraightLineSegments => {
            MLeaderPathType::StraightLineSegments
        }
        acadrust::objects::MultiLeaderPathType::Spline => MLeaderPathType::Spline,
    }
}
