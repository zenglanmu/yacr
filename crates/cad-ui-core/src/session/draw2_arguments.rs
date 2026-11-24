//! Typed-argument dispatch for the 2026-11-07 draw/modify batch.
//!
//! Pure parsing lives in [`super::commands::draw2`]; this module maps one
//! admitted selector to its [`CommandId`] and dispatches the payload through the
//! same `HostCore::command` funnel every other typed form uses. An argument
//! outside a selector's grammar is refused before IPC with a stable reason.

use cad_app::interaction::InteractionRequest;
use cad_app::{
    CommandId, CommandPayload, ExtendedCommandId, TablePayload, ToolParameter, ToolValue,
};

use super::commands::{
    array_argument_refusal, array_payload, invalid_arraypath_argument_reason,
    invalid_matchcell_argument_reason, invalid_tablestyle_argument_reason, parse_array_argument,
    parse_arraypath_argument, parse_clip_argument, parse_donut_argument, parse_draworder_argument,
    parse_helix_argument, parse_imageadjust_argument, parse_imageattach_argument,
    parse_imageclip_argument, parse_matchcell_argument, parse_mline_argument,
    parse_mlstyle_argument, parse_mpedit_argument, parse_solid_argument,
    parse_table_create_argument, parse_tabledit_argument, parse_tabledit_option_argument,
    parse_tablestyle_argument, parse_trex_argument, parse_xclip_argument, tabledit_format_refusal,
    ArrayArgument, ARRAYCLASSIC_SELECTOR, ARRAYPATH_SELECTOR, ARRAY_SELECTOR, CLIP_SELECTOR,
    DASH_ARRAY_SELECTOR, DASH_IMAGEADJUST_SELECTOR, DASH_TABLE_SELECTOR, DONUT_SELECTOR,
    HELIX_SELECTOR, IMAGEATTACH_SELECTOR, IMAGECLIP_SELECTOR, MATCHCELL_SELECTOR, MLINE_SELECTOR,
    MLSTYLE_SELECTOR, MPEDIT_SELECTOR, SOLID_SELECTOR, TABLEDIT_SELECTOR, TABLESTYLE_SELECTOR,
    TABLE_SELECTOR, TREX_SELECTOR, XCLIP_SELECTOR,
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

/// The `CommandId` one typed `ARRAY` family line dispatches.
///
/// `ARRAY`, `-ARRAY` and `ARRAYCLASSIC` are three spellings of the same router
/// (`array_router`), so the selector the user typed is the selector that runs and
/// the payload is the ordinary `ArrayRect`/`ArrayPolar` one.
fn array_command_id(selector: &str) -> CommandId {
    if selector == ARRAY_SELECTOR {
        CommandId::Extended(ExtendedCommandId::Array)
    } else if selector == DASH_ARRAY_SELECTOR {
        CommandId::Extended(ExtendedCommandId::DashArray)
    } else {
        debug_assert_eq!(selector, ARRAYCLASSIC_SELECTOR);
        CommandId::Extended(ExtendedCommandId::ArrayClassic)
    }
}

/// The stable refusal for an `IMAGEATTACH` / `IMAGECLIP` / `IMAGEADJUST`
/// argument outside the documented grammar.
pub(crate) fn invalid_image_argument_reason(selector: &str, text: &str) -> String {
    // String `const`s are not pattern literals in Rust, so this is an if/else
    // chain rather than a `match` (a `match` arm would bind, not compare).
    let (code, forms) = if selector == IMAGEATTACH_SELECTOR {
        (
            "imageattach.argument",
            "`IMAGEATTACH <file> <x> <y> <width_px> <height_px> <units_per_pixel> \
             [<rotation_deg>]`",
        )
    } else if selector == IMAGECLIP_SELECTOR {
        (
            "imageclip.argument",
            "`IMAGECLIP <minx> <miny> <maxx> <maxy>`, `IMAGECLIP POLY <x1> <y1> <x2> <y2> <x3> \
             <y3> …` or `IMAGECLIP OFF`",
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

/// The stable refusal for an `XCLIP` argument outside the documented grammar.
pub(crate) fn invalid_xclip_argument_reason(text: &str) -> String {
    format!(
        "xclip.argument: unsupported or malformed XCLIP argument in `{}`: the typed forms are \
         `XCLIP RECT <minx> <miny> <maxx> <maxy>`, `XCLIP POLY <x1> <y1> <x2> <y2> <x3> <y3> …`, \
         `XCLIP ON|OFF|DELETE|INVERT` and `XCLIP ?`; nothing was changed",
        text.trim()
    )
}

/// The stable refusal for a `CLIP` argument outside the documented grammar.
pub(crate) fn invalid_clip_argument_reason(text: &str) -> String {
    format!(
        "clip.argument: unsupported or malformed CLIP argument in `{}`: the typed forms are \
         `CLIP RECT <minx> <miny> <maxx> <maxy>`, `CLIP POLY <x1> <y1> <x2> <y2> <x3> <y3> …` \
         and `CLIP ON|OFF|DELETE`; nothing was changed",
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
         grammar), `TABLEDIT <row> <col> [text...]` with a 1-based cell address, \
         `TABLEDIT MERGE <row> <col> <height> <width>`, `TABLEDIT UNMERGE <row> <col>` and \
         `TABLEDIT FORMAT <row> <col> [ALIGN <TL|TC|TR|ML|MC|MR|BL|BC|BR>] [ROTATION <degrees>]`; \
         nothing was changed",
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

    /// Apply one typed `ARRAY RECT …` / `ARRAY POLAR …` line (also the `-ARRAY`
    /// and `ARRAYCLASSIC` spellings, which route to the same planner).
    ///
    /// The numbers are the **session tool-parameter channel** the planners already
    /// read (`arrayRows`/`arrayColumns`/`arrayRowSpacing`/`arrayColumnSpacing` and
    /// `arrayItems`/`arrayFillAngle`), so this writes that one channel and then
    /// dispatches the ordinary `ArrayRect`/`ArrayPolar` payload — no second value
    /// store, and the copies/placement/cap/undo step stay the planner's. Every
    /// grammar refusal (`array.count`, `array.spacing`, `array.angle`,
    /// `array.cap`) happens **before** the channel is touched, and a Viewer session
    /// is refused by the channel's own authorization (nothing dispatched).
    pub(super) fn array_argument(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        let parsed = match parse_array_argument(argument) {
            Ok(parsed) => parsed,
            Err(error) => {
                return self.refuse_argument(array_argument_refusal(selector, error, text))
            }
        };
        let refs = {
            let controller = self.core.controller.borrow();
            controller.session.selection.refs().to_vec()
        };
        if refs.is_empty() {
            return self.refuse_argument(format!(
                "array.selection: {selector} needs at least one selected entity (the source \
                 object); nothing was written"
            ));
        }
        // One retained channel, one authority: the same values the ARRAYRECT /
        // ARRAYPOLAR tools and their option palette use.
        let sets: Vec<InteractionRequest> = match &parsed {
            ArrayArgument::Rect(rect) => [
                (ToolParameter::ArrayRows, rect.rows as f64),
                (ToolParameter::ArrayColumns, rect.columns as f64),
                (ToolParameter::ArrayRowSpacing, rect.row_spacing),
                (ToolParameter::ArrayColumnSpacing, rect.column_spacing),
            ]
            .into_iter()
            .map(|(parameter, value)| InteractionRequest::SetToolParameter {
                parameter,
                value: ToolValue::Number(value),
            })
            .collect(),
            ArrayArgument::Polar(polar) => [
                (ToolParameter::ArrayItems, polar.items as f64),
                (ToolParameter::ArrayFillAngle, polar.fill_angle),
            ]
            .into_iter()
            .map(|(parameter, value)| InteractionRequest::SetToolParameter {
                parameter,
                value: ToolValue::Number(value),
            })
            .collect(),
        };
        // A refused set (Viewer mode, or a value the channel rejects) returns the
        // real refusal and dispatches no command.
        self.submit_interactions(sets)?;
        let payload = array_payload(parsed, refs);
        self.core.record_command(selector);
        match self.core.command(array_command_id(selector), payload) {
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

    /// Apply one typed `XCLIP ...` line.
    ///
    /// The parsed boundary or switch rides the existing command funnel with
    /// `CommandId::Extended(ExtendedCommandId::Xclip)`; the command layer owns
    /// every semantic refusal (rectangle ordering, polygon validity, axis-aligned
    /// world mapping, singular/off-plane transform, exactly one selected
    /// INSERT). A malformed argument is refused before IPC with the stable
    /// reason and changes nothing.
    pub(super) fn xclip_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(payload) = parse_xclip_argument(argument) else {
            return self.refuse_argument(invalid_xclip_argument_reason(text));
        };
        self.core.record_command(XCLIP_SELECTOR);
        match self.core.command(
            CommandId::Extended(ExtendedCommandId::Xclip),
            CommandPayload::XClip(Box::new(payload)),
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

    /// Apply one typed `CLIP ...` line.
    ///
    /// The parsed boundary or state word rides the existing command funnel with
    /// `CommandId::Extended(ExtendedCommandId::Clip)`; the command layer owns
    /// every semantic refusal (selection count and kind, the rectangle ordering,
    /// polygon validity, the world→block-local mapping and the target-specific
    /// `imageclip.*` / `xclip.*` codes). A malformed argument is refused before
    /// IPC with the stable reason and changes nothing.
    pub(super) fn clip_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(payload) = parse_clip_argument(argument) else {
            return self.refuse_argument(invalid_clip_argument_reason(text));
        };
        self.core.record_command(CLIP_SELECTOR);
        match self.core.command(
            CommandId::Extended(ExtendedCommandId::Clip),
            CommandPayload::Clip(Box::new(payload)),
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
                // A recognised `TABLEDIT FORMAT` field this build cannot store is
                // refused by name before any parsing: it must never be treated as
                // a malformed line (which would hide the real reason) nor be
                // accepted and dropped.
                if let Some(reason) = tabledit_format_refusal(argument) {
                    return self.refuse_argument(reason);
                }
                // `TABLEDIT MERGE …` / `TABLEDIT UNMERGE …` / `TABLEDIT FORMAT …`
                // first: their first token is a word, so they can never be
                // mistaken for the 1-based cell address the text form starts with.
                if let Some(payload) = parse_tabledit_option_argument(argument) {
                    (
                        CommandId::Extended(ExtendedCommandId::TableEdit),
                        CommandPayload::Table(Box::new(payload)),
                    )
                } else {
                    let Some(payload) = parse_tabledit_argument(argument) else {
                        return self.refuse_argument(invalid_table_argument_reason(selector, text));
                    };
                    (
                        CommandId::Extended(ExtendedCommandId::TableEdit),
                        CommandPayload::TableEdit(Box::new(payload)),
                    )
                }
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

    /// Apply one typed `MATCHCELL …` line.
    pub(super) fn matchcell_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(payload) = parse_matchcell_argument(argument) else {
            return self.refuse_argument(invalid_matchcell_argument_reason(text));
        };
        self.dispatch_table_payload(ExtendedCommandId::MatchCell, MATCHCELL_SELECTOR, payload)
    }

    /// Apply one typed `TABLESTYLE …` line (the bare selector is the report and
    /// never reaches this parser).
    pub(super) fn tablestyle_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(payload) = parse_tablestyle_argument(argument) else {
            return self.refuse_argument(invalid_tablestyle_argument_reason(text));
        };
        if payload.is_empty() {
            return self.refuse_argument(invalid_tablestyle_argument_reason(text));
        }
        self.dispatch_table_payload(
            ExtendedCommandId::TableStyleList,
            TABLESTYLE_SELECTOR,
            TablePayload::Style(Box::new(payload)),
        )
    }

    /// One shared dispatch for the table payloads (`MATCHCELL`, `TABLESTYLE`).
    fn dispatch_table_payload(
        &self,
        ext: ExtendedCommandId,
        selector: &'static str,
        payload: TablePayload,
    ) -> Result<(), String> {
        self.core.record_command(selector);
        match self.core.command(
            CommandId::Extended(ext),
            CommandPayload::Table(Box::new(payload)),
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
}
