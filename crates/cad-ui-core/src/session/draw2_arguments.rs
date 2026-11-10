//! Typed-argument dispatch for the 2026-11-07 draw/modify batch.
//!
//! Pure parsing lives in [`super::commands::draw2`]; this module maps one
//! admitted selector to its [`CommandId`] and dispatches the payload through the
//! same `HostCore::command` funnel every other typed form uses. An argument
//! outside a selector's grammar is refused before IPC with a stable reason.

use cad_app::{CommandId, CommandPayload, ExtendedCommandId};

use super::commands::{
    invalid_arraypath_argument_reason, parse_arraypath_argument, parse_donut_argument,
    parse_draworder_argument, parse_helix_argument, parse_imageadjust_argument,
    parse_imageattach_argument, parse_imageclip_argument, parse_mline_argument,
    parse_mlstyle_argument, parse_mpedit_argument, parse_solid_argument,
    parse_table_create_argument, parse_tabledit_argument, parse_trex_argument, ARRAYPATH_SELECTOR,
    DASH_IMAGEADJUST_SELECTOR, DASH_TABLE_SELECTOR, DONUT_SELECTOR, HELIX_SELECTOR,
    IMAGEATTACH_SELECTOR, IMAGECLIP_SELECTOR, MLINE_SELECTOR, MLSTYLE_SELECTOR, MPEDIT_SELECTOR,
    SOLID_SELECTOR, TABLEDIT_SELECTOR, TABLE_SELECTOR, TREX_SELECTOR,
};
use super::diagnostics::cad_error_text;
use super::CadSession;

/// The stable refusal for an argument outside a selector's grammar.
pub fn invalid_draw2_argument_reason(selector: &str, text: &str) -> String {
    format!(
        "unsupported or malformed {selector} argument in `{}`: the typed form is {selector} \
         <argument> (see the command contract); nothing was changed",
        text.trim()
    )
}

/// The stable refusal for an `IMAGEATTACH` / `IMAGECLIP` / `IMAGEADJUST`
/// argument outside the documented grammar.
pub(crate) fn invalid_image_argument_reason(selector: &str, text: &str) -> String {
    // String `const`s are not pattern literals in Rust, so this is an if/else
    // chain rather than a `match` (a `match` arm would bind, not compare).
    let (code, forms) = if selector == IMAGEATTACH_SELECTOR {
        (
            "imageattach.argument",
            "`IMAGEATTACH <file> <x> <y> <width_px> <height_px> <units_per_pixel>`",
        )
    } else if selector == IMAGECLIP_SELECTOR {
        (
            "imageclip.argument",
            "`IMAGECLIP <minx> <miny> <maxx> <maxy>` or `IMAGECLIP OFF`",
        )
    } else {
        (
            "imageadjust.argument",
            "`IMAGEADJUST <brightness> <contrast> <fade> [MONO|COLOR]` and `IMAGEADJUST RESET`",
        )
    };
    format!(
        "{code}: unsupported or malformed {selector} argument in `{}`: the typed form is \
         {forms}; nothing was changed",
        text.trim()
    )
}

/// The stable refusal for a `TABLE` / `-TABLE` / `TABLEDIT` argument outside the
/// documented grammar.
pub(crate) fn invalid_table_argument_reason(selector: &str, text: &str) -> String {
    let code = match selector {
        TABLE_SELECTOR | DASH_TABLE_SELECTOR => "table.argument",
        _ => "tabledit.argument",
    };
    format!(
        "{code}: unsupported or malformed {selector} argument in `{}`: the typed forms are \
         `TABLE <rows> <cols> <col_width> <row_height> <x> <y>` (`-TABLE` accepts the same \
         grammar) and `TABLEDIT <row> <col> [text...]` with a 1-based cell address; nothing was \
         changed",
        text.trim()
    )
}

impl CadSession {
    pub(super) fn draw2_argument(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        let (id, payload) = match selector {
            DONUT_SELECTOR => (CommandId::Donut, parse_donut_argument(argument)),
            SOLID_SELECTOR => (CommandId::Solid, parse_solid_argument(argument)),
            HELIX_SELECTOR => (CommandId::Helix, parse_helix_argument(argument)),
            MLINE_SELECTOR => (CommandId::Mline, parse_mline_argument(argument)),
            MLSTYLE_SELECTOR => (CommandId::MlStyle, parse_mlstyle_argument(argument)),
            TREX_SELECTOR => (CommandId::Trex, parse_trex_argument(argument)),
            MPEDIT_SELECTOR => (CommandId::Mpedit, parse_mpedit_argument(argument)),
            _ => (CommandId::DrawOrder, parse_draworder_argument(argument)),
        };
        let Some(payload) = payload else {
            return self.refuse_argument(invalid_draw2_argument_reason(selector, text));
        };
        self.core.record_command(selector);
        match self
            .core
            .command(id, CommandPayload::Draw2(Box::new(payload)))
        {
            Ok(()) => {
                *self.error.borrow_mut() = None;
                Ok(())
            }
            Err(error) => {
                let text = cad_error_text(&error);
                *self.error.borrow_mut() = Some(text.clone());
                Err(text)
            }
        }
    }

