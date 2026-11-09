//! Dispatch for the 2026-11 P3 option batch: `OVERKILL`, `REVCLOUD`,
//! `WIPEOUT`, `TEXTMASK` and `TXT2MTXT`.
//!
//! The grammar lives in [`super::commands::payload_options`]; this module turns
//! the parsed value into the typed [`cad_app::CommandPayload`] the application
//! already accepts and dispatches it through the ordinary host funnel. No second
//! IPC and no second write path.
//!
//! The `REVCLOUD` modes AutoCAD captures interactively are **selection-based**
//! here, exactly like the bare Object form: the current selection must be one
//! `LWPOLYLINE`, and its vertices (or axis-aligned bounding box for `RECT`)
//! supply the path. A missing or non-polyline selection is a stable named
//! refusal and changes nothing.
//!
//! `WIPEOUT POLYLINE` likewise takes its target from the one selected ref; the
//! application still validates that it is a closed straight-segment polyline.

use cad_app::{
    AnnotationBatchPayload, CommandId, CommandPayload, RevisionCloudMode, RevisionCloudOptions,
    SelectionToolPayload, TextMaskOptions, Txt2MTextOptions, UtilityPayload, WipeoutPayload,
};
use cad_domain::{Point3, SelectionRef, SemanticGeometry};

use super::commands::{
    invalid_overkill_argument_reason, invalid_revcloud_argument_reason,
    invalid_textmask_argument_reason, invalid_txt2mtext_argument_reason,
    invalid_wipeout_argument_reason, parse_overkill_argument, parse_revcloud_argument,
    parse_textmask_argument, parse_txt2mtext_argument, parse_wipeout_argument, RevCloudArgument,
    TextMaskArgument, Txt2MTextArgument, WipeoutArgument, OVERKILL_SELECTOR, REVCLOUD_SELECTOR,
    TEXTMASK_SELECTOR, TXT2MTXT_SELECTOR, WIPEOUT_SELECTOR,
};
use super::CadSession;

impl CadSession {
    /// Route one typed P3 option line to its payload path.
    pub(super) fn payload_option_argument(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        match selector {
            OVERKILL_SELECTOR => self.overkill_argument(argument, text),
            REVCLOUD_SELECTOR => self.revcloud_argument(argument, text),
            WIPEOUT_SELECTOR => self.wipeout_argument(argument, text),
            TEXTMASK_SELECTOR => self.textmask_argument(argument, text),
            TXT2MTXT_SELECTOR => self.txt2mtext_argument(argument, text),
            _ => self.refuse_argument(format!("unsupported payload-option selector: {selector}")),
        }
    }

