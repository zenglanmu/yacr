//! Human-readable labels for the typed [`HatchEditOption`] forms.
//!
//! Split out of `hatch_edit.rs` to stay inside the code-size gate. The labels
//! are the command's own option spelling (what the user typed, canonicalised),
//! never a translated UI string: they appear in the `hatchedit.applied` /
//! `hatchedit.unchanged` diagnostics and in the `CadError` detail text.

use super::*;
use crate::{HatchDrawOrder, HatchEditOption};

/// The canonical spelling of one typed option, including its value.
pub(super) fn option_label(option: &HatchEditOption) -> String {
    match option {
        HatchEditOption::Angle(degrees) => format!("ANGLE {degrees}"),
        HatchEditOption::Scale(factor) => format!("SCALE {factor}"),
        HatchEditOption::Pattern(name) => format!("PATTERN {name}"),
        HatchEditOption::PatternType(pattern_type) => {
            format!("PATTERNTYPE {}", pattern_type_label(*pattern_type))
        }
        HatchEditOption::Style(style) => format!("STYLE {}", style_label(*style)),
        HatchEditOption::DrawOrder(order) => format!("DRAWORDER {}", draw_order_label(*order)),
        HatchEditOption::Solid => "SOLID".into(),
        HatchEditOption::Gradient => "GRADIENT".into(),
        HatchEditOption::RecreateBoundary => "RECREATEBOUNDARY".into(),
        HatchEditOption::SeparateHatches => "SEPARATE".into(),
        HatchEditOption::Color(aci) => format!("COLOR {aci}"),
        HatchEditOption::ColorByLayer => "COLOR BYLAYER".into(),
        HatchEditOption::Transparency(percent) => format!("TRANSPARENCY {percent}"),
        HatchEditOption::TransparencyByLayer => "TRANSPARENCY BYLAYER".into(),
        HatchEditOption::Layer(name) => format!("LAYER {name}"),
        HatchEditOption::Origin => "ORIGIN".into(),
        HatchEditOption::Associate => "ASSOCIATE".into(),
        HatchEditOption::Disassociate => "DISASSOCIATE".into(),
        HatchEditOption::Annotative => "ANNOTATIVE".into(),
        HatchEditOption::AddBoundaries => "ADDBOUNDARIES".into(),
        HatchEditOption::RemoveBoundaries => "REMOVEBOUNDARIES".into(),
    }
}

fn pattern_type_label(pattern_type: HatchPatternType) -> &'static str {
    match pattern_type {
        HatchPatternType::UserDefined => "USER",
        HatchPatternType::Predefined => "PREDEFINED",
        HatchPatternType::Custom => "CUSTOM",
    }
}

fn style_label(style: HatchStyle) -> &'static str {
    match style {
        HatchStyle::Normal => "NORMAL",
        HatchStyle::Outer => "OUTER",
        HatchStyle::Ignore => "IGNORE",
    }
}

fn draw_order_label(order: HatchDrawOrder) -> &'static str {
    match order {
        HatchDrawOrder::BehindAll => "BEHIND",
        HatchDrawOrder::InFrontOfAll => "FRONT",
        HatchDrawOrder::BehindBoundary => "BOUNDARY",
    }
}