    /// Apply one typed `ARRAYPATH <count> [ALIGN]` line.
    ///
    /// The count and the align flag ride the payload, together with the
    /// **selection snapshot** taken here: the first selected entity is the path
    /// and the rest are items. Carrying the refs in the payload keeps the
    /// command's input self-contained (the handler never re-reads a mutable
    /// session), exactly like `ARRAYRECT`/`ARRAYPOLAR`. A malformed argument is
    /// refused before IPC with the stable reason and changes nothing; an empty or
    /// single-object selection is refused by the handler with
    /// `arraypath.selection`.
    pub(super) fn arraypath_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some((count, align)) = parse_arraypath_argument(argument) else {
            return self.refuse_argument(invalid_arraypath_argument_reason(text));
        };
        let refs = {
            let controller = self.core.controller.borrow();
            controller.session.selection.refs().to_vec()
        };
        self.core.record_command(ARRAYPATH_SELECTOR);
        match self.core.command(
            CommandId::ArrayPath,
            CommandPayload::ArrayPath { refs, count, align },
        ) {
            Ok(()) => {
                *self.error.borrow_mut() = None;
                Ok(())
            }
            Err(error) => {
                let text = cad_error_text(&error);
                *self.error.borrow_mut() = Some(text.clone());
                Err(text)
            }
        }
    }

    /// Apply one typed `IMAGEATTACH ...` / `IMAGECLIP ...` / `IMAGEADJUST ...`
    /// line.
    ///
    /// The parsed payload rides the existing command funnel with the exact
    /// `CommandId::Extended(ExtendedCommandId::ImageAttach | ImageClip |
    /// ImageAdjust | DashImageAdjust)`; the command layer owns every semantic
    /// refusal (safe resource key, positive pixel size and units-per-pixel,
    /// ordered clip rectangle, 0..=100 transfer percentages, exactly one
    /// selected image). A malformed argument is refused before IPC with the
    /// stable reason and changes nothing.
    pub(super) fn image_argument(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        let (id, payload) = if selector == IMAGEATTACH_SELECTOR {
            let Some(payload) = parse_imageattach_argument(argument) else {
                return self.refuse_argument(invalid_image_argument_reason(selector, text));
            };
            (
                CommandId::Extended(ExtendedCommandId::ImageAttach),
                CommandPayload::ImageAttach(Box::new(payload)),
            )
        } else if selector == IMAGECLIP_SELECTOR {
            let Some(payload) = parse_imageclip_argument(argument) else {
                return self.refuse_argument(invalid_image_argument_reason(selector, text));
            };
            (
                CommandId::Extended(ExtendedCommandId::ImageClip),
                CommandPayload::ImageClip(Box::new(payload)),
            )
        } else {
            let Some(payload) = parse_imageadjust_argument(argument) else {
                return self.refuse_argument(invalid_image_argument_reason(selector, text));
            };
            let id = if selector == DASH_IMAGEADJUST_SELECTOR {
                ExtendedCommandId::DashImageAdjust
            } else {
                ExtendedCommandId::ImageAdjust
            };
            (
                CommandId::Extended(id),
                CommandPayload::ImageAdjust(Box::new(payload)),
            )
        };
        self.core.record_command(selector);
        match self.core.command(id, payload) {
            Ok(()) => {
                *self.error.borrow_mut() = None;
                Ok(())
            }
            Err(error) => {
                let text = cad_error_text(&error);
                *self.error.borrow_mut() = Some(text.clone());
                Err(text)
            }
        }
    }

    /// Apply one typed `TABLE ...`, `-TABLE ...` or `TABLEDIT ...` line.
    ///
    /// The parsed payload rides the existing command funnel with the exact
    /// `CommandId::Extended(ExtendedCommandId::TableCreate | DashTable | TableEdit)`;
    /// the command layer owns every semantic refusal (dimension caps, positive
    /// sizes, finite origin, 1-based cell range, exactly one selected table). A
    /// malformed argument is refused before IPC with the stable reason and
    /// changes nothing.
    pub(super) fn table_argument(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        let (id, payload) = match selector {
            TABLEDIT_SELECTOR => {
                let Some(payload) = parse_tabledit_argument(argument) else {
                    return self.refuse_argument(invalid_table_argument_reason(selector, text));
                };
                (
                    CommandId::Extended(ExtendedCommandId::TableEdit),
                    CommandPayload::TableEdit(Box::new(payload)),
                )
            }
            _ => {
                let Some(payload) = parse_table_create_argument(argument) else {
                    return self.refuse_argument(invalid_table_argument_reason(selector, text));
                };
                let id = if selector == TABLE_SELECTOR {
                    CommandId::Extended(ExtendedCommandId::TableCreate)
                } else {
                    debug_assert_eq!(selector, DASH_TABLE_SELECTOR);
                    CommandId::Extended(ExtendedCommandId::DashTable)
                };
                (id, CommandPayload::TableCreate(Box::new(payload)))
            }
        };
        self.core.record_command(selector);
        match self.core.command(id, payload) {
            Ok(()) => {
                *self.error.borrow_mut() = None;
                Ok(())
            }
            Err(error) => {
                let text = cad_error_text(&error);
                *self.error.borrow_mut() = Some(text.clone());
                Err(text)
            }
        }
    }
}