    /// `OVERKILL <option>`: the dialog's tolerance/ignore/option set.
    fn overkill_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(settings) = parse_overkill_argument(argument) else {
            return self.refuse_argument(invalid_overkill_argument_reason(text));
        };
        self.dispatch_typed_argument(
            OVERKILL_SELECTOR,
            CommandId::Overkill,
            CommandPayload::SelectionTool(Box::new(SelectionToolPayload::Overkill(Box::new(
                settings,
            )))),
        )
    }

    /// `REVCLOUD <option>`: mode, chord length, style or Modify.
    fn revcloud_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(parsed) = parse_revcloud_argument(argument) else {
            return self.refuse_argument(invalid_revcloud_argument_reason(text));
        };
        let options = match parsed {
            RevCloudArgument::Object { arc_length } => RevisionCloudOptions {
                mode: RevisionCloudMode::Object,
                arc_length,
            },
            RevCloudArgument::Style(style) => RevisionCloudOptions {
                mode: RevisionCloudMode::Style(style),
                arc_length: None,
            },
            RevCloudArgument::Rectangular => {
                let (_, points, _) = self.selected_polyline("revcloud")?;
                let (first, second) = polyline_bounds(&points).ok_or_else(|| {
                    "revcloud.degenerate: the selected polyline has no vertices".to_string()
                })?;
                RevisionCloudOptions {
                    mode: RevisionCloudMode::Rectangular {
                        first,
                        second,
                        reverse: false,
                    },
                    arc_length: None,
                }
            }
            RevCloudArgument::Polygonal => {
                let (_, points, closed) = self.selected_polyline("revcloud")?;
                RevisionCloudOptions {
                    mode: RevisionCloudMode::Polygonal {
                        points,
                        closed,
                        reverse: false,
                    },
                    arc_length: None,
                }
            }
            RevCloudArgument::Freehand => {
                let (_, points, closed) = self.selected_polyline("revcloud")?;
                RevisionCloudOptions {
                    mode: RevisionCloudMode::Freehand {
                        points,
                        closed,
                        reverse: false,
                    },
                    arc_length: None,
                }
            }
            RevCloudArgument::Modify => {
                let (target, points, _) = self.selected_polyline("revcloud")?;
                let erase_side = *points.first().ok_or_else(|| {
                    "revcloud.degenerate: the selected polyline has no vertices".to_string()
                })?;
                RevisionCloudOptions {
                    mode: RevisionCloudMode::Modify {
                        target,
                        add_points: Vec::new(),
                        erase_side,
                    },
                    arc_length: None,
                }
            }
        };
        let payload = CommandPayload::Annotation(Box::new(AnnotationBatchPayload::Utility(
            Box::new(UtilityPayload::RevisionCloud(options)),
        )));
        self.dispatch_typed_argument(REVCLOUD_SELECTOR, CommandId::RevisionCloud, payload)
    }

    /// `WIPEOUT <option>`: the session frame state or the Polyline conversion.
    fn wipeout_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(parsed) = parse_wipeout_argument(argument) else {
            return self.refuse_argument(invalid_wipeout_argument_reason(text));
        };
        let payload = match parsed {
            WipeoutArgument::Frames(frames) => WipeoutPayload::Frames(frames),
            WipeoutArgument::Polyline { erase } => {
                let target = self.selected_ref("wipeout.polyline")?;
                WipeoutPayload::Polyline { target, erase }
            }
        };
        let payload = CommandPayload::Annotation(Box::new(AnnotationBatchPayload::Utility(
            Box::new(UtilityPayload::Wipeout(payload)),
        )));
        self.dispatch_typed_argument(WIPEOUT_SELECTOR, CommandId::Wipeout, payload)
    }

    /// `TEXTMASK <option>`: the offset factor or the mask type.
    fn textmask_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(parsed) = parse_textmask_argument(argument) else {
            return self.refuse_argument(invalid_textmask_argument_reason(text));
        };
        let mut options = TextMaskOptions::default();
        match parsed {
            TextMaskArgument::Offset(offset) => options.offset_factor = offset,
            TextMaskArgument::Mask(kind) => options.kind = kind,
        }
        let payload = CommandPayload::Annotation(Box::new(AnnotationBatchPayload::Utility(
            Box::new(UtilityPayload::TextMask(options)),
        )));
        self.dispatch_typed_argument(TEXTMASK_SELECTOR, CommandId::TextMask, payload)
    }

    /// `TXT2MTXT <option>`: one settings toggle over the documented defaults.
    fn txt2mtext_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(parsed) = parse_txt2mtext_argument(argument) else {
            return self.refuse_argument(invalid_txt2mtext_argument_reason(text));
        };
        let mut options = Txt2MTextOptions::default();
        match parsed {
            Txt2MTextArgument::Combine(combine) => options.combine = combine,
            Txt2MTextArgument::Order(order) => options.order = order,
            Txt2MTextArgument::WordWrap => options.word_wrap = true,
            Txt2MTextArgument::Uniform => options.force_uniform_line_spacing = true,
        }
        let payload = CommandPayload::Annotation(Box::new(AnnotationBatchPayload::Utility(
            Box::new(UtilityPayload::Txt2MText(options)),
        )));
        self.dispatch_typed_argument(TXT2MTXT_SELECTOR, CommandId::Txt2MText, payload)
    }

    /// The one selected ref, or a stable named refusal.
    fn selected_ref(&self, prefix: &str) -> Result<SelectionRef, String> {
        let controller = self.core.controller.borrow();
        match controller.session.selection.refs() {
            [] => Err(format!(
                "{prefix}_selection: exactly one selected object is required, got none"
            )),
            [one] => Ok(one.clone()),
            many => Err(format!(
                "{prefix}_selection: exactly one selected object is required, got {}",
                many.len()
            )),
        }
    }

    /// The one selected `LWPOLYLINE` and its vertex run/closure, or a refusal.
    fn selected_polyline(&self, prefix: &str) -> Result<(SelectionRef, Vec<Point3>, bool), String> {
        let controller = self.core.controller.borrow();
        let reference = match controller.session.selection.refs() {
            [] => {
                return Err(format!(
                    "{prefix}.selection: exactly one selected LWPOLYLINE is required, got none"
                ));
            }
            [one] => one.clone(),
            many => {
                return Err(format!(
                    "{prefix}.selection: exactly one selected LWPOLYLINE is required, got {}",
                    many.len()
                ));
            }
        };
        if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
            return Err(format!(
                "{prefix}.not_addressable: the selection is a block-instance member or a \
                 sub-element; a directly addressable model-space LWPOLYLINE is required"
            ));
        }
        let drawing = controller
            .drawing()
            .ok_or_else(|| format!("{prefix}.selection: no drawing is open"))?;
        let entity = drawing.entity(reference.entity).ok_or_else(|| {
            format!(
                "{prefix}.target_missing: entity {} does not exist",
                reference.entity.0
            )
        })?;
        match &entity.geometry {
            SemanticGeometry::Polyline { points, closed, .. } => {
                Ok((reference, points.clone(), *closed))
            }
            _ => Err(format!(
                "{prefix}.not_polyline: the selected entity is not an LWPOLYLINE"
            )),
        }
    }
}

/// The axis-aligned bounding box of a vertex run, as two opposite corners.
fn polyline_bounds(points: &[Point3]) -> Option<(Point3, Point3)> {
    let first = *points.first()?;
    let (mut min, mut max) = (first, first);
    for point in points {
        min.x = min.x.min(point.x);
        min.y = min.y.min(point.y);
        min.z = min.z.min(point.z);
        max.x = max.x.max(point.x);
        max.y = max.y.max(point.y);
        max.z = max.z.max(point.z);
    }
    Some((min, max))
}
